//! Checked application-byte and worker admission; the runner owns execution.
use crate::{
    config::{Consistency, MigrationMode, MigrationOptions},
    convert,
    model::RowPosition,
};
use std::{mem::size_of, sync::Arc};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ResourceError {
    #[error("{0}")]
    InvalidLimits(&'static str),
    #[error("resource-limit arithmetic overflow")]
    Overflow,
    #[error("individual byte reservations must fit u32")]
    PermitTooLarge,
    #[error("memory_bytes {available} is below one pipeline's reservation {minimum}")]
    InsufficientMemory { minimum: usize, available: usize },
    #[error("artifact memory reservation was already taken or is unavailable")]
    ArtifactReservationUnavailable,
    #[error("single_snapshot requires one table worker and one reader per table")]
    SnapshotParallelism,
    #[error("range readers require frozen source consistency")]
    RangeRequiresFrozen,
    #[error("range readers require a positive max_key_span")]
    RangeSpanRequired,
    #[error("resource acquisition canceled")]
    Cancelled,
    #[error("resource admission closed")]
    Closed,
    #[error("no work was admitted for this resource")]
    NoCapacity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryLayout {
    pub batch_reservation: usize,
    pub workspace_reservation: usize,
    pub pipeline_reservation: usize,
}
impl MemoryLayout {
    /// Includes Q queued batches, one active batch, one filling batch and W.
    /// Metadata, allocator/driver/TLS overhead remain outside this byte budget.
    pub fn compute(options: &MigrationOptions) -> Result<Self, ResourceError> {
        if options.batch_rows == 0
            || options.batch_bytes == 0
            || options.max_row_bytes == 0
            || options.queue_batches == 0
            || options.max_row_bytes > options.batch_bytes
        {
            return Err(ResourceError::InvalidLimits(
                "batch/row/queue limits must be positive and max_row_bytes <= batch_bytes",
            ));
        }
        let batch_reservation = options
            .batch_rows
            .checked_mul(size_of::<RowPosition>())
            .and_then(|offsets| options.batch_bytes.checked_add(offsets))
            .ok_or(ResourceError::Overflow)?;
        let encoder = convert::encoder_workspace_bytes(options.max_row_bytes)
            .map_err(|_| ResourceError::Overflow)?;
        let workspace_reservation = options
            .max_row_bytes
            .checked_mul(3)
            .and_then(|bytes| bytes.checked_add(encoder))
            .and_then(|bytes| bytes.checked_add(65_536))
            .ok_or(ResourceError::Overflow)?;
        if u32::try_from(batch_reservation).is_err()
            || u32::try_from(workspace_reservation).is_err()
        {
            return Err(ResourceError::PermitTooLarge);
        }
        let pipeline_reservation = options
            .queue_batches
            .checked_add(2)
            .and_then(|batches| batches.checked_mul(batch_reservation))
            .and_then(|bytes| bytes.checked_add(workspace_reservation))
            .ok_or(ResourceError::Overflow)?;
        Ok(Self {
            batch_reservation,
            workspace_reservation,
            pipeline_reservation,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionBounds {
    pub normal_source: usize,
    pub normal_target: usize,
    pub emergency_target: usize,
    pub peak_target: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Admission {
    pub layout: MemoryLayout,
    pub table_workers: usize,
    pub reader_workers: usize,
    pub index_workers: usize,
    pub queue_batches: usize,
    pub memory_bytes: usize,
    pub artifact_reservation_bytes: usize,
    pub connections: ConnectionBounds,
}
impl Admission {
    pub fn new(
        options: &MigrationOptions,
        consistency: Consistency,
        selected_tables: usize,
        index_jobs: usize,
    ) -> Result<Self, ResourceError> {
        Self::new_with_artifact_reservation(options, consistency, selected_tables, index_jobs, 0)
    }

    pub fn new_with_artifact_reservation(
        options: &MigrationOptions,
        consistency: Consistency,
        selected_tables: usize,
        index_jobs: usize,
        artifact_reservation_bytes: usize,
    ) -> Result<Self, ResourceError> {
        let layout = MemoryLayout::compute(options)?;
        if options.table_workers == 0
            || options.index_workers == 0
            || options.readers_per_table == 0
            || options.memory_bytes == 0
            || options.memory_bytes > Semaphore::MAX_PERMITS
        {
            return Err(ResourceError::InvalidLimits(
                "worker/memory limits must be positive and fit semaphore capacity",
            ));
        }
        if consistency == Consistency::SingleSnapshot
            && (options.table_workers != 1 || options.readers_per_table != 1)
        {
            return Err(ResourceError::SnapshotParallelism);
        }
        if options.readers_per_table > 1 {
            if consistency != Consistency::Frozen {
                return Err(ResourceError::RangeRequiresFrozen);
            }
            if options.max_key_span.is_none_or(|span| span == 0) {
                return Err(ResourceError::RangeSpanRequired);
            }
        }
        let selected_tables = if options.mode == MigrationMode::SchemaOnly {
            0
        } else {
            selected_tables
        };
        if artifact_reservation_bytes > options.memory_bytes {
            return Err(ResourceError::InsufficientMemory {
                minimum: artifact_reservation_bytes,
                available: options.memory_bytes,
            });
        }
        let table_bytes = options.memory_bytes - artifact_reservation_bytes;
        let requested_tables = options.table_workers.min(selected_tables);
        let pipeline_capacity = table_bytes / layout.pipeline_reservation;
        let requested_readers = requested_tables
            .checked_mul(options.readers_per_table)
            .ok_or(ResourceError::Overflow)?;
        let admitted_readers = requested_readers.min(pipeline_capacity);
        // Every admitted table can reserve its first lane and still acquire
        // the remaining configured lanes; partial fan-out would deadlock when
        // each table holds one permit while waiting for another.
        let table_workers = requested_tables.min(admitted_readers / options.readers_per_table);
        let reader_workers = table_workers
            .checked_mul(options.readers_per_table)
            .ok_or(ResourceError::Overflow)?;
        if selected_tables > 0 && reader_workers == 0 {
            return Err(ResourceError::InsufficientMemory {
                minimum: artifact_reservation_bytes
                    .checked_add(layout.pipeline_reservation)
                    .ok_or(ResourceError::Overflow)?,
                available: options.memory_bytes,
            });
        }
        let index_workers = options.index_workers.min(index_jobs);
        if index_workers > Semaphore::MAX_PERMITS {
            return Err(ResourceError::InvalidLimits(
                "index worker admission exceeds semaphore capacity",
            ));
        }
        let target_workers = reader_workers.max(index_workers);
        let normal_target = target_workers
            .checked_add(1)
            .ok_or(ResourceError::Overflow)?;
        let normal_source = match consistency {
            // Preflight must be cancellable even before it knows the selected
            // table count, so its reader and separate control session coexist.
            Consistency::SingleSnapshot => 2,
            Consistency::Frozen => reader_workers
                .checked_add(1)
                .ok_or(ResourceError::Overflow)?,
        };
        // CancelToken opens a separate socket even when normal admission is full.
        let peak_target = normal_target
            .checked_mul(2)
            .ok_or(ResourceError::Overflow)?;
        Ok(Self {
            layout,
            table_workers,
            reader_workers,
            index_workers,
            queue_batches: options.queue_batches,
            memory_bytes: options.memory_bytes,
            artifact_reservation_bytes,
            connections: ConnectionBounds {
                normal_source,
                normal_target,
                emergency_target: normal_target,
                peak_target,
            },
        })
    }
}

#[derive(Debug)]
pub struct Resources {
    admission: Admission,
    tables: Arc<Semaphore>,
    readers: Arc<Semaphore>,
    indexes: Arc<Semaphore>,
    targets: Arc<Semaphore>,
    bytes: Arc<Semaphore>,
    artifact_memory: std::sync::Mutex<Option<OwnedSemaphorePermit>>,
}
/// Hold until both connections, stream, queue and all retained batches are gone.
#[derive(Debug)]
#[must_use]
pub struct TablePermit {
    _table: OwnedSemaphorePermit,
    reader: ReaderPermit,
}
impl TablePermit {
    /// Share this table's already-admitted workspace with accepted artifact
    /// retention so producer cancellation cannot release its byte charge.
    pub(crate) fn workspace(&self) -> Arc<OwnedSemaphorePermit> {
        self.reader.workspace()
    }
}
/// Hold until one reader's connections, stream, queue and retained batches end.
#[derive(Debug)]
#[must_use]
pub struct ReaderPermit {
    _reader: OwnedSemaphorePermit,
    _target: OwnedSemaphorePermit,
    _workspace: Arc<OwnedSemaphorePermit>,
}
impl ReaderPermit {
    pub(crate) fn workspace(&self) -> Arc<OwnedSemaphorePermit> {
        self._workspace.clone()
    }
}
/// Hold until the index operation and its target connection finish teardown.
#[derive(Debug)]
#[must_use]
pub struct IndexPermit {
    _index: OwnedSemaphorePermit,
    _target: OwnedSemaphorePermit,
}
impl Resources {
    pub fn new(admission: Admission) -> Self {
        let bytes = Arc::new(Semaphore::new(admission.memory_bytes));
        let artifact_memory = if admission.artifact_reservation_bytes == 0 {
            None
        } else {
            Some(
                bytes
                    .clone()
                    .try_acquire_many_owned(admission.artifact_reservation_bytes as u32)
                    .expect("admission validates artifact memory against the global budget"),
            )
        };
        Self {
            admission,
            tables: Arc::new(Semaphore::new(admission.table_workers)),
            readers: Arc::new(Semaphore::new(admission.reader_workers)),
            indexes: Arc::new(Semaphore::new(admission.index_workers)),
            targets: Arc::new(Semaphore::new(
                admission.reader_workers.max(admission.index_workers),
            )),
            bytes,
            artifact_memory: std::sync::Mutex::new(artifact_memory),
        }
    }
    pub fn admission(&self) -> &Admission {
        &self.admission
    }
    pub fn available_bytes(&self) -> usize {
        self.bytes.available_permits()
    }
    /// Transfer the run-lifetime reservation to the durable artifact writer.
    /// Resources removes it before any table or batch permit can be acquired.
    pub fn take_artifact_reservation(&self) -> Result<OwnedSemaphorePermit, ResourceError> {
        self.artifact_memory
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .ok_or(ResourceError::ArtifactReservationUnavailable)
    }
    /// Prevent further admission without revoking already-owned batch permits.
    pub fn close(&self) {
        self.tables.close();
        self.readers.close();
        self.indexes.close();
        self.targets.close();
        self.bytes.close();
    }
    pub async fn table(
        &self,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<TablePermit, ResourceError> {
        if *cancel.borrow() {
            return Err(ResourceError::Cancelled);
        }
        if self.admission.table_workers == 0 {
            return Err(ResourceError::NoCapacity);
        }
        let table = acquire(&self.tables, 1, cancel).await?;
        let reader = self.reader(cancel).await?;
        Ok(TablePermit {
            _table: table,
            reader,
        })
    }
    pub async fn reader(
        &self,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<ReaderPermit, ResourceError> {
        if *cancel.borrow() {
            return Err(ResourceError::Cancelled);
        }
        if self.admission.reader_workers == 0 {
            return Err(ResourceError::NoCapacity);
        }
        let reader = acquire(&self.readers, 1, cancel).await?;
        let target = acquire(&self.targets, 1, cancel).await?;
        let workspace = acquire(
            &self.bytes,
            self.admission.layout.workspace_reservation as u32,
            cancel,
        )
        .await?;
        Ok(ReaderPermit {
            _reader: reader,
            _target: target,
            _workspace: Arc::new(workspace),
        })
    }
    pub async fn index(
        &self,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<IndexPermit, ResourceError> {
        if *cancel.borrow() {
            return Err(ResourceError::Cancelled);
        }
        if self.admission.index_workers == 0 {
            return Err(ResourceError::NoCapacity);
        }
        let index = acquire(&self.indexes, 1, cancel).await?;
        let target = acquire(&self.targets, 1, cancel).await?;
        Ok(IndexPermit {
            _index: index,
            _target: target,
        })
    }
    /// Root bounds batches per active table to queue_batches + 2.
    pub async fn batch(
        &self,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<OwnedSemaphorePermit, ResourceError> {
        acquire(
            &self.bytes,
            self.admission.layout.batch_reservation as u32,
            cancel,
        )
        .await
    }
}

async fn cancelled(cancel: &mut watch::Receiver<bool>) {
    loop {
        if *cancel.borrow() {
            return;
        }
        if cancel.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}
async fn acquire(
    semaphore: &Arc<Semaphore>,
    permits: u32,
    cancel: &mut watch::Receiver<bool>,
) -> Result<OwnedSemaphorePermit, ResourceError> {
    tokio::select! {
        biased;
        _ = cancelled(cancel) => Err(ResourceError::Cancelled),
        result = semaphore.clone().acquire_many_owned(permits) => result.map_err(|_| ResourceError::Closed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BatchStorage, EncodedBatch, RowLocator};
    use bytes::Bytes;
    use std::time::Duration;
    use tokio::sync::mpsc;

    fn options() -> MigrationOptions {
        MigrationOptions {
            batch_rows: 2,
            batch_bytes: 1024,
            max_row_bytes: 64,
            queue_batches: 2,
            table_workers: 3,
            index_workers: 2,
            ..Default::default()
        }
    }
    #[test]
    fn exact_minimum_and_checked_limits_include_every_retained_batch() {
        let mut options = options();
        let layout = MemoryLayout::compute(&options).unwrap();
        assert_eq!(
            layout.batch_reservation,
            1024 + 2 * size_of::<RowPosition>()
        );
        assert_eq!(layout.workspace_reservation, 65_992);
        assert_eq!(
            layout.pipeline_reservation,
            65_992 + 4 * layout.batch_reservation
        );
        options.memory_bytes = layout.pipeline_reservation - 1;
        assert!(matches!(
            Admission::new(&options, Consistency::Frozen, 3, 5),
            Err(ResourceError::InsufficientMemory { .. })
        ));
        options.memory_bytes += 1;
        let admitted = Admission::new(&options, Consistency::Frozen, 3, 5).unwrap();
        assert_eq!((admitted.table_workers, admitted.index_workers), (1, 2));
        assert_eq!(
            admitted.connections,
            ConnectionBounds {
                normal_source: 2,
                normal_target: 3,
                emergency_target: 3,
                peak_target: 6,
            }
        );
        options.memory_bytes *= 2;
        assert_eq!(
            Admission::new(&options, Consistency::Frozen, 3, 1)
                .unwrap()
                .table_workers,
            2
        );
        options.batch_rows = usize::MAX;
        assert_eq!(
            MemoryLayout::compute(&options),
            Err(ResourceError::Overflow)
        );
        options = self::options();
        options.queue_batches = usize::MAX;
        assert_eq!(
            MemoryLayout::compute(&options),
            Err(ResourceError::Overflow)
        );
        if usize::BITS > 32 {
            options = self::options();
            options.batch_bytes = u32::MAX as usize + 1;
            assert_eq!(
                MemoryLayout::compute(&options),
                Err(ResourceError::PermitTooLarge)
            );
        }
    }
    #[test]
    fn artifact_memory_is_reserved_before_table_admission_and_limits_workers() {
        let mut options = options();
        let pipeline = MemoryLayout::compute(&options)
            .unwrap()
            .pipeline_reservation;
        let artifact = 4096;
        options.memory_bytes = artifact + pipeline - 1;
        assert_eq!(
            Admission::new_with_artifact_reservation(&options, Consistency::Frozen, 3, 0, artifact,),
            Err(ResourceError::InsufficientMemory {
                minimum: artifact + pipeline,
                available: artifact + pipeline - 1,
            })
        );

        options.memory_bytes = artifact + 2 * pipeline - 1;
        let one =
            Admission::new_with_artifact_reservation(&options, Consistency::Frozen, 3, 0, artifact)
                .unwrap();
        assert_eq!(one.table_workers, 1);
        assert_eq!(one.artifact_reservation_bytes, artifact);

        options.memory_bytes = artifact + 2 * pipeline;
        let two =
            Admission::new_with_artifact_reservation(&options, Consistency::Frozen, 3, 0, artifact)
                .unwrap();
        assert_eq!(two.table_workers, 2);

        let resources = Resources::new(two);
        assert_eq!(resources.available_bytes(), 2 * pipeline);
        let reservation = resources.take_artifact_reservation().unwrap();
        assert_eq!(reservation.num_permits(), artifact);
        assert_eq!(resources.available_bytes(), 2 * pipeline);
        assert_eq!(
            resources.take_artifact_reservation().unwrap_err(),
            ResourceError::ArtifactReservationUnavailable
        );
        drop(reservation);
        assert_eq!(resources.available_bytes(), artifact + 2 * pipeline);
    }

    #[tokio::test]
    async fn retained_artifact_keeps_the_existing_workspace_charge_after_table_drop() {
        let options = options();
        let admitted = Admission::new(&options, Consistency::Frozen, 1, 0).unwrap();
        let resources = Resources::new(admitted);
        let (cancel_tx, mut cancel) = watch::channel(false);
        let table = resources.table(&mut cancel).await.unwrap();
        let retained = table.workspace();
        let workspace = admitted.layout.workspace_reservation;
        assert_eq!(retained.num_permits() as usize, workspace);
        drop(table);
        assert_eq!(
            resources.available_bytes(),
            admitted.memory_bytes - workspace
        );
        drop(retained);
        assert_eq!(resources.available_bytes(), admitted.memory_bytes);
        drop(cancel_tx);
    }

    #[test]
    fn schema_only_admission_still_requires_the_artifact_reservation() {
        let mut options = options();
        options.mode = MigrationMode::SchemaOnly;
        let artifact = 4096;
        options.memory_bytes = artifact - 1;
        assert_eq!(
            Admission::new_with_artifact_reservation(&options, Consistency::Frozen, 3, 0, artifact,),
            Err(ResourceError::InsufficientMemory {
                minimum: artifact,
                available: artifact - 1,
            })
        );

        options.memory_bytes = artifact;
        let admitted =
            Admission::new_with_artifact_reservation(&options, Consistency::Frozen, 3, 0, artifact)
                .unwrap();
        assert_eq!(admitted.table_workers, 0);
        let resources = Resources::new(admitted);
        assert_eq!(resources.available_bytes(), 0);
        assert_eq!(
            resources.take_artifact_reservation().unwrap().num_permits(),
            artifact
        );
    }
    #[test]
    fn snapshot_and_schema_only_admission_keep_their_own_connection_contracts() {
        let mut options = options();
        assert_eq!(
            Admission::new(&options, Consistency::SingleSnapshot, 3, 1),
            Err(ResourceError::SnapshotParallelism)
        );
        options.table_workers = 1;
        let admitted = Admission::new(&options, Consistency::SingleSnapshot, 3, 0).unwrap();
        assert_eq!(admitted.table_workers, 1);
        assert_eq!(admitted.connections.normal_source, 2);
        options.mode = MigrationMode::SchemaOnly;
        options.memory_bytes = 1;
        let admitted = Admission::new(&options, Consistency::SingleSnapshot, 3, 2).unwrap();
        assert_eq!((admitted.table_workers, admitted.index_workers), (0, 2));
        assert_eq!(admitted.connections.normal_source, 2);
        options.readers_per_table = 2;
        options.max_key_span = Some(10);
        // The two-reader request remains incompatible with a single source
        // snapshot even when no COPY tables will run in schema-only mode.
        assert_eq!(
            Admission::new(&options, Consistency::SingleSnapshot, 3, 1),
            Err(ResourceError::SnapshotParallelism)
        );
        let frozen = Admission::new(&options, Consistency::Frozen, 3, 1).unwrap();
        assert_eq!(
            (
                frozen.table_workers,
                frozen.reader_workers,
                frozen.index_workers
            ),
            (0, 0, 1)
        );
        assert_eq!(
            frozen.connections,
            ConnectionBounds {
                normal_source: 1,
                normal_target: 2,
                emergency_target: 2,
                peak_target: 4,
            }
        );
    }
    #[tokio::test]
    async fn cancellation_wins_ready_admission_and_false_closure_is_not_a_signal() {
        let resources =
            Resources::new(Admission::new(&options(), Consistency::Frozen, 1, 1).unwrap());
        let (sender, mut cancel) = watch::channel(false);
        sender.send(true).unwrap();
        drop(sender);
        assert_eq!(
            resources.table(&mut cancel).await.unwrap_err(),
            ResourceError::Cancelled
        );
        assert_eq!(
            resources.index(&mut cancel).await.unwrap_err(),
            ResourceError::Cancelled
        );
        assert_eq!(
            resources.batch(&mut cancel).await.unwrap_err(),
            ResourceError::Cancelled
        );
        assert_eq!(
            resources.available_bytes(),
            resources.admission.memory_bytes
        );
        let (_, mut closed_false) = watch::channel(false);
        let table = resources.table(&mut closed_false).await.unwrap();
        drop(table);
        resources.close();
        assert_eq!(
            resources.table(&mut closed_false).await.unwrap_err(),
            ResourceError::Closed
        );
    }
    #[tokio::test]
    async fn partial_pair_acquisition_and_workspace_are_released_on_cancel_or_error() {
        let resources =
            Resources::new(Admission::new(&options(), Consistency::Frozen, 1, 1).unwrap());
        let (sender, mut cancel) = watch::channel(false);
        let index = resources.index(&mut cancel).await.unwrap();
        let mut waiting = Box::pin(resources.table(&mut cancel));
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut waiting)
                .await
                .is_err()
        );
        assert_eq!(resources.tables.available_permits(), 0);
        sender.send(true).unwrap();
        assert_eq!(waiting.await.unwrap_err(), ResourceError::Cancelled);
        assert_eq!(resources.tables.available_permits(), 1);
        assert_eq!(
            resources.available_bytes(),
            resources.admission.memory_bytes
        );
        drop(index);
        let (_, mut cancel) = watch::channel(false);
        resources.bytes.close();
        assert_eq!(
            resources.table(&mut cancel).await.unwrap_err(),
            ResourceError::Closed
        );
        assert_eq!(resources.tables.available_permits(), 1);
        assert_eq!(resources.targets.available_permits(), 1);
    }
    async fn batch(resources: &Resources, cancel: &mut watch::Receiver<bool>) -> EncodedBatch {
        EncodedBatch {
            storage: Arc::new(BatchStorage {
                bytes: Bytes::from_static(b"1\n"),
                permit: resources.batch(cancel).await.unwrap(),
            }),
            rows: vec![RowPosition {
                start: 0,
                end: 2,
                locator: RowLocator {
                    ordinal: 1,
                    key: None,
                },
            }],
        }
    }
    #[tokio::test]
    async fn full_queue_cancel_retains_retry_bytes_and_returns_all_permits_after_teardown() {
        let mut options = options();
        options.memory_bytes = MemoryLayout::compute(&options)
            .unwrap()
            .pipeline_reservation;
        let resources =
            Resources::new(Admission::new(&options, Consistency::Frozen, 3, 0).unwrap());
        let (sender, mut cancel) = watch::channel(false);
        let table = resources.table(&mut cancel).await.unwrap();
        let (tx, rx) = mpsc::channel(resources.admission.queue_batches);
        let active = batch(&resources, &mut cancel).await;
        let retry_alias = active.storage.clone();
        for _ in 0..2 {
            tx.send(batch(&resources, &mut cancel).await).await.unwrap();
        }
        let filling = batch(&resources, &mut cancel).await;
        assert_eq!(resources.available_bytes(), 0);
        let mut send_cancel = cancel.clone();
        let mut blocked = Box::pin(async {
            tokio::select! {
                biased;
                _ = cancelled(&mut send_cancel) => Err(ResourceError::Cancelled),
                _ = tx.send(filling) => Ok(()),
            }
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut blocked)
                .await
                .is_err()
        );
        sender.send(true).unwrap();
        assert_eq!(blocked.await, Err(ResourceError::Cancelled));
        assert_eq!(
            resources.available_bytes(),
            resources.admission.layout.batch_reservation
        );
        drop(rx);
        drop(tx);
        drop(active);
        assert_eq!(resources.tables.available_permits(), 0);
        assert_eq!(resources.targets.available_permits(), 0);
        assert_eq!(
            resources.available_bytes(),
            3 * resources.admission.layout.batch_reservation
        );
        drop(retry_alias);
        drop(table);
        assert_eq!(resources.available_bytes(), options.memory_bytes);
        assert_eq!(resources.tables.available_permits(), 1);
        assert_eq!(resources.targets.available_permits(), 1);
    }
    #[tokio::test]
    async fn empty_queue_and_exhausted_byte_budget_wake_on_explicit_cancel() {
        let mut options = options();
        options.memory_bytes = MemoryLayout::compute(&options)
            .unwrap()
            .pipeline_reservation;
        let resources =
            Resources::new(Admission::new(&options, Consistency::Frozen, 1, 0).unwrap());
        let (sender, mut cancel) = watch::channel(false);
        let table = resources.table(&mut cancel).await.unwrap();
        let mut retained = Vec::new();
        for _ in 0..options.queue_batches + 2 {
            retained.push(batch(&resources, &mut cancel).await);
        }
        let mut permit_cancel = cancel.clone();
        let mut waiting = Box::pin(resources.batch(&mut permit_cancel));
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut waiting)
                .await
                .is_err()
        );
        let (_tx, mut rx) = mpsc::channel::<EncodedBatch>(1);
        let mut consumer = Box::pin(async {
            tokio::select! {
                biased;
                _ = cancelled(&mut cancel) => Err(ResourceError::Cancelled),
                _ = rx.recv() => Ok(()),
            }
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut consumer)
                .await
                .is_err()
        );
        sender.send(true).unwrap();
        assert_eq!(waiting.await.unwrap_err(), ResourceError::Cancelled);
        assert_eq!(consumer.await, Err(ResourceError::Cancelled));
        assert_eq!(resources.available_bytes(), 0);
        drop(retained);
        drop(table);
        assert_eq!(resources.available_bytes(), options.memory_bytes);
    }

    #[tokio::test]
    async fn every_admitted_table_can_fill_its_queue_and_index_capacity_is_separate() {
        let mut options = options();
        options.memory_bytes = 3 * MemoryLayout::compute(&options)
            .unwrap()
            .pipeline_reservation;
        let resources =
            Resources::new(Admission::new(&options, Consistency::Frozen, 3, 5).unwrap());
        let (_sender, mut cancel) = watch::channel(false);
        let mut tables = Vec::new();
        let mut retained = Vec::new();
        for _ in 0..3 {
            tables.push(resources.table(&mut cancel).await.unwrap());
            for _ in 0..options.queue_batches + 2 {
                retained.push(batch(&resources, &mut cancel).await);
            }
        }
        assert_eq!(resources.available_bytes(), 0);
        drop(retained);
        drop(tables);
        assert_eq!(resources.available_bytes(), options.memory_bytes);
        let first = resources.index(&mut cancel).await.unwrap();
        let second = resources.index(&mut cancel).await.unwrap();
        assert_eq!(resources.targets.available_permits(), 1);
        let mut waiting = Box::pin(resources.index(&mut cancel));
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut waiting)
                .await
                .is_err()
        );
        resources.close();
        assert_eq!(waiting.await.unwrap_err(), ResourceError::Closed);
        assert_eq!(resources.targets.available_permits(), 1);
        drop(first);
        drop(second);
        assert_eq!(resources.targets.available_permits(), 3);
        assert_eq!(resources.indexes.available_permits(), 2);
    }
}
