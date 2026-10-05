//! One owned writer process and one admitted payload; receipts follow sync success.
use super::{StoredValues, private_new, sync_directory};
use crate::model::{
    BatchStorage, Diagnostic, MigrationPlan, RawValue, RowLocator, RunReport, RunStatus, Severity,
    VerificationStatus,
};
use serde::Serialize;
use std::{
    fs,
    io::{self, Read, Write},
    mem::size_of,
    path::{Path, PathBuf},
    process::{Child, Command as ProcessCommand, Stdio},
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{ChildStdin, ChildStdout},
    sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch},
    time::Instant,
};

const MAGIC: [u8; 8] = *b"M2PGART1";
const HEADER: usize = 80;
const RESPONSE: usize = 48;
const BUFFER: usize = 16_384;
pub const WORKER_ARGUMENT: &str = "--internal-artifact-worker-v1";

#[derive(Debug, thiserror::Error)]
#[error(
    "artifact operation {sequence} failed ({kind:?}); last confirmed report generation {confirmed_generation}"
)]
pub struct ArtifactError {
    pub kind: ArtifactFailure,
    pub sequence: u64,
    pub confirmed_generation: u64,
    pub errno: Option<i32>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactFailure {
    Bounds,
    Busy,
    Closed,
    Io,
    /// Reject files collided before any reject bytes were written. Reports remain writable.
    Refused,
    Protocol,
    Unknown,
}
fn failure(kind: ArtifactFailure) -> ArtifactError {
    ArtifactError {
        kind,
        sequence: 0,
        confirmed_generation: 0,
        errno: None,
    }
}
type Result<T> = std::result::Result<T, ArtifactError>;
fn checked_add(a: usize, b: usize) -> Result<usize> {
    a.checked_add(b)
        .ok_or_else(|| failure(ArtifactFailure::Bounds))
}
fn checked_mul(a: usize, b: usize) -> Result<usize> {
    a.checked_mul(b)
        .ok_or_else(|| failure(ArtifactFailure::Bounds))
}

#[derive(Clone, Debug)]
pub struct ArtifactIdentity {
    pub run_id: String,
    pub directory: PathBuf,
    base: PathBuf,
}
impl ArtifactIdentity {
    pub fn new(base: &Path) -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let next = NEXT
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .map_err(|_| failure(ArtifactFailure::Bounds))?;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| failure(ArtifactFailure::Bounds))?
            .as_nanos();
        let run_id = format!("run-{nanos:x}-{:x}-{next:x}", std::process::id());
        Ok(Self {
            directory: base.join(&run_id),
            run_id,
            base: base.to_owned(),
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ArtifactLimits {
    pub payload_bytes: usize,
    pub max_copy_bytes: usize,
    pub tables: usize,
    pub ledger_bytes: usize,
    pub path_bytes: usize,
    pub fixed_bytes: usize,
    pub reservation_bytes: usize,
}
struct Count(usize);
impl Write for Count {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::other("artifact serialization overflow"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn count(value: &impl Serialize, pretty: bool) -> Result<usize> {
    let mut output = Count(0);
    if pretty {
        serde_json::to_writer_pretty(&mut output, value)
    } else {
        serde_json::to_writer(&mut output, value)
    }
    .map_err(|_| failure(ArtifactFailure::Bounds))?;
    checked_add(output.0, 1)
}
struct SliceWriter<'a> {
    bytes: &'a mut [u8],
    used: usize,
}
impl Write for SliceWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let end = self
            .used
            .checked_add(bytes.len())
            .filter(|&n| n <= self.bytes.len())
            .ok_or_else(|| io::Error::other("artifact payload exceeds admitted cap"))?;
        self.bytes[self.used..end].copy_from_slice(bytes);
        self.used = end;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[derive(Serialize)]
struct ConversionRecord<'a> {
    version: u32,
    kind: &'static str,
    table_id: &'a str,
    locator: &'a RowLocator,
    values: StoredValues<'a>,
    reason: &'a Diagnostic,
}
#[derive(Serialize)]
struct CopyRecord<'a> {
    version: u32,
    kind: &'static str,
    encoding: &'static str,
    table_id: &'a str,
    locator: &'a RowLocator,
    offset: u64,
    length: u64,
    reason: &'a Diagnostic,
}
fn conversion<'a>(
    table_id: &'a str,
    locator: &'a RowLocator,
    values: &'a [RawValue],
    reason: &'a Diagnostic,
) -> ConversionRecord<'a> {
    ConversionRecord {
        version: 1,
        kind: "conversion",
        table_id,
        locator,
        values: StoredValues(values),
        reason,
    }
}
fn copy_record<'a>(
    table_id: &'a str,
    locator: &'a RowLocator,
    offset: u64,
    length: u64,
    reason: &'a Diagnostic,
) -> CopyRecord<'a> {
    CopyRecord {
        version: 1,
        kind: "copy",
        encoding: "postgresql_copy_text",
        table_id,
        locator,
        offset,
        length,
        reason,
    }
}

impl ArtifactLimits {
    pub fn derive(
        identity: &ArtifactIdentity,
        plan: &MigrationPlan,
        report: &RunReport,
        max_row_bytes: usize,
    ) -> Result<Self> {
        if report.run_id != identity.run_id
            || report.artifact_dir != identity.directory.to_string_lossy()
        {
            return Err(failure(ArtifactFailure::Bounds));
        }
        if report.tables.len() != plan.tables.len()
            || report
                .tables
                .iter()
                .zip(&plan.tables)
                .any(|(r, p)| r.id != p.id)
        {
            return Err(failure(ArtifactFailure::Bounds));
        }
        let mut template = report.clone();
        template.status = RunStatus::Indeterminate;
        template.elapsed_millis = u64::MAX;
        template.verification.status = VerificationStatus::Unsupported;
        template.verification.tables_checked = usize::MAX;
        template.failed_steps = plan
            .ddl
            .iter()
            .map(|s| s.object.clone())
            .chain(plan.hooks.iter().map(|s| s.path.clone()))
            .collect();
        for (table, planned) in template.tables.iter_mut().zip(&plan.tables) {
            table.status = RunStatus::Indeterminate;
            table.rows_read = u64::MAX;
            table.committed_rows = u64::MAX;
            table.committed_bytes = u64::MAX;
            table.copy_elapsed_millis = u64::MAX;
            table.rejected_rows = u64::MAX;
            table.unresolved_rows = u64::MAX;
            table.indeterminate_rows = u64::MAX;
            table.transformations = planned
                .columns
                .iter()
                .filter(|c| c.transform.is_some())
                .map(|c| (c.source_name.clone(), u64::MAX))
                .collect();
        }
        template.diagnostics.push(Diagnostic {code:"ARTIFACT_SHUTDOWN_UNKNOWN".into(),stage:"artifacts".into(),object:None,severity:Severity::Error,message:"artifact persistence was not acknowledged; retain the confirmed immutable report generation and inspect the unacknowledged suffix".into()});
        let n = plan.tables.len();
        let base = identity
            .base
            .to_str()
            .ok_or_else(|| failure(ArtifactFailure::Bounds))?;
        let bootstrap = checked_add(
            checked_add(40, base.len())?,
            checked_add(identity.run_id.len(), checked_mul(n, 8)?)?,
        )?;
        let fixed_documents = count(plan, true)?
            .max(count(&template, true)?)
            .max(bootstrap);
        let samples = [
            RawValue::Null,
            RawValue::Int(i64::MIN),
            RawValue::UInt(u64::MAX),
            RawValue::Float(f32::from_bits(u32::MAX)),
            RawValue::Double(f64::from_bits(u64::MAX)),
            RawValue::Date {
                year: u16::MAX,
                month: u8::MAX,
                day: u8::MAX,
                hour: u8::MAX,
                minute: u8::MAX,
                second: u8::MAX,
                micros: u32::MAX,
            },
            RawValue::Time {
                negative: false,
                days: u32::MAX,
                hour: u8::MAX,
                minute: u8::MAX,
                second: u8::MAX,
                micros: u32::MAX,
            },
        ];
        let scalar = samples
            .iter()
            .map(|v| count(&StoredValues(std::slice::from_ref(v)), false).map(|n| n - 3))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .max()
            .expect("nonempty scalar domains");
        let bytes = count(&StoredValues(&[RawValue::Bytes(vec![])]), false)? - 3;
        let id = "t_ffffffffffffffff";
        let locator = RowLocator {
            ordinal: u64::MAX,
            key: None,
        };
        let reason = Diagnostic {
            code: "CONVERSION_ROW_REJECTED".into(),
            stage: "conversion".into(),
            object: Some(id.into()),
            severity: Severity::Warning,
            message: String::new(),
        };
        let envelope = count(&conversion(id, &locator, &[], &reason), false)?;
        let copy_envelope = count(
            &copy_record(id, &locator, u64::MAX, u64::MAX, &reason),
            false,
        )?;
        let mut payload = fixed_documents;
        if plan.mode != crate::config::MigrationMode::SchemaOnly {
            payload = payload.max(checked_add(copy_envelope, fixed_documents)?);
            for table in &plan.tables {
                let columns = table.columns.iter().filter(|c| c.copy).count();
                let remaining = max_row_bytes
                    .saturating_sub(checked_mul(columns, size_of::<mysql_async::Value>())?);
                let values = if columns == 0 {
                    0
                } else {
                    checked_add(
                        checked_mul(columns, scalar)?.max(checked_add(
                            checked_add(checked_mul(columns - 1, scalar)?, bytes)?,
                            checked_mul(remaining, 2)?,
                        )?),
                        columns - 1,
                    )?
                };
                payload = payload.max(checked_add(
                    checked_add(envelope, values)?,
                    fixed_documents,
                )?);
            }
        }
        if payload > u32::MAX as usize {
            return Err(failure(ArtifactFailure::Bounds));
        }
        let ledger_bytes = checked_mul(
            n,
            checked_add(size_of::<AckEntry>(), size_of::<WorkerTable>())?,
        )?;
        let directory = identity.directory.as_os_str().len();
        let path_bytes = checked_add(
            checked_mul(checked_add(base.len(), identity.run_id.len())?, 2)?,
            checked_mul(checked_add(directory, 64)?, 6)?,
        )?;
        let fixed_bytes = fixed_reservation();
        let reservation_bytes = checked_add(
            checked_add(checked_add(payload, ledger_bytes)?, path_bytes)?,
            fixed_bytes,
        )?;
        if reservation_bytes > u32::MAX as usize {
            return Err(failure(ArtifactFailure::Bounds));
        }
        Ok(Self {
            payload_bytes: payload,
            max_copy_bytes: if plan.mode == crate::config::MigrationMode::SchemaOnly {
                0
            } else {
                max_row_bytes
            },
            tables: n,
            ledger_bytes,
            path_bytes,
            fixed_bytes,
            reservation_bytes,
        })
    }
}

fn fixed_reservation() -> usize {
    2 * BUFFER
        + 2 * (HEADER + RESPONSE)
        + size_of::<Command>()
        + size_of::<Life>()
        + size_of::<Global>()
        + size_of::<Flight>()
        + size_of::<ChildSlot>()
        + size_of::<Worker>()
        + size_of::<DurableArtifacts>()
        + 6 * size_of::<usize>()
}
impl ArtifactLimits {
    fn validate(&self, identity: &ArtifactIdentity, tables: usize) -> Result<()> {
        let base = identity
            .base
            .to_str()
            .ok_or_else(|| failure(ArtifactFailure::Bounds))?;
        let directory = identity.directory.as_os_str().len();
        let paths = checked_add(
            checked_mul(checked_add(base.len(), identity.run_id.len())?, 2)?,
            checked_mul(checked_add(directory, 64)?, 6)?,
        )?;
        let ledger = checked_mul(
            tables,
            checked_add(size_of::<AckEntry>(), size_of::<WorkerTable>())?,
        )?;
        let bootstrap = checked_add(
            checked_add(40, base.len())?,
            checked_add(identity.run_id.len(), checked_mul(tables, 8)?)?,
        )?;
        let reservation = checked_add(
            checked_add(checked_add(self.payload_bytes, ledger)?, paths)?,
            fixed_reservation(),
        )?;
        if self.tables != tables
            || self.ledger_bytes != ledger
            || self.path_bytes != paths
            || self.fixed_bytes != fixed_reservation()
            || self.reservation_bytes != reservation
            || reservation > u32::MAX as usize
            || self.payload_bytes < bootstrap
            || identity.directory != identity.base.join(&identity.run_id)
        {
            return Err(failure(ArtifactFailure::Bounds));
        }
        Ok(())
    }
}

pub struct OwnedRejectBudget {
    limit: u64,
    used: AtomicU64,
}
impl OwnedRejectBudget {
    pub fn new(limit: u64) -> Arc<Self> {
        Arc::new(Self {
            limit,
            used: AtomicU64::new(0),
        })
    }
    pub fn used(&self) -> u64 {
        self.used.load(Ordering::Acquire)
    }
    pub fn reserve(self: &Arc<Self>) -> Option<OwnedRejectTicket> {
        self.used
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.limit).then(|| n + 1)
            })
            .ok()
            .map(|_| OwnedRejectTicket {
                budget: self.clone(),
                durable: false,
            })
    }
}
pub struct OwnedRejectTicket {
    budget: Arc<OwnedRejectBudget>,
    durable: bool,
}
impl OwnedRejectTicket {
    pub(crate) fn commit(&mut self) {
        self.durable = true;
    }
}
impl Drop for OwnedRejectTicket {
    fn drop(&mut self) {
        if !self.durable {
            self.budget.used.fetch_sub(1, Ordering::AcqRel);
        }
    }
}
pub struct RawRetention {
    pub values: Vec<RawValue>,
    pub permit: Arc<OwnedSemaphorePermit>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AckEntry {
    pub table_id: u64,
    pub rejected_rows: u64,
    pub last_sequence: u64,
    pub copy_offset: u64,
}
#[derive(Clone, Debug, Default)]
pub struct Confirmed {
    pub sequence: u64,
    pub report_generation: u64,
    pub tables: Vec<AckEntry>,
    pub pending_sequence: Option<u64>,
    pub unreaped_pid: Option<u32>,
}
#[derive(Clone, Copy, Debug)]
pub struct Receipt {
    pub sequence: u64,
    pub report_generation: u64,
    pub copy_offset: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum Op {
    Create = 1,
    Plan = 2,
    Report = 3,
    Conversion = 4,
    Copy = 5,
    Flush = 6,
    Close = 7,
}
impl Op {
    fn decode(v: u8) -> io::Result<Self> {
        match v {
            1 => Ok(Self::Create),
            2 => Ok(Self::Plan),
            3 => Ok(Self::Report),
            4 => Ok(Self::Conversion),
            5 => Ok(Self::Copy),
            6 => Ok(Self::Flush),
            7 => Ok(Self::Close),
            _ => Err(io::Error::other("invalid artifact operation")),
        }
    }
}
#[derive(Clone, Copy)]
struct Header {
    op: Op,
    sequence: u64,
    generation: u64,
    table: u64,
    offset: u64,
    data: u64,
    payload: u64,
    confirmed: u64,
}
fn put(out: &mut [u8], offset: usize, value: u64) {
    out[offset..offset + 8].copy_from_slice(&value.to_be_bytes());
}
fn get(input: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes(
        input[offset..offset + 8]
            .try_into()
            .expect("fixed frame field"),
    )
}
impl Header {
    fn encode(self) -> [u8; HEADER] {
        let mut b = [0; HEADER];
        b[..8].copy_from_slice(&MAGIC);
        b[8..10].copy_from_slice(&1u16.to_be_bytes());
        b[10] = self.op as u8;
        put(&mut b, 12, self.sequence);
        put(&mut b, 20, self.generation);
        if matches!(self.op, Op::Copy | Op::Conversion) {
            b[28..46].copy_from_slice(format!("t_{:016x}", self.table).as_bytes());
        }
        put(&mut b, 48, self.offset);
        put(&mut b, 56, self.data);
        put(&mut b, 64, self.payload);
        put(&mut b, 72, self.confirmed);
        b
    }
    fn decode(b: &[u8; HEADER]) -> io::Result<Self> {
        if b[..8] != MAGIC || b[8..10] != 1u16.to_be_bytes() || b[11] != 0 || b[46..48] != [0, 0] {
            return Err(io::Error::other("invalid artifact frame"));
        }
        let op = Op::decode(b[10])?;
        let table = if matches!(op, Op::Copy | Op::Conversion) {
            parse_id(std::str::from_utf8(&b[28..46]).map_err(io::Error::other)?)
                .map_err(|_| io::Error::other("invalid artifact table"))?
        } else {
            if b[28..46].iter().any(|&b| b != 0) {
                return Err(io::Error::other("unexpected artifact table"));
            }
            0
        };
        Ok(Self {
            op,
            sequence: get(b, 12),
            generation: get(b, 20),
            table,
            offset: get(b, 48),
            data: get(b, 56),
            payload: get(b, 64),
            confirmed: get(b, 72),
        })
    }
}
fn parse_id(id: &str) -> Result<u64> {
    if id.len() != 18
        || !id.starts_with("t_")
        || !id[2..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(failure(ArtifactFailure::Bounds));
    }
    u64::from_str_radix(&id[2..], 16).map_err(|_| failure(ArtifactFailure::Bounds))
}

struct Life {
    token: u64,
    identity: ArtifactIdentity,
    ledger: Mutex<Confirmed>,
    reaped: AtomicBool,
    actor_done: AtomicBool,
    cancelled: AtomicBool,
    rejects_failed: AtomicBool,
    notify: Notify,
    budget: Mutex<Option<Arc<OwnedRejectBudget>>>,
    _memory: OwnedSemaphorePermit,
}
enum Retention {
    None,
    Raw(RawRetention),
    Copy {
        storage: Arc<BatchStorage>,
        start: usize,
        end: usize,
    },
}
struct Flight {
    expected_offset: u64,
    header: Header,
    body: Box<[u8]>,
    length: usize,
    retention: Retention,
    ticket: Mutex<Option<OwnedRejectTicket>>,
    _permit: OwnedSemaphorePermit,
}
struct Command {
    flight: Arc<Flight>,
    reply: oneshot::Sender<Result<Receipt>>,
}
type WorkerPipes = (std::process::ChildStdin, std::process::ChildStdout);
struct StartJob {
    command: ProcessCommand,
    reply: oneshot::Sender<io::Result<WorkerPipes>>,
}
struct ChildSlot {
    child: Option<Child>,
    start: Option<StartJob>,
    spawning: bool,
    spawn_finished: bool,
    life: Arc<Life>,
    pending: Option<Arc<Flight>>,
}
struct Global {
    slot: Mutex<Option<ChildSlot>>,
    changed: Condvar,
}
fn global() -> Result<&'static Arc<Global>> {
    static GLOBAL: OnceLock<std::result::Result<Arc<Global>, ()>> = OnceLock::new();
    GLOBAL
        .get_or_init(|| {
            let state = Arc::new(Global {
                slot: Mutex::new(None),
                changed: Condvar::new(),
            });
            let worker = state.clone();
            std::thread::Builder::new()
                .name("my2pg-artifact-reaper".into())
                .spawn(move || {
                    loop {
                        let mut slot = worker
                            .slot
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        let Some(current) = slot.as_mut() else {
                            let _guard = worker
                                .changed
                                .wait(slot)
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            continue;
                        };

                        if current.child.is_none() && !current.spawning {
                            if let Some(mut job) = current.start.take() {
                                if current.life.cancelled.load(Ordering::Acquire) {
                                    current.spawn_finished = true;
                                    current.life.reaped.store(true, Ordering::Release);
                                    current.life.actor_done.store(true, Ordering::Release);
                                    current.life.notify.notify_waiters();
                                    drop(slot);
                                    let _ = job.reply.send(Err(io::Error::other(
                                        "artifact startup cancelled before spawn",
                                    )));
                                    worker.changed.notify_one();
                                    continue;
                                }
                                current.spawning = true;
                                let token = current.life.token;
                                drop(slot);

                                // std::process::Command::spawn may block in the OS
                                // fork/exec handshake. This sole reaper owns that
                                // blocking call, outside every shared mutex.
                                let spawned = job.command.spawn().and_then(|mut child| {
                                    let Some(stdin) = child.stdin.take() else {
                                        let _ = child.kill();
                                        let _ = child.wait();
                                        return Err(io::Error::other(
                                            "artifact worker stdin unavailable",
                                        ));
                                    };
                                    let Some(stdout) = child.stdout.take() else {
                                        let _ = child.kill();
                                        let _ = child.wait();
                                        return Err(io::Error::other(
                                            "artifact worker stdout unavailable",
                                        ));
                                    };
                                    Ok((child, stdin, stdout))
                                });

                                let mut slot = worker
                                    .slot
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                let Some(current) =
                                    slot.as_mut().filter(|slot| slot.life.token == token)
                                else {
                                    if let Ok((mut child, _, _)) = spawned {
                                        let _ = child.kill();
                                        let _ = child.wait();
                                    }
                                    continue;
                                };
                                current.spawning = false;
                                current.spawn_finished = true;
                                match spawned {
                                    Ok((child, stdin, stdout)) => {
                                        let pid = child.id();
                                        current
                                            .life
                                            .ledger
                                            .lock()
                                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                                            .unreaped_pid = Some(pid);
                                        current.child = Some(child);
                                        let pipes =
                                            if current.life.cancelled.load(Ordering::Acquire) {
                                                Err(io::Error::other("artifact startup cancelled"))
                                            } else {
                                                Ok((stdin, stdout))
                                            };
                                        if let Err(pipes) = job.reply.send(pipes) {
                                            drop(pipes);
                                            current.life.cancelled.store(true, Ordering::Release);
                                            current.life.actor_done.store(true, Ordering::Release);
                                        }
                                        if current.life.cancelled.load(Ordering::Acquire)
                                            && let Some(child) = current.child.as_mut()
                                        {
                                            let _ = child.kill();
                                        }
                                    }
                                    Err(error) => {
                                        current.life.reaped.store(true, Ordering::Release);
                                        current.life.actor_done.store(true, Ordering::Release);
                                        current
                                            .life
                                            .ledger
                                            .lock()
                                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                                            .unreaped_pid = None;
                                        current.life.notify.notify_waiters();
                                        let _ = job.reply.send(Err(error));
                                    }
                                }
                                worker.changed.notify_one();
                                continue;
                            }
                            if current.spawn_finished
                                && current.life.actor_done.load(Ordering::Acquire)
                            {
                                current.life.reaped.store(true, Ordering::Release);
                                current.life.notify.notify_waiters();
                                drop(slot.take());
                                continue;
                            }
                        }

                        if let Some(child) = current.child.as_mut()
                            && matches!(child.try_wait(), Ok(Some(_)))
                        {
                            current.life.reaped.store(true, Ordering::Release);
                            current
                                .life
                                .ledger
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .unreaped_pid = None;
                            current.life.notify.notify_waiters();
                            if current.life.actor_done.load(Ordering::Acquire) {
                                drop(slot.take());
                                continue;
                            }
                        }
                        let _guard = worker
                            .changed
                            .wait_timeout(slot, Duration::from_millis(25))
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                    }
                })
                .map_err(|_| ())?;
            Ok(state)
        })
        .as_ref()
        .map_err(|_| failure(ArtifactFailure::Io))
}
fn kill_owned(life: &Life) {
    if let Ok(global) = global() {
        let mut slot = global
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(child) = slot.as_mut()
            && child.life.token == life.token
        {
            life.cancelled.store(true, Ordering::Release);
            if let Some(process) = child.child.as_mut() {
                let _ = process.kill();
            }
            global.changed.notify_one();
        }
    }
}

struct StartGuard(Option<Arc<Life>>);
impl Drop for StartGuard {
    fn drop(&mut self) {
        if let Some(life) = &self.0 {
            life.actor_done.store(true, Ordering::Release);
            if !life.reaped.load(Ordering::Acquire) {
                kill_owned(life);
            }
        }
    }
}

/// Allocation-free lifecycle evidence, including when a cancelled start never
/// returned its facade. A present slot must prevent another child from starting.
#[derive(Clone, Copy, Debug)]
pub struct WorkerState {
    pub starting: bool,
    pub unreaped_pid: Option<u32>,
    pub report_generation: u64,
    pub pending_sequence: Option<u64>,
}
pub fn live_worker() -> Result<Option<WorkerState>> {
    let slot = global()?
        .slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(slot.as_ref().map(|child| {
        let ledger = child
            .life
            .ledger
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        WorkerState {
            starting: child.child.is_none() && !child.life.reaped.load(Ordering::Acquire),
            unreaped_pid: ledger.unreaped_pid,
            report_generation: ledger.report_generation,
            pending_sequence: ledger.pending_sequence,
        }
    }))
}
struct ActorGuard(Arc<Life>);
impl Drop for ActorGuard {
    fn drop(&mut self) {
        self.0.actor_done.store(true, Ordering::Release);
        self.0.notify.notify_waiters();
        if !self.0.reaped.load(Ordering::Acquire) {
            kill_owned(&self.0);
        }
        if let Ok(global) = global() {
            global.changed.notify_one();
        }
    }
}

pub struct DurableArtifacts {
    life: Arc<Life>,
    limits: ArtifactLimits,
    tx: mpsc::Sender<Command>,
    operation: Arc<Semaphore>,
    stop: watch::Sender<Option<Instant>>,
    next: AtomicU64,
}
pub struct ArtifactLease<'a> {
    writer: &'a DurableArtifacts,
    permit: OwnedSemaphorePermit,
}

impl DurableArtifacts {
    pub async fn start(
        mut command: ProcessCommand,
        identity: ArtifactIdentity,
        limits: ArtifactLimits,
        table_ids: &[String],
        memory: OwnedSemaphorePermit,
    ) -> Result<Self> {
        limits.validate(&identity, table_ids.len())?;
        if memory.num_permits() < limits.reservation_bytes || table_ids.len() != limits.tables {
            return Err(failure(ArtifactFailure::Bounds));
        }
        let mut tables = Vec::with_capacity(table_ids.len());
        for id in table_ids {
            let table_id = parse_id(id)?;
            if tables.iter().any(|t: &AckEntry| t.table_id == table_id) {
                return Err(failure(ArtifactFailure::Bounds));
            }
            tables.push(AckEntry {
                table_id,
                ..Default::default()
            });
        }
        command
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        static TOKEN: AtomicU64 = AtomicU64::new(1);
        let token = TOKEN
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .map_err(|_| failure(ArtifactFailure::Bounds))?;
        let life = Arc::new(Life {
            token,
            identity,
            ledger: Mutex::new(Confirmed {
                tables,
                unreaped_pid: None,
                ..Default::default()
            }),
            reaped: AtomicBool::new(false),
            actor_done: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            rejects_failed: AtomicBool::new(false),
            notify: Notify::new(),
            budget: Mutex::new(None),
            _memory: memory,
        });
        let (reply, started) = oneshot::channel();
        let global = global()?;
        {
            let mut slot = global
                .slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if slot.is_some() {
                return Err(failure(ArtifactFailure::Busy));
            }
            *slot = Some(ChildSlot {
                child: None,
                start: Some(StartJob { command, reply }),
                spawning: false,
                spawn_finished: false,
                life: life.clone(),
                pending: None,
            });
        }
        global.changed.notify_one();

        let mut startup = StartGuard(Some(life.clone()));
        let (stdin, stdout) = started
            .await
            .map_err(|_| failure(ArtifactFailure::Io))?
            .map_err(|_| failure(ArtifactFailure::Io))?;
        startup.0.take();
        let guard = ActorGuard(life.clone());
        let stdin = ChildStdin::from_std(stdin).map_err(|_| {
            kill_owned(&life);
            failure(ArtifactFailure::Io)
        })?;
        let stdout = ChildStdout::from_std(stdout).map_err(|_| {
            kill_owned(&life);
            failure(ArtifactFailure::Io)
        })?;
        let (tx, rx) = mpsc::channel(1);
        let (stop, stopped) = watch::channel(None);
        let operation = Arc::new(Semaphore::new(1));
        tokio::spawn(actor(guard, operation.clone(), stdin, stdout, rx, stopped));
        let writer = Self {
            life,
            limits,
            tx,
            operation,
            stop,
            next: AtomicU64::new(1),
        };
        let lease = writer.lease().await?;
        let mut body = lease.buffer()?;
        let base = writer
            .life
            .identity
            .base
            .to_str()
            .ok_or_else(|| failure(ArtifactFailure::Bounds))?
            .as_bytes();
        let id = writer.life.identity.run_id.as_bytes();
        let mut encoder = SliceWriter {
            bytes: &mut body,
            used: 0,
        };
        for n in [
            limits.payload_bytes as u64,
            limits.max_copy_bytes as u64,
            limits.tables as u64,
            base.len() as u64,
            id.len() as u64,
        ] {
            encoder
                .write_all(&n.to_be_bytes())
                .map_err(|_| failure(ArtifactFailure::Bounds))?;
        }
        encoder
            .write_all(base)
            .and_then(|()| encoder.write_all(id))
            .map_err(|_| failure(ArtifactFailure::Bounds))?;
        for table in &writer
            .life
            .ledger
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .tables
        {
            encoder
                .write_all(&table.table_id.to_be_bytes())
                .map_err(|_| failure(ArtifactFailure::Bounds))?;
        }
        let length = encoder.used;
        let result = lease
            .submit(Op::Create, 0, 0, 0, body, length, Retention::None, None)
            .await;
        if result.is_err() {
            kill_owned(&writer.life);
        }
        result?;
        Ok(writer)
    }
    pub fn identity(&self) -> &ArtifactIdentity {
        &self.life.identity
    }
    pub fn limits(&self) -> ArtifactLimits {
        self.limits
    }
    pub fn confirmed(&self) -> Confirmed {
        self.life
            .ledger
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
    pub async fn lease(&self) -> Result<ArtifactLease<'_>> {
        Ok(ArtifactLease {
            writer: self,
            permit: self
                .operation
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| failure(ArtifactFailure::Closed))?,
        })
    }
    pub async fn flush(&self) -> Result<Receipt> {
        let lease = self.lease().await?;
        let body = lease.buffer()?;
        lease
            .submit(Op::Flush, 0, 0, 0, body, 0, Retention::None, None)
            .await
    }
    pub async fn shutdown(&self, deadline: Instant) -> Confirmed {
        self.operation.close();
        let _ = self.stop.send(Some(deadline));
        loop {
            let notified = self.life.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.life.reaped.load(Ordering::Acquire)
                && self.life.actor_done.load(Ordering::Acquire)
            {
                break;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                kill_owned(&self.life);
                break;
            }
        }
        self.confirmed()
    }
}
impl Drop for DurableArtifacts {
    fn drop(&mut self) {
        self.operation.close();
        let _ = self.stop.send(Some(Instant::now()));
        kill_owned(&self.life);
    }
}
fn context_failure(writer: &DurableArtifacts, kind: ArtifactFailure) -> ArtifactError {
    let ledger = writer
        .life
        .ledger
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    ArtifactError {
        kind,
        sequence: ledger.pending_sequence.unwrap_or(0),
        confirmed_generation: ledger.report_generation,
        errno: None,
    }
}
impl ArtifactLease<'_> {
    fn buffer(&self) -> Result<Box<[u8]>> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(self.writer.limits.payload_bytes)
            .map_err(|_| context_failure(self.writer, ArtifactFailure::Bounds))?;
        bytes.resize(self.writer.limits.payload_bytes, 0);
        Ok(bytes.into_boxed_slice())
    }
    fn encode(&self, value: &impl Serialize, pretty: bool) -> Result<(Box<[u8]>, usize)> {
        if count(value, pretty)? > self.writer.limits.payload_bytes {
            return Err(context_failure(self.writer, ArtifactFailure::Bounds));
        }
        let mut body = self.buffer()?;
        let mut encoder = SliceWriter {
            bytes: &mut body,
            used: 0,
        };
        if pretty {
            serde_json::to_writer_pretty(&mut encoder, value)
        } else {
            serde_json::to_writer(&mut encoder, value)
        }
        .map_err(|_| context_failure(self.writer, ArtifactFailure::Bounds))?;
        encoder
            .write_all(b"\n")
            .map_err(|_| context_failure(self.writer, ArtifactFailure::Bounds))?;
        let length = encoder.used;
        Ok((body, length))
    }
    pub async fn write_plan(self, plan: &MigrationPlan) -> Result<Receipt> {
        let (body, length) = self.encode(plan, true)?;
        self.submit(Op::Plan, 0, 0, 0, body, length, Retention::None, None)
            .await
    }
    pub async fn write_report(self, generation: u64, report: &RunReport) -> Result<Receipt> {
        if generation == 0
            || report.run_id != self.writer.life.identity.run_id
            || report.artifact_dir != self.writer.life.identity.directory.to_string_lossy()
        {
            return Err(context_failure(self.writer, ArtifactFailure::Bounds));
        }
        let (body, length) = self.encode(report, true)?;
        self.submit(
            Op::Report,
            generation,
            0,
            0,
            body,
            length,
            Retention::None,
            None,
        )
        .await
    }
    pub async fn reject_conversion(
        self,
        table_index: usize,
        locator: &RowLocator,
        raw: RawRetention,
        reason: &Diagnostic,
        ticket: OwnedRejectTicket,
    ) -> Result<Receipt> {
        if self.writer.limits.max_copy_bytes == 0 || locator.ordinal == 0 {
            return Err(context_failure(self.writer, ArtifactFailure::Bounds));
        }
        let allocated = raw.values.iter().try_fold(
            checked_mul(raw.values.capacity(), size_of::<RawValue>())?,
            |total, value| {
                checked_add(
                    total,
                    match value {
                        RawValue::Bytes(bytes) => bytes.capacity(),
                        _ => 0,
                    },
                )
            },
        )?;
        let logical = raw.values.iter().try_fold(
            checked_mul(raw.values.len(), size_of::<mysql_async::Value>())?,
            |total, value| {
                checked_add(
                    total,
                    match value {
                        RawValue::Bytes(bytes) => bytes.len(),
                        _ => 0,
                    },
                )
            },
        )?;
        if allocated > raw.permit.num_permits() || logical > self.writer.limits.max_copy_bytes {
            return Err(context_failure(self.writer, ArtifactFailure::Bounds));
        }
        let table = self.table(table_index)?;
        let id = format!("t_{:016x}", table.table_id);
        let (body, length) = self.encode(&conversion(&id, locator, &raw.values, reason), false)?;
        self.submit(
            Op::Conversion,
            0,
            table.table_id,
            0,
            body,
            length,
            Retention::Raw(raw),
            Some(ticket),
        )
        .await
    }
    pub async fn reject_copy(
        self,
        table_index: usize,
        locator: &RowLocator,
        storage: Arc<BatchStorage>,
        range: std::ops::Range<usize>,
        reason: &Diagnostic,
        ticket: OwnedRejectTicket,
    ) -> Result<Receipt> {
        if locator.ordinal == 0
            || range.start >= range.end
            || range.end > storage.bytes.len()
            || storage.bytes.len() > storage.permit.num_permits()
            || range.len() > self.writer.limits.max_copy_bytes
        {
            return Err(context_failure(self.writer, ArtifactFailure::Bounds));
        }
        let table = self.table(table_index)?;
        let id = format!("t_{:016x}", table.table_id);
        let (body, length) = self.encode(
            &copy_record(&id, locator, table.copy_offset, range.len() as u64, reason),
            false,
        )?;
        self.submit(
            Op::Copy,
            0,
            table.table_id,
            table.copy_offset,
            body,
            length,
            Retention::Copy {
                storage,
                start: range.start,
                end: range.end,
            },
            Some(ticket),
        )
        .await
    }
    fn table(&self, index: usize) -> Result<AckEntry> {
        self.writer
            .life
            .ledger
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .tables
            .get(index)
            .copied()
            .ok_or_else(|| context_failure(self.writer, ArtifactFailure::Bounds))
    }
    #[allow(clippy::too_many_arguments)]
    async fn submit(
        self,
        op: Op,
        generation: u64,
        table: u64,
        offset: u64,
        body: Box<[u8]>,
        length: usize,
        retention: Retention,
        ticket: Option<OwnedRejectTicket>,
    ) -> Result<Receipt> {
        if matches!(op, Op::Copy | Op::Conversion)
            && self.writer.life.rejects_failed.load(Ordering::Acquire)
        {
            return Err(context_failure(self.writer, ArtifactFailure::Refused));
        }
        if let Some(ticket) = &ticket {
            let mut budget = self
                .writer
                .life
                .budget
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(existing) = &*budget {
                if !Arc::ptr_eq(existing, &ticket.budget) {
                    return Err(context_failure(self.writer, ArtifactFailure::Bounds));
                }
            } else {
                *budget = Some(ticket.budget.clone());
            }
        }
        if self.writer.stop.borrow().is_some() {
            return Err(context_failure(self.writer, ArtifactFailure::Closed));
        }
        let sequence = self
            .writer
            .next
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .map_err(|_| context_failure(self.writer, ArtifactFailure::Bounds))?;
        let confirmed = self
            .writer
            .life
            .ledger
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .report_generation;
        let data = match &retention {
            Retention::Copy { start, end, .. } => (end - start) as u64,
            _ => 0,
        };
        let expected_offset = if op == Op::Conversion {
            self.writer
                .life
                .ledger
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .tables
                .iter()
                .find(|t| t.table_id == table)
                .ok_or_else(|| context_failure(self.writer, ArtifactFailure::Bounds))?
                .copy_offset
        } else {
            offset
                .checked_add(data)
                .ok_or_else(|| context_failure(self.writer, ArtifactFailure::Bounds))?
        };
        let flight = Arc::new(Flight {
            expected_offset,
            header: Header {
                op,
                sequence,
                generation,
                table,
                offset,
                data,
                payload: length as u64,
                confirmed,
            },
            body,
            length,
            retention,
            ticket: Mutex::new(ticket),
            _permit: self.permit,
        });
        let (reply, receive) = oneshot::channel();
        {
            let mut slot = global()?
                .slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let child = slot
                .as_mut()
                .filter(|c| c.life.token == self.writer.life.token)
                .ok_or_else(|| context_failure(self.writer, ArtifactFailure::Closed))?;
            if child.pending.is_some() {
                return Err(context_failure(self.writer, ArtifactFailure::Busy));
            }
            child.pending = Some(flight.clone());
            self.writer
                .life
                .ledger
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .pending_sequence = Some(sequence);
        }
        if self.writer.tx.try_send(Command { flight, reply }).is_err() {
            kill_owned(&self.writer.life);
            return Err(context_failure(self.writer, ArtifactFailure::Closed));
        }
        receive
            .await
            .map_err(|_| context_failure(self.writer, ArtifactFailure::Unknown))?
    }
}

fn response(
    header: Header,
    result: u16,
    errno: i32,
    generation: u64,
    offset: u64,
) -> [u8; RESPONSE] {
    let mut b = [0; RESPONSE];
    b[..8].copy_from_slice(&MAGIC);
    b[8..10].copy_from_slice(&1u16.to_be_bytes());
    b[10..12].copy_from_slice(&result.to_be_bytes());
    put(&mut b, 12, header.sequence);
    put(&mut b, 20, generation);
    put(&mut b, 28, offset);
    b[36..40].copy_from_slice(&errno.to_be_bytes());
    b
}
async fn transmit(
    stdin: &mut ChildStdin,
    stdout: &mut ChildStdout,
    flight: &Flight,
) -> Result<Receipt> {
    let mut result = [0; RESPONSE];
    let io_result = async {
        stdin.write_all(&flight.header.encode()).await?;
        if let Retention::Raw(raw) = &flight.retention {
            let _ = (&raw.values, &raw.permit);
        }
        if let Retention::Copy {
            storage,
            start,
            end,
        } = &flight.retention
        {
            stdin.write_all(&storage.bytes[*start..*end]).await?;
        }
        stdin.write_all(&flight.body[..flight.length]).await?;
        stdin.flush().await?;
        stdout.read_exact(&mut result).await?;
        Ok::<_, io::Error>(())
    }
    .await;
    if io_result.is_err() {
        return Err(failure(ArtifactFailure::Unknown));
    }
    if result[..8] != MAGIC
        || result[8..10] != 1u16.to_be_bytes()
        || get(&result, 12) != flight.header.sequence
        || result[40..].iter().any(|&b| b != 0)
    {
        return Err(failure(ArtifactFailure::Protocol));
    }
    let status = u16::from_be_bytes(result[10..12].try_into().expect("fixed response"));
    if status != 0 {
        let refused = status == 2
            && matches!(flight.header.op, Op::Copy | Op::Conversion)
            && get(&result, 20) == flight.header.confirmed
            && get(&result, 28) == 0;
        return Err(ArtifactError {
            kind: if status == 1 {
                ArtifactFailure::Io
            } else if refused {
                ArtifactFailure::Refused
            } else {
                ArtifactFailure::Protocol
            },
            sequence: flight.header.sequence,
            confirmed_generation: flight.header.confirmed,
            errno: Some(i32::from_be_bytes(
                result[36..40].try_into().expect("fixed errno"),
            )),
        });
    }
    let receipt = Receipt {
        sequence: get(&result, 12),
        report_generation: get(&result, 20),
        copy_offset: get(&result, 28),
    };
    let generation = if flight.header.op == Op::Report {
        flight.header.generation
    } else {
        flight.header.confirmed
    };
    if receipt.report_generation != generation
        || result[36..40] != [0; 4]
        || receipt.copy_offset != flight.expected_offset
    {
        return Err(failure(ArtifactFailure::Protocol));
    }
    Ok(receipt)
}
async fn actor(
    guard: ActorGuard,
    operation: Arc<Semaphore>,
    mut stdin: ChildStdin,
    mut stdout: ChildStdout,
    mut commands: mpsc::Receiver<Command>,
    mut stop: watch::Receiver<Option<Instant>>,
) {
    let life = guard.0.clone();
    let _guard = guard;
    let mut failed = false;
    loop {
        let command = tokio::select! {biased;_ = stop.changed()=>break,command=commands.recv()=>match command {Some(c)=>c,None=>break}};
        let Command { flight, reply } = command;
        let result = {
            let io = transmit(&mut stdin, &mut stdout, &flight);
            tokio::pin!(io);
            loop {
                let deadline = *stop.borrow();
                if let Some(deadline) = deadline {
                    break tokio::time::timeout_at(deadline, &mut io)
                        .await
                        .unwrap_or_else(|_| Err(failure(ArtifactFailure::Unknown)));
                }
                tokio::select! {biased;changed=stop.changed()=>{if changed.is_err(){break Err(failure(ArtifactFailure::Closed));}},result=&mut io=>break result}
            }
        };
        if let Ok(receipt) = &result {
            let mut ledger = life
                .ledger
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(ticket) = flight
                .ticket
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_mut()
            {
                ticket.commit();
                let table = ledger
                    .tables
                    .iter_mut()
                    .find(|t| t.table_id == flight.header.table)
                    .expect("validated parent table");
                table.rejected_rows += 1;
                table.last_sequence = receipt.sequence;
                table.copy_offset = receipt.copy_offset;
            }
            ledger.sequence = receipt.sequence;
            ledger.report_generation = receipt.report_generation;
            ledger.pending_sequence = None;
        }
        let refused = result
            .as_ref()
            .is_err_and(|error| error.kind == ArtifactFailure::Refused);
        if refused {
            life.rejects_failed.store(true, Ordering::Release);
            let mut ledger = life
                .ledger
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // The full frame was consumed, but no reject bytes were written or acknowledged.
            ledger.sequence = flight.header.sequence;
            ledger.pending_sequence = None;
        }
        if result.is_ok() || refused {
            let mut slot = global()
                .expect("initialized reaper")
                .slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(child) = slot.as_mut()
                && child.life.token == life.token
            {
                child.pending = None;
            }
        }
        let result = result.map_err(|mut error| {
            error.sequence = flight.header.sequence;
            error.confirmed_generation = flight.header.confirmed;
            error
        });
        failed = result.is_err() && !refused;
        drop(flight);
        let _ = reply.send(result);
        if failed {
            break;
        }
        if stop.borrow().is_some() {
            break;
        }
    }
    operation.close();
    let shutdown_deadline = *stop.borrow();
    if !failed && let Some(deadline) = shutdown_deadline {
        let (sequence, generation) = {
            let ledger = life
                .ledger
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (ledger.sequence, ledger.report_generation)
        };
        let closing = async {
            let header = Header {
                op: Op::Close,
                sequence: sequence.checked_add(1).ok_or_else(invalid)?,
                generation: 0,
                table: 0,
                offset: 0,
                data: 0,
                payload: 0,
                confirmed: generation,
            };
            stdin.write_all(&header.encode()).await?;
            stdin.flush().await?;
            let mut bytes = [0; RESPONSE];
            stdout.read_exact(&mut bytes).await?;
            if bytes != response(header, 0, 0, generation, 0) {
                return Err(invalid());
            }
            Ok::<_, io::Error>(())
        };
        let _ = tokio::time::timeout_at(deadline, closing).await;
    }
    kill_owned(&life);
}

struct WorkerTable {
    id: u64,
    files: Option<super::RejectFiles>,
}
struct Worker {
    directory: PathBuf,
    payload_limit: usize,
    copy_limit: usize,
    tables: Vec<WorkerTable>,
    generation: u64,
    confirmed: u64,
    reject_refusal: Option<i32>,
    frame_refusal: Option<i32>,
}
fn reject_open(path: &Path, refusal: &mut Option<i32>) -> io::Result<fs::File> {
    private_new(path).inspect_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            *refusal = Some(error.raw_os_error().unwrap_or(0));
        }
    })
}
fn read_u64(reader: &mut impl Read) -> io::Result<u64> {
    let mut b = [0; 8];
    reader.read_exact(&mut b)?;
    Ok(u64::from_be_bytes(b))
}
fn invalid() -> io::Error {
    io::Error::other("invalid artifact protocol")
}
fn sized_string(reader: &mut impl Read, size: usize, limit: usize) -> io::Result<String> {
    if size > limit {
        return Err(invalid());
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(io::Error::other)
}
fn transfer(reader: &mut impl Read, writer: &mut impl Write, length: u64) -> io::Result<()> {
    let mut left = length;
    let mut buffer = [0; BUFFER];
    while left > 0 {
        let amount = (left.min(BUFFER as u64)) as usize;
        reader.read_exact(&mut buffer[..amount])?;
        writer.write_all(&buffer[..amount])?;
        left -= amount as u64;
    }
    writer.flush()
}
fn generation_path(directory: &Path, generation: u64) -> PathBuf {
    directory.join(format!("report.{generation:020}.json"))
}
impl Worker {
    fn create(reader: &mut impl Read, header: Header) -> io::Result<Self> {
        if header.sequence != 1
            || header.generation != 0
            || header.confirmed != 0
            || header.data != 0
            || header.offset != 0
            || header.payload < 40
        {
            return Err(invalid());
        }
        let payload_limit = usize::try_from(read_u64(reader)?).map_err(io::Error::other)?;
        let copy_limit = usize::try_from(read_u64(reader)?).map_err(io::Error::other)?;
        let tables = usize::try_from(read_u64(reader)?).map_err(io::Error::other)?;
        let base_len = usize::try_from(read_u64(reader)?).map_err(io::Error::other)?;
        let id_len = usize::try_from(read_u64(reader)?).map_err(io::Error::other)?;
        let expected = 40usize
            .checked_add(base_len)
            .and_then(|n| n.checked_add(id_len))
            .and_then(|n| tables.checked_mul(8).and_then(|t| n.checked_add(t)))
            .ok_or_else(invalid)?;
        if payload_limit > u32::MAX as usize
            || expected as u64 != header.payload
            || expected > payload_limit
            || id_len > 62
        {
            return Err(invalid());
        }
        let base = sized_string(reader, base_len, payload_limit)?;
        let id = sized_string(reader, id_len, 62)?;
        if !id.starts_with("run-")
            || (id[4..].split('-').count() != 3
                || id[4..].split('-').any(|part| {
                    part.is_empty()
                        || !part
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                }))
            || base.as_bytes().contains(&0)
        {
            return Err(invalid());
        }
        let mut known = Vec::new();
        known.try_reserve_exact(tables).map_err(io::Error::other)?;
        for _ in 0..tables {
            let id = read_u64(reader)?;
            if known.iter().any(|t: &WorkerTable| t.id == id) {
                return Err(invalid());
            }
            known.push(WorkerTable { id, files: None });
        }
        let base = PathBuf::from(base);
        fs::create_dir_all(&base)?;
        let directory = base.join(id);
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&directory)?;
        sync_directory(&base)?;
        Ok(Self {
            directory,
            payload_limit,
            copy_limit,
            tables: known,
            generation: 0,
            confirmed: 0,
            reject_refusal: None,
            frame_refusal: None,
        })
    }
    fn execute(&mut self, reader: &mut impl Read, header: Header) -> io::Result<u64> {
        self.frame_refusal = None;
        if header.op == Op::Create {
            return Err(invalid());
        }
        if header.payload > self.payload_limit as u64
            || header.data > self.copy_limit as u64
            || header.confirmed > self.generation
        {
            return Err(invalid());
        }
        if !matches!(header.op, Op::Report) && header.generation != 0 {
            return Err(invalid());
        }
        if header.op != Op::Copy && (header.data != 0 || header.offset != 0) {
            return Err(invalid());
        }
        if matches!(header.op, Op::Plan | Op::Report | Op::Copy | Op::Conversion)
            && header.payload == 0
        {
            return Err(invalid());
        }
        if header.op == Op::Report && header.generation <= self.generation {
            return Err(invalid());
        }
        if matches!(header.op, Op::Flush | Op::Close) && (header.payload != 0 || header.offset != 0)
        {
            return Err(invalid());
        }
        if matches!(header.op, Op::Copy | Op::Conversion) {
            if self.copy_limit == 0 {
                return Err(invalid());
            }
            let table = self
                .tables
                .iter()
                .find(|t| t.id == header.table)
                .ok_or_else(invalid)?;
            if header.op == Op::Copy
                && (header.data == 0
                    || header.offset != table.files.as_ref().map_or(0, |f| f.offset))
            {
                return Err(invalid());
            }
        }
        if header.confirmed > self.confirmed {
            if header.confirmed != self.generation {
                return Err(invalid());
            }
            if self.confirmed != 0 {
                fs::remove_file(generation_path(&self.directory, self.confirmed))?;
                sync_directory(&self.directory)?;
            }
            self.confirmed = header.confirmed;
        }
        match header.op {
            Op::Create => return Err(invalid()),
            Op::Plan => {
                let path = self.directory.join("plan.json");
                let file = private_new(&path)?;
                let mut output = io::BufWriter::with_capacity(BUFFER, file);
                transfer(reader, &mut output, header.payload)?;
                output.get_ref().sync_all()?;
                sync_directory(&self.directory)?;
            }
            Op::Report => {
                if header.generation <= self.generation {
                    return Err(invalid());
                }
                let temporary = self.directory.join("report.json.tmp");
                let immutable = generation_path(&self.directory, header.generation);
                let head = self.directory.join("report.head.tmp");
                let mut own_temporary = false;
                let mut own_head = false;
                let result = (|| {
                    let file = private_new(&temporary)?;
                    own_temporary = true;
                    let mut output = io::BufWriter::with_capacity(BUFFER, file);
                    transfer(reader, &mut output, header.payload)?;
                    output.get_ref().sync_all()?;
                    fs::hard_link(&temporary, &immutable)?;
                    fs::remove_file(&temporary)?;
                    own_temporary = false;
                    sync_directory(&self.directory)?;
                    fs::hard_link(&immutable, &head)?;
                    own_head = true;
                    fs::rename(&head, self.directory.join("report.json"))?;
                    own_head = false;
                    sync_directory(&self.directory)
                })();
                if result.is_err() {
                    if own_temporary {
                        let _ = fs::remove_file(&temporary);
                    }
                    if own_head {
                        let _ = fs::remove_file(&head);
                    }
                }
                result?;
                self.generation = header.generation;
            }
            Op::Conversion | Op::Copy => {
                if let Some(errno) = self.reject_refusal {
                    self.frame_refusal = Some(errno);
                    return Err(io::ErrorKind::AlreadyExists.into());
                }
                let table = self
                    .tables
                    .iter_mut()
                    .find(|t| t.id == header.table)
                    .ok_or_else(invalid)?;
                if table.files.is_none() {
                    let id = format!("t_{:016x}", table.id);
                    let data = reject_open(
                        &self.directory.join(format!("{id}.reject.copy")),
                        &mut self.frame_refusal,
                    )?;
                    let metadata = reject_open(
                        &self.directory.join(format!("{id}.reject.jsonl")),
                        &mut self.frame_refusal,
                    )?;
                    sync_directory(&self.directory)?;
                    table.files = Some(super::RejectFiles {
                        data,
                        metadata,
                        offset: 0,
                    });
                }
                let files = table.files.as_mut().expect("opened files");
                if header.op == Op::Copy {
                    if header.offset != files.offset {
                        return Err(invalid());
                    }
                    transfer(reader, &mut files.data, header.data)?;
                    files.data.sync_all()?;
                }
                let mut output = io::BufWriter::with_capacity(BUFFER, &mut files.metadata);
                transfer(reader, &mut output, header.payload)?;
                output.get_ref().sync_all()?;
                drop(output);
                if header.op == Op::Copy {
                    files.offset = files.offset.checked_add(header.data).ok_or_else(invalid)?;
                }
                return Ok(files.offset);
            }
            Op::Flush | Op::Close => {
                if header.payload != 0 {
                    return Err(invalid());
                }
                for table in &mut self.tables {
                    if let Some(files) = &mut table.files {
                        files.data.flush()?;
                        files.data.sync_all()?;
                        files.metadata.flush()?;
                        files.metadata.sync_all()?;
                    }
                }
                sync_directory(&self.directory)?;
            }
        }
        Ok(0)
    }
}
pub fn serve_worker(mut input: impl Read, mut output: impl Write) -> io::Result<()> {
    let mut worker: Option<Worker> = None;
    let mut sequence = 0u64;
    loop {
        let mut bytes = [0; HEADER];
        input.read_exact(&mut bytes)?;
        let header = Header::decode(&bytes)?;
        if header.sequence != sequence.checked_add(1).ok_or_else(invalid)? {
            return Err(invalid());
        }
        sequence = header.sequence;
        let result = if let Some(worker) = worker.as_mut() {
            worker.execute(&mut input, header)
        } else {
            if header.op != Op::Create {
                return Err(invalid());
            }
            Worker::create(&mut input, header).map(|created| {
                worker = Some(created);
                0
            })
        };
        let generation = worker.as_ref().map_or(0, |w| w.generation);
        match result {
            Ok(offset) => {
                output.write_all(&response(header, 0, 0, generation, offset))?;
                output.flush()?;
                if header.op == Op::Close {
                    return Ok(());
                }
            }
            Err(error) => {
                if let Some(errno) = worker
                    .as_mut()
                    .and_then(|worker| worker.frame_refusal.take())
                {
                    // Only exclusive opens before any frame consumption produce this marker.
                    // A complete bounded drain is required before the refusal is definitive.
                    let left = header
                        .data
                        .checked_add(header.payload)
                        .ok_or_else(invalid)?;
                    transfer(&mut input, &mut io::sink(), left)?;
                    worker.as_mut().ok_or_else(invalid)?.reject_refusal = Some(errno);
                    output.write_all(&response(header, 2, errno, generation, 0))?;
                    output.flush()?;
                    continue;
                }
                output.write_all(&response(
                    header,
                    1,
                    error.raw_os_error().unwrap_or(0),
                    generation,
                    0,
                ))?;
                output.flush()?;
                return Err(error);
            }
        }
    }
}
pub fn worker_stdio() -> io::Result<()> {
    serve_worker(io::stdin().lock(), io::stdout().lock())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct CollidingInput {
        bytes: io::Cursor<Vec<u8>>,
        create_end: u64,
        sentinel: PathBuf,
    }
    impl Read for CollidingInput {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            if self.bytes.position() == self.create_end {
                fs::write(&self.sentinel, b"untouched sentinel")?;
            }
            Read::read(&mut self.bytes, bytes)
        }
    }
    #[test]
    fn reject_collision_requires_complete_frame_drain_and_preserves_report_channel() {
        for (op, file, truncated) in [
            (Op::Copy, "t_0000000000000001.reject.copy", false),
            (Op::Copy, "t_0000000000000001.reject.jsonl", false),
            (Op::Conversion, "t_0000000000000001.reject.jsonl", false),
            (Op::Copy, "t_0000000000000001.reject.jsonl", true),
            (Op::Plan, "plan.json", false),
        ] {
            let root = std::env::temp_dir().join(format!(
                "my2pg-collision-frame-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let id = "run-1-2-3";
            let base = root.to_str().unwrap().as_bytes();
            let mut body = Vec::new();
            for n in [65_536, 65_536, 1, base.len() as u64, id.len() as u64] {
                body.extend_from_slice(&n.to_be_bytes());
            }
            body.extend_from_slice(base);
            body.extend_from_slice(id.as_bytes());
            body.extend_from_slice(&1u64.to_be_bytes());
            let create = Header {
                op: Op::Create,
                sequence: 1,
                generation: 0,
                table: 0,
                offset: 0,
                data: 0,
                payload: body.len() as u64,
                confirmed: 0,
            };
            let mut bytes = create.encode().to_vec();
            bytes.extend_from_slice(&body);
            let create_end = bytes.len() as u64;
            let reject = Header {
                op,
                sequence: 2,
                table: if op == Op::Plan { 0 } else { 1 },
                data: if op == Op::Copy { 9 } else { 0 },
                payload: (BUFFER + 17) as u64,
                ..create
            };
            bytes.extend_from_slice(&reject.encode());
            let remaining = reject.data + reject.payload;
            bytes.resize(
                bytes.len() + remaining as usize - usize::from(truncated),
                b'x',
            );
            if !truncated {
                let report = Header {
                    op: Op::Report,
                    sequence: 3,
                    generation: 1,
                    payload: 2,
                    ..create
                };
                bytes.extend_from_slice(&report.encode());
                bytes.extend_from_slice(b"{}");
                bytes.extend_from_slice(
                    &Header {
                        op: Op::Close,
                        sequence: 4,
                        payload: 0,
                        confirmed: 1,
                        ..create
                    }
                    .encode(),
                );
            }
            let sentinel = root.join(id).join(file);
            let mut output = Vec::new();
            let result = serve_worker(
                CollidingInput {
                    bytes: io::Cursor::new(bytes),
                    create_end,
                    sentinel: sentinel.clone(),
                },
                &mut output,
            );
            if truncated {
                assert!(result.is_err());
                assert_eq!(
                    output.len(),
                    RESPONSE,
                    "incomplete drain must not emit refusal"
                );
            } else if op == Op::Plan {
                assert!(result.is_err());
                assert_eq!(output.len(), 2 * RESPONSE);
                assert_eq!(&output[RESPONSE + 10..RESPONSE + 12], &1u16.to_be_bytes());
                assert!(!root.join(id).join("report.json").exists());
            } else {
                result.unwrap();
                assert_eq!(output.len(), 4 * RESPONSE);
                assert_eq!(&output[RESPONSE + 10..RESPONSE + 12], &2u16.to_be_bytes());
                assert_eq!(get(&output[RESPONSE..], 20), 0);
                assert_eq!(get(&output[RESPONSE..], 28), 0);
                assert_eq!(fs::read(root.join(id).join("report.json")).unwrap(), b"{}");
            }
            assert_eq!(fs::read(sentinel).unwrap(), b"untouched sentinel");
            fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn serialized_raw_value_domain_matches_admission_constants() {
        let largest = RawValue::Date {
            year: u16::MAX,
            month: u8::MAX,
            day: u8::MAX,
            hour: u8::MAX,
            minute: u8::MAX,
            second: u8::MAX,
            micros: u32::MAX,
        };
        assert_eq!(count(&StoredValues(&[largest]), false).unwrap() - 3, 142);
        assert_eq!(
            count(&StoredValues(&[RawValue::Bytes(vec![])]), false).unwrap() - 3,
            44
        );
        let data = RawValue::Bytes(vec![0, 255, 17]);
        assert_eq!(count(&StoredValues(&[data]), false).unwrap() - 3, 50);
    }
    #[test]
    fn finite_frames_refuse_reserved_fields_unknown_ops_names_and_sequence() {
        let header = Header {
            op: Op::Create,
            sequence: 1,
            generation: 0,
            table: 0,
            offset: 0,
            data: 0,
            payload: 40,
            confirmed: 0,
        };
        for (at, value) in [(8, 2), (10, 255), (11, 1), (28, 1), (46, 1)] {
            let mut bytes = header.encode();
            bytes[at] = value;
            assert!(Header::decode(&bytes).is_err());
        }
        let mut row = Header {
            op: Op::Copy,
            ..header
        }
        .encode();
        row[30] = b'A';
        assert!(Header::decode(&row).is_err());
        let mut output = Vec::new();
        let frame = Header {
            sequence: 2,
            ..header
        }
        .encode();
        assert!(serve_worker(&frame[..], &mut output).is_err());
        assert!(output.is_empty());
        let mut output = Vec::new();
        let frame = Header {
            payload: 0,
            ..header
        }
        .encode();
        assert!(serve_worker(&frame[..], &mut output).is_err());
        assert_eq!(output.len(), RESPONSE);
        assert_eq!(&output[10..12], &1u16.to_be_bytes());
    }
}
