//! Exact multiset comparison for rows that have already been canonically encoded.
//!
//! Canonicalization is deliberately owned by the verifier's typed-value layer. This
//! module only sorts complete row encodings, so it can preserve duplicates exactly.

use crate::{
    config::{Consistency, ExistingPolicy, MigrationMode, VerificationMode},
    model::{MigrationPlan, TablePlan, TableReport, VerificationReport, VerificationStatus},
    mysql::{self, SourceConnection},
    postgres::{self, TargetConnection},
    verify::canonical,
};
use futures_util::TryStreamExt;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering as AtomicOrdering},
};
use tokio_postgres::IsolationLevel;

const MIN_MEMORY_BYTES: usize = 256 * 1024;
static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContentComparison {
    pub equal: bool,
    pub source_rows: u64,
    pub target_rows: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ContentError {
    #[error("content verification needs at least 256 KiB of memory")]
    MemoryLimit,
    #[error("content verification row exceeds its bounded record workspace")]
    RowTooLarge,
    #[error("content verification row count overflow")]
    CountOverflow,
    #[error("content verification temporary record is invalid")]
    InvalidRecord,
    #[error("content verification temporary storage failed")]
    Storage(#[source] io::Error),
    #[error("content verification temporary storage is unsupported on this platform")]
    UnsupportedPlatform,
}

impl From<io::Error> for ContentError {
    fn from(error: io::Error) -> Self {
        Self::Storage(error)
    }
}

/// Resource limits and run context for one content verification request.
pub struct ContentVerificationOptions {
    pub memory_bytes: usize,
    pub max_row_bytes: usize,
    pub source_snapshot_already_pinned: bool,
    pub append_baseline: Option<AppendContentBaseline>,
}

/// Run exact canonical content verification for the reviewed supported subset.
///
/// `source_snapshot_already_pinned` must reflect the caller's connection state:
/// a single-snapshot migration reuses its preflight snapshot, while a frozen
/// migration starts a fresh read-only snapshot at verification time. The
/// target is scanned once inside a read-only repeatable-read transaction.
pub async fn verify_content(
    source: &mut SourceConnection,
    target: &mut TargetConnection,
    plan: &MigrationPlan,
    reports: &[TableReport],
    mut options: ContentVerificationOptions,
) -> VerificationReport {
    let mut result = verify_content_inner(source, target, plan, reports, &mut options).await;
    if let Some(baseline) = options.append_baseline.take()
        && baseline.cleanup().is_err()
    {
        result.status = VerificationStatus::Error;
        result
            .differences
            .push("append content baseline cleanup failed".into());
    }
    result
}

async fn verify_content_inner(
    source: &mut SourceConnection,
    target: &mut TargetConnection,
    plan: &MigrationPlan,
    reports: &[TableReport],
    options: &mut ContentVerificationOptions,
) -> VerificationReport {
    let mut result = VerificationReport {
        mode: VerificationMode::Content,
        status: VerificationStatus::NotRun,
        tables_checked: 0,
        differences: Vec::new(),
    };
    if plan.verification != VerificationMode::Content {
        result.status = VerificationStatus::Unsupported;
        result
            .differences
            .push("content verifier called for another verification mode".into());
        return result;
    }
    if plan.mode == MigrationMode::SchemaOnly {
        return unsupported(result, "schema_only has no migrated content to verify");
    }
    if plan.on_existing == ExistingPolicy::Append && options.append_baseline.is_none() {
        return unsupported(
            result,
            "append content verification requires an exact pre-run target multiset baseline",
        );
    }
    if plan.on_existing != ExistingPolicy::Append && options.append_baseline.is_some() {
        return unsupported(
            result,
            "append baseline supplied for a non-append migration",
        );
    }
    if plan.consistency == Consistency::SingleSnapshot && !options.source_snapshot_already_pinned {
        return unsupported(
            result,
            "single_snapshot verification requires the migration's original source snapshot",
        );
    }
    if plan.tables.is_empty() {
        return unsupported(
            result,
            "content verification requires at least one selected table",
        );
    }
    let by_id: std::collections::BTreeMap<_, _> =
        reports.iter().map(|report| (&report.id, report)).collect();
    if by_id.len() != reports.len()
        || reports.len() != plan.tables.len()
        || plan
            .tables
            .iter()
            .any(|table| !by_id.contains_key(&table.id))
    {
        return unsupported(
            result,
            "run accounting does not cover each selected table exactly once",
        );
    }
    for table in &plan.tables {
        if table.is_view || !table.engine.eq_ignore_ascii_case("InnoDB") {
            return unsupported(
                result,
                "content verification requires selected InnoDB base tables",
            );
        }
        let columns: Vec<_> = table
            .columns
            .iter()
            .filter(|column| column.copy)
            .cloned()
            .collect();
        if canonical::validate_columns(&columns).is_err() {
            return unsupported(
                result,
                "selected columns include a type or transform outside the canonical subset",
            );
        }
        let report = by_id[&table.id];
        if report.source_name != table.source_name
            || report.target_schema != table.target_schema
            || report.target_name != table.target_name
            || report.status != crate::model::RunStatus::Complete
            || report.accounted_rows() != Some(report.rows_read)
            || report.rows_read != report.committed_rows
            || report.rejected_rows != 0
            || report.unresolved_rows != 0
            || report.indeterminate_rows != 0
        {
            return unsupported(
                result,
                "migration accounting is incomplete or does not match the plan",
            );
        }
    }
    if options.memory_bytes < MIN_MEMORY_BYTES.saturating_mul(2) || options.max_row_bytes == 0 {
        return unsupported(
            result,
            "content verification memory or row limit is below the supported minimum",
        );
    }
    if let Some(baseline) = options.append_baseline.as_ref() {
        let expected: std::collections::BTreeSet<_> =
            plan.tables.iter().map(|table| table.id.as_str()).collect();
        let actual: std::collections::BTreeSet<_> =
            baseline.by_table.keys().map(String::as_str).collect();
        if expected != actual {
            return unsupported(
                result,
                "append baseline table identities do not match the selected plan",
            );
        }
    }
    let per_side_memory = options.memory_bytes / 2;
    if plan.consistency == Consistency::Frozen
        && !options.source_snapshot_already_pinned
        && mysql::start_snapshot(source).await.is_err()
    {
        result.status = VerificationStatus::Error;
        result
            .differences
            .push("could not establish the read-only MySQL verification snapshot".into());
        return result;
    }
    let transaction = target
        .client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await;
    let Ok(transaction) = transaction else {
        result.status = VerificationStatus::Error;
        result
            .differences
            .push("could not establish the read-only PostgreSQL verification snapshot".into());
        return result;
    };
    let mut different = false;
    for table in &plan.tables {
        let report = by_id[&table.id];
        match compare_live_table(
            source,
            &transaction,
            &plan.source_database,
            table,
            per_side_memory,
            options.max_row_bytes,
            options
                .append_baseline
                .as_mut()
                .and_then(|baseline| baseline.take_table(&table.id)),
        )
        .await
        {
            Ok((comparison, migrated_rows)) => {
                result.tables_checked += 1;
                if migrated_rows != report.rows_read
                    || (plan.on_existing != ExistingPolicy::Append
                        && comparison.source_rows != report.committed_rows)
                    || !comparison.equal
                {
                    different = true;
                    result.differences.push(format!(
                        "{}: canonical source and target row multisets differ",
                        table.id
                    ));
                }
            }
            Err(LiveCompareError::Unsupported) => {
                result.status = VerificationStatus::Unsupported;
                result.differences.push(format!(
                    "{}: canonical verification encountered an unsupported value",
                    table.id
                ));
                return result;
            }
            Err(LiveCompareError::Failed) => {
                result.status = VerificationStatus::Error;
                result
                    .differences
                    .push(format!("{}: canonical content scan failed", table.id));
                return result;
            }
        }
    }
    if transaction.commit().await.is_err() {
        result.status = VerificationStatus::Error;
        result
            .differences
            .push("PostgreSQL verification transaction did not complete".into());
        return result;
    }
    result.status = if different {
        VerificationStatus::Different
    } else {
        VerificationStatus::Complete
    };
    result
}

fn unsupported(mut report: VerificationReport, reason: &str) -> VerificationReport {
    report.status = VerificationStatus::Unsupported;
    report.differences.push(reason.into());
    report
}

enum LiveCompareError {
    Unsupported,
    Failed,
}

async fn compare_live_table(
    source: &mut SourceConnection,
    target: &tokio_postgres::Transaction<'_>,
    source_database: &str,
    table: &TablePlan,
    memory_bytes: usize,
    max_row_bytes: usize,
    baseline: Option<SortedMultiset>,
) -> Result<(ContentComparison, u64), LiveCompareError> {
    let columns: Vec<_> = table
        .columns
        .iter()
        .filter(|column| column.copy)
        .cloned()
        .collect();
    let mut source_set =
        MultisetBuilder::new(memory_bytes, "source").map_err(|_| LiveCompareError::Failed)?;
    let mut target_set =
        MultisetBuilder::new(memory_bytes, "target").map_err(|_| LiveCompareError::Failed)?;
    let mut source_stream = mysql::table_stream(source, source_database, table)
        .await
        .map_err(|_| LiveCompareError::Failed)?;
    while let Some(row) = source_stream
        .try_next()
        .await
        .map_err(|_| LiveCompareError::Failed)?
    {
        let raw = mysql::raw_values(row, max_row_bytes).map_err(|_| LiveCompareError::Failed)?;
        let encoded = canonical::mysql_row(&columns, &raw).map_err(|error| match error {
            canonical::CanonicalError::UnsupportedKind(_) => LiveCompareError::Unsupported,
            _ => LiveCompareError::Failed,
        })?;
        if encoded.len() > max_row_bytes {
            return Err(LiveCompareError::Failed);
        }
        source_set
            .push(encoded)
            .map_err(|_| LiveCompareError::Failed)?;
    }
    drop(source_stream);

    let query = target_projection(&columns, table);
    let statement = target
        .prepare(&query)
        .await
        .map_err(|_| LiveCompareError::Failed)?;
    let rows = target
        .query_raw(&statement, std::iter::empty::<&str>())
        .await
        .map_err(|_| LiveCompareError::Failed)?;
    futures_util::pin_mut!(rows);
    while let Some(row) = rows
        .try_next()
        .await
        .map_err(|_| LiveCompareError::Failed)?
    {
        let encoded = canonical_target_row(&row, &columns, max_row_bytes)?;
        target_set
            .push(encoded)
            .map_err(|_| LiveCompareError::Failed)?;
    }
    let source_rows = source_set.finish().map_err(|_| LiveCompareError::Failed)?;
    let migrated_rows = source_rows.row_count;
    let target_rows = target_set.finish().map_err(|_| LiveCompareError::Failed)?;
    let source_rows = if let Some(baseline) = baseline {
        merge_sorted_multisets(source_rows, baseline).map_err(|_| LiveCompareError::Failed)?
    } else {
        source_rows
    };
    compare_sorted_multisets(source_rows, target_rows)
        .map(|comparison| (comparison, migrated_rows))
        .map_err(|_| LiveCompareError::Failed)
}

fn target_projection(columns: &[crate::model::ColumnPlan], table: &TablePlan) -> String {
    let projection = columns
        .iter()
        .map(|column| format!("{}::text", postgres::quote_ident(&column.target_name)))
        .collect::<Vec<_>>();
    format!(
        "SELECT {} FROM {}",
        projection.join(","),
        postgres::qualified(&table.target_schema, &table.target_name)
    )
}

fn canonical_target_row(
    row: &tokio_postgres::Row,
    columns: &[crate::model::ColumnPlan],
    max_row_bytes: usize,
) -> Result<Vec<u8>, LiveCompareError> {
    let mut values = Vec::with_capacity(columns.len());
    for index in 0..columns.len() {
        values.push(
            row.try_get::<_, Option<String>>(index)
                .map_err(|_| LiveCompareError::Failed)?,
        );
    }
    let borrowed = values
        .iter()
        .map(|value| value.as_deref())
        .collect::<Vec<_>>();
    let encoded =
        canonical::postgres_text_row(columns, &borrowed).map_err(|error| match error {
            canonical::CanonicalError::UnsupportedKind(_) => LiveCompareError::Unsupported,
            _ => LiveCompareError::Failed,
        })?;
    if encoded.len() > max_row_bytes {
        return Err(LiveCompareError::Failed);
    }
    Ok(encoded)
}

/// Compare complete canonical row encodings while retaining only bounded runs in RAM.
///
/// Rows are sorted and compared as exact bytes, not row hashes. Repeated encodings
/// remain repeated in each run and in every merge pass.
pub fn compare_multisets<S, T>(
    source: S,
    target: T,
    memory_bytes: usize,
) -> Result<ContentComparison, ContentError>
where
    S: IntoIterator<Item = Vec<u8>>,
    T: IntoIterator<Item = Vec<u8>>,
{
    if memory_bytes < MIN_MEMORY_BYTES {
        return Err(ContentError::MemoryLimit);
    }
    #[cfg(not(unix))]
    {
        let _ = (source, target);
        return Err(ContentError::UnsupportedPlatform);
    }
    #[cfg(unix)]
    {
        let mut source_rows = MultisetBuilder::new(memory_bytes, "source")?;
        let mut target_rows = MultisetBuilder::new(memory_bytes, "target")?;
        for row in source {
            source_rows.push(row)?;
        }
        for row in target {
            target_rows.push(row)?;
        }
        compare_sorted_multisets(source_rows.finish()?, target_rows.finish()?)
    }
}

/// A bounded external sorter that accepts rows incrementally from an async reader.
///
/// It retains at most half of `memory_bytes` in row allocations, with a per-row
/// ceiling of one eighth of the budget. The final file remains private and is
/// removed when the finished multiset is dropped.
pub struct MultisetBuilder {
    #[cfg(unix)]
    temp: TempDirectory,
    #[cfg(unix)]
    runs: RunSet,
    #[cfg(unix)]
    chunk: Vec<Vec<u8>>,
    #[cfg(unix)]
    retained: usize,
    #[cfg(unix)]
    max_row_bytes: usize,
    #[cfg(unix)]
    memory_bytes: usize,
    #[cfg(unix)]
    prefix: String,
    #[cfg(unix)]
    row_count: u64,
}

impl MultisetBuilder {
    pub fn new(memory_bytes: usize, prefix: &str) -> Result<Self, ContentError> {
        if memory_bytes < MIN_MEMORY_BYTES {
            return Err(ContentError::MemoryLimit);
        }
        #[cfg(not(unix))]
        {
            let _ = prefix;
            Err(ContentError::UnsupportedPlatform)
        }
        #[cfg(unix)]
        {
            Ok(Self {
                temp: TempDirectory::new()?,
                runs: RunSet::default(),
                chunk: Vec::new(),
                retained: 0,
                max_row_bytes: memory_bytes / 8,
                memory_bytes,
                prefix: prefix.into(),
                row_count: 0,
            })
        }
    }

    pub fn push(&mut self, row: Vec<u8>) -> Result<(), ContentError> {
        #[cfg(not(unix))]
        {
            let _ = row;
            Err(ContentError::UnsupportedPlatform)
        }
        #[cfg(unix)]
        {
            let row = validate_row_capacity(row, self.max_row_bytes)?;
            let cost = row
                .capacity()
                .saturating_add(std::mem::size_of::<Vec<u8>>());
            if !self.chunk.is_empty() && self.retained.saturating_add(cost) > self.memory_bytes / 2
            {
                self.flush_run()?;
            }
            self.retained = self.retained.saturating_add(cost);
            self.chunk.push(row);
            self.row_count = self
                .row_count
                .checked_add(1)
                .ok_or(ContentError::CountOverflow)?;
            Ok(())
        }
    }

    #[cfg(unix)]
    fn flush_run(&mut self) -> Result<(), ContentError> {
        let path = write_run(
            &mut self.chunk,
            &self.temp.0,
            &self.prefix,
            self.runs.next_path,
        )?;
        self.runs.next_path = self
            .runs
            .next_path
            .checked_add(1)
            .ok_or(ContentError::CountOverflow)?;
        self.runs
            .insert(path, &self.temp.0, &self.prefix, self.max_row_bytes)?;
        self.retained = 0;
        Ok(())
    }

    pub fn finish(mut self) -> Result<SortedMultiset, ContentError> {
        #[cfg(not(unix))]
        {
            Err(ContentError::UnsupportedPlatform)
        }
        #[cfg(unix)]
        {
            if !self.chunk.is_empty() || self.runs.is_empty() {
                self.flush_run()?;
            }
            let path = std::mem::take(&mut self.runs).finish(
                &self.temp.0,
                &self.prefix,
                self.max_row_bytes,
            )?;
            let max_row_bytes = self.max_row_bytes;
            let row_count = self.row_count;
            let temp = std::mem::replace(&mut self.temp, TempDirectory(PathBuf::new()));
            Ok(SortedMultiset {
                temp,
                path,
                max_row_bytes,
                row_count,
            })
        }
    }
}

/// Private, sorted exact row records ready for a merge comparison.
pub struct SortedMultiset {
    #[cfg(unix)]
    temp: TempDirectory,
    #[cfg(unix)]
    path: PathBuf,
    #[cfg(unix)]
    max_row_bytes: usize,
    #[cfg(unix)]
    row_count: u64,
}

impl SortedMultiset {
    #[cfg(unix)]
    fn cleanup(self) -> Result<(), ContentError> {
        self.temp.cleanup()
    }

    #[cfg(not(unix))]
    fn cleanup(self) -> Result<(), ContentError> {
        Err(ContentError::UnsupportedPlatform)
    }
}

/// Exact pre-run content for every selected append target table. Capture this
/// before mutation and keep it alive until `verify_content` consumes the run.
pub struct AppendContentBaseline {
    by_table: std::collections::BTreeMap<String, SortedMultiset>,
}

#[derive(Debug, thiserror::Error)]
pub enum BaselineError {
    #[error("append content baseline is unsupported for this plan")]
    Unsupported,
    #[error("append content baseline capture failed")]
    Failed,
}

impl AppendContentBaseline {
    fn take_table(&mut self, id: &str) -> Option<SortedMultiset> {
        self.by_table.remove(id)
    }

    fn cleanup(self) -> Result<(), ContentError> {
        let mut first_error = None;
        for (_, multiset) in self.by_table {
            if let Err(error) = multiset.cleanup() {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

/// Capture exact target rows before an append run. The returned private spill
/// files are the content baseline; row counts alone are not a content oracle.
pub async fn capture_append_baseline(
    target: &mut TargetConnection,
    plan: &MigrationPlan,
    memory_bytes: usize,
    max_row_bytes: usize,
) -> Result<AppendContentBaseline, BaselineError> {
    if plan.on_existing != ExistingPolicy::Append
        || plan.mode == MigrationMode::SchemaOnly
        || plan.tables.is_empty()
        || memory_bytes < MIN_MEMORY_BYTES
        || max_row_bytes == 0
    {
        return Err(BaselineError::Unsupported);
    }
    for table in &plan.tables {
        if table.is_view || !table.engine.eq_ignore_ascii_case("InnoDB") {
            return Err(BaselineError::Unsupported);
        }
        let columns: Vec<_> = table
            .columns
            .iter()
            .filter(|column| column.copy)
            .cloned()
            .collect();
        canonical::validate_columns(&columns).map_err(|_| BaselineError::Unsupported)?;
    }
    let transaction = target
        .client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await
        .map_err(|_| BaselineError::Failed)?;
    let mut by_table = std::collections::BTreeMap::new();
    for table in &plan.tables {
        let rows = capture_target_table(&transaction, table, memory_bytes, max_row_bytes).await?;
        if by_table.insert(table.id.clone(), rows).is_some() {
            return Err(BaselineError::Unsupported);
        }
    }
    transaction
        .commit()
        .await
        .map_err(|_| BaselineError::Failed)?;
    Ok(AppendContentBaseline { by_table })
}

async fn capture_target_table(
    transaction: &tokio_postgres::Transaction<'_>,
    table: &TablePlan,
    memory_bytes: usize,
    max_row_bytes: usize,
) -> Result<SortedMultiset, BaselineError> {
    let columns: Vec<_> = table
        .columns
        .iter()
        .filter(|column| column.copy)
        .cloned()
        .collect();
    let mut set =
        MultisetBuilder::new(memory_bytes, "baseline").map_err(|_| BaselineError::Failed)?;
    let query = target_projection(&columns, table);
    let statement = transaction
        .prepare(&query)
        .await
        .map_err(|_| BaselineError::Failed)?;
    let rows = transaction
        .query_raw(&statement, std::iter::empty::<&str>())
        .await
        .map_err(|_| BaselineError::Failed)?;
    futures_util::pin_mut!(rows);
    while let Some(row) = rows.try_next().await.map_err(|_| BaselineError::Failed)? {
        let encoded = canonical_target_row(&row, &columns, max_row_bytes)
            .map_err(|_| BaselineError::Failed)?;
        set.push(encoded).map_err(|_| BaselineError::Failed)?;
    }
    set.finish().map_err(|_| BaselineError::Failed)
}

pub fn compare_sorted_multisets(
    source: SortedMultiset,
    target: SortedMultiset,
) -> Result<ContentComparison, ContentError> {
    #[cfg(not(unix))]
    {
        let _ = (source, target);
        Err(ContentError::UnsupportedPlatform)
    }
    #[cfg(unix)]
    {
        let max_row_bytes = source.max_row_bytes.min(target.max_row_bytes);
        let compared = compare_sorted(&source.path, &target.path, max_row_bytes);
        source.cleanup()?;
        target.cleanup()?;
        compared
    }
}

fn merge_sorted_multisets(
    left: SortedMultiset,
    right: SortedMultiset,
) -> Result<SortedMultiset, ContentError> {
    #[cfg(not(unix))]
    {
        let _ = (left, right);
        Err(ContentError::UnsupportedPlatform)
    }
    #[cfg(unix)]
    {
        let max_row_bytes = left.max_row_bytes.min(right.max_row_bytes);
        let row_count = left
            .row_count
            .checked_add(right.row_count)
            .ok_or(ContentError::CountOverflow)?;
        let temp = TempDirectory::new()?;
        let path = temp.0.join("union-run");
        merge_pair(&left.path, &right.path, &path, max_row_bytes)?;
        left.cleanup()?;
        right.cleanup()?;
        Ok(SortedMultiset {
            temp,
            path,
            max_row_bytes,
            row_count,
        })
    }
}

fn validate_row_capacity(row: Vec<u8>, max_row_bytes: usize) -> Result<Vec<u8>, ContentError> {
    if row.len() > max_row_bytes || row.capacity() > max_row_bytes {
        return Err(ContentError::RowTooLarge);
    }
    Ok(row)
}

#[cfg(unix)]
fn write_run(
    rows: &mut Vec<Vec<u8>>,
    dir: &Path,
    prefix: &str,
    index: usize,
) -> Result<PathBuf, ContentError> {
    rows.sort_unstable();
    let path = dir.join(format!("{prefix}-run-{index}"));
    let mut writer = BufWriter::new(create_private_file(&path)?);
    for row in rows.drain(..) {
        write_record(&mut writer, &row)?;
    }
    writer.flush()?;
    Ok(path)
}

#[cfg(unix)]
#[derive(Default)]
struct RunSet {
    // Binary-carry levels hold at most one run each, bounding path metadata by
    // O(log(number of input chunks)); every merge opens exactly two runs.
    levels: Vec<Option<PathBuf>>,
    next_path: usize,
}

#[cfg(unix)]
impl RunSet {
    fn is_empty(&self) -> bool {
        self.levels.iter().all(Option::is_none)
    }

    fn insert(
        &mut self,
        mut run: PathBuf,
        dir: &Path,
        prefix: &str,
        max_row_bytes: usize,
    ) -> Result<(), ContentError> {
        for level in 0.. {
            if level == self.levels.len() {
                self.levels.push(None);
            }
            if let Some(previous) = self.levels[level].take() {
                let merged = dir.join(format!("{prefix}-merge-{}", self.next_path));
                self.next_path = self
                    .next_path
                    .checked_add(1)
                    .ok_or(ContentError::CountOverflow)?;
                merge_pair(&previous, &run, &merged, max_row_bytes)?;
                fs::remove_file(previous)?;
                fs::remove_file(run)?;
                run = merged;
            } else {
                self.levels[level] = Some(run);
                return Ok(());
            }
        }
        unreachable!()
    }

    fn finish(
        self,
        dir: &Path,
        prefix: &str,
        max_row_bytes: usize,
    ) -> Result<PathBuf, ContentError> {
        let Self {
            levels,
            mut next_path,
        } = self;
        let mut runs = levels.into_iter().flatten();
        let mut result = runs.next().expect("each side has an empty run");
        for next in runs {
            let merged = dir.join(format!("{prefix}-merge-{next_path}"));
            next_path = next_path
                .checked_add(1)
                .ok_or(ContentError::CountOverflow)?;
            merge_pair(&result, &next, &merged, max_row_bytes)?;
            fs::remove_file(result)?;
            fs::remove_file(next)?;
            result = merged;
        }
        Ok(result)
    }
}

#[cfg(unix)]
fn merge_pair(
    left: &Path,
    right: &Path,
    output: &Path,
    max_row_bytes: usize,
) -> Result<(), ContentError> {
    let mut left_reader = BufReader::new(File::open(left)?);
    let mut right_reader = BufReader::new(File::open(right)?);
    let mut left_row = read_record(&mut left_reader, max_row_bytes)?;
    let mut right_row = read_record(&mut right_reader, max_row_bytes)?;
    let mut writer = BufWriter::new(create_private_file(output)?);
    loop {
        match (&left_row, &right_row) {
            (Some(left), Some(right)) => {
                if left.len() > max_row_bytes || right.len() > max_row_bytes {
                    return Err(ContentError::RowTooLarge);
                }
                if left <= right {
                    write_record(&mut writer, left)?;
                    left_row = read_record(&mut left_reader, max_row_bytes)?;
                } else {
                    write_record(&mut writer, right)?;
                    right_row = read_record(&mut right_reader, max_row_bytes)?;
                }
            }
            (Some(left), None) => {
                write_record(&mut writer, left)?;
                left_row = read_record(&mut left_reader, max_row_bytes)?;
            }
            (None, Some(right)) => {
                write_record(&mut writer, right)?;
                right_row = read_record(&mut right_reader, max_row_bytes)?;
            }
            (None, None) => break,
        }
    }
    writer.flush()?;
    Ok(())
}

#[cfg(unix)]
fn compare_sorted(
    source: &Path,
    target: &Path,
    max_row_bytes: usize,
) -> Result<ContentComparison, ContentError> {
    let mut source = BufReader::new(File::open(source)?);
    let mut target = BufReader::new(File::open(target)?);
    let mut equal = true;
    let mut source_rows = 0u64;
    let mut target_rows = 0u64;
    loop {
        let source_row = read_record(&mut source, max_row_bytes)?;
        let target_row = read_record(&mut target, max_row_bytes)?;
        if source_row.is_some() {
            source_rows = source_rows
                .checked_add(1)
                .ok_or(ContentError::CountOverflow)?;
        }
        if target_row.is_some() {
            target_rows = target_rows
                .checked_add(1)
                .ok_or(ContentError::CountOverflow)?;
        }
        equal &= match (&source_row, &target_row) {
            (Some(left), Some(right)) => left == right,
            (None, None) => break,
            _ => false,
        };
    }
    Ok(ContentComparison {
        equal,
        source_rows,
        target_rows,
    })
}

#[cfg(unix)]
fn create_private_file(path: &Path) -> Result<File, ContentError> {
    use std::os::unix::fs::OpenOptionsExt;

    Ok(OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?)
}

#[cfg(unix)]
struct TempDirectory(PathBuf);

#[cfg(unix)]
impl TempDirectory {
    fn new() -> Result<Self, ContentError> {
        use std::os::unix::fs::DirBuilderExt;

        let parent = std::env::temp_dir();
        for _ in 0..32 {
            let suffix = NEXT_DIR.fetch_add(1, AtomicOrdering::Relaxed);
            let path = parent.join(format!("my2pg-content-{}-{suffix}", std::process::id()));
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            match builder.create(&path) {
                Ok(()) => {
                    return Ok(Self(path));
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "temporary directory collisions",
        )
        .into())
    }

    fn cleanup(mut self) -> Result<(), ContentError> {
        fs::remove_dir_all(&self.0)?;
        self.0.clear();
        Ok(())
    }
}

#[cfg(unix)]
impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
fn write_record(writer: &mut impl Write, row: &[u8]) -> Result<(), ContentError> {
    let len = u64::try_from(row.len()).map_err(|_| ContentError::RowTooLarge)?;
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(row)?;
    Ok(())
}

#[cfg(unix)]
fn read_record(
    reader: &mut impl Read,
    max_row_bytes: usize,
) -> Result<Option<Vec<u8>>, ContentError> {
    let mut len = [0u8; 8];
    match reader.read(&mut len[..1])? {
        0 => return Ok(None),
        1 => reader.read_exact(&mut len[1..])?,
        _ => unreachable!(),
    }
    let len = usize::try_from(u64::from_be_bytes(len)).map_err(|_| ContentError::InvalidRecord)?;
    if len > max_row_bytes {
        return Err(ContentError::InvalidRecord);
    }
    let mut row = vec![0; len];
    reader.read_exact(&mut row)?;
    Ok(Some(row))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(values: &[&[u8]]) -> Vec<Vec<u8>> {
        values.iter().map(|value| value.to_vec()).collect()
    }

    #[test]
    fn exact_multiset_comparison_preserves_duplicates_and_detects_equal_count_corruption() {
        let same = compare_multisets(
            rows(&[b"a", b"b", b"b"]),
            rows(&[b"b", b"a", b"b"]),
            MIN_MEMORY_BYTES,
        )
        .unwrap();
        assert_eq!(
            same,
            ContentComparison {
                equal: true,
                source_rows: 3,
                target_rows: 3
            }
        );

        let duplicate_changed = compare_multisets(
            rows(&[b"a", b"b", b"b"]),
            rows(&[b"a", b"a", b"b"]),
            MIN_MEMORY_BYTES,
        )
        .unwrap();
        assert_eq!(duplicate_changed.source_rows, duplicate_changed.target_rows);
        assert!(!duplicate_changed.equal);

        let value_changed = compare_multisets(
            rows(&[b"id=1,value=old", b"id=2,value=x"]),
            rows(&[b"id=1,value=new", b"id=2,value=x"]),
            MIN_MEMORY_BYTES,
        )
        .unwrap();
        assert_eq!(value_changed.source_rows, value_changed.target_rows);
        assert!(!value_changed.equal);
    }

    #[test]
    fn empty_streams_and_external_merge_runs_are_exact() {
        let empty = compare_multisets(Vec::new(), Vec::new(), MIN_MEMORY_BYTES).unwrap();
        assert_eq!(
            empty,
            ContentComparison {
                equal: true,
                source_rows: 0,
                target_rows: 0
            }
        );

        let rows: Vec<_> = (0..9000)
            .map(|value| format!("row-{value:05}").into_bytes())
            .collect();
        let mut reversed = rows.clone();
        reversed.reverse();
        let compared = compare_multisets(rows, reversed, MIN_MEMORY_BYTES).unwrap();
        assert!(compared.equal);
        assert_eq!((compared.source_rows, compared.target_rows), (9000, 9000));
    }

    #[test]
    fn resource_limits_are_explicit() {
        assert!(matches!(
            compare_multisets(Vec::new(), Vec::new(), MIN_MEMORY_BYTES - 1),
            Err(ContentError::MemoryLimit)
        ));
        assert!(matches!(
            compare_multisets(
                vec![vec![0; MIN_MEMORY_BYTES / 8 + 1]],
                Vec::new(),
                MIN_MEMORY_BYTES
            ),
            Err(ContentError::RowTooLarge)
        ));
    }

    #[test]
    fn run_bookkeeping_stays_logarithmic_as_runs_are_inserted() {
        let temp = TempDirectory::new().unwrap();
        let mut runs = RunSet::default();
        for index in 0usize..256 {
            let mut row = vec![format!("row-{index:04}").into_bytes()];
            let path = write_run(&mut row, &temp.0, "levels", runs.next_path).unwrap();
            runs.next_path += 1;
            runs.insert(path, &temp.0, "levels", 4096).unwrap();
            let inputs = index + 1;
            let max_levels = usize::BITS - inputs.leading_zeros();
            assert!(runs.levels.len() <= max_levels as usize);
        }
        let sorted = runs.finish(&temp.0, "levels", 4096).unwrap();
        let mut reader = BufReader::new(File::open(sorted).unwrap());
        let mut count = 0;
        while read_record(&mut reader, 4096).unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, 256);
        temp.cleanup().unwrap();
    }

    #[test]
    fn rejects_oversized_framed_record_before_allocating_it() {
        let length = u64::MAX.to_be_bytes();
        let mut reader = io::Cursor::new(length);
        assert!(matches!(
            read_record(&mut reader, 128),
            Err(ContentError::InvalidRecord)
        ));
    }

    #[test]
    fn rejects_small_rows_with_oversized_allocations() {
        let mut row = Vec::with_capacity(MIN_MEMORY_BYTES);
        row.push(b'x');
        assert!(row.capacity() > MIN_MEMORY_BYTES / 8);
        assert!(matches!(
            validate_row_capacity(row, MIN_MEMORY_BYTES / 8),
            Err(ContentError::RowTooLarge)
        ));
    }

    #[test]
    fn append_union_keeps_baseline_and_migrated_duplicate_multiplicity_exact() {
        let mut baseline = MultisetBuilder::new(MIN_MEMORY_BYTES, "baseline-test").unwrap();
        baseline.push(b"existing".to_vec()).unwrap();
        baseline.push(b"existing".to_vec()).unwrap();
        let baseline = baseline.finish().unwrap();

        let mut migrated = MultisetBuilder::new(MIN_MEMORY_BYTES, "migrated-test").unwrap();
        migrated.push(b"new".to_vec()).unwrap();
        let expected = merge_sorted_multisets(baseline, migrated.finish().unwrap()).unwrap();

        let mut actual = MultisetBuilder::new(MIN_MEMORY_BYTES, "actual-test").unwrap();
        actual.push(b"existing".to_vec()).unwrap();
        actual.push(b"new".to_vec()).unwrap();
        actual.push(b"existing".to_vec()).unwrap();
        assert!(
            compare_sorted_multisets(expected, actual.finish().unwrap())
                .unwrap()
                .equal
        );

        let mut baseline = MultisetBuilder::new(MIN_MEMORY_BYTES, "baseline-test").unwrap();
        baseline.push(b"existing".to_vec()).unwrap();
        baseline.push(b"existing".to_vec()).unwrap();
        let expected = merge_sorted_multisets(baseline.finish().unwrap(), {
            let mut migrated = MultisetBuilder::new(MIN_MEMORY_BYTES, "migrated-test").unwrap();
            migrated.push(b"new".to_vec()).unwrap();
            migrated.finish().unwrap()
        })
        .unwrap();
        let mut corrupted = MultisetBuilder::new(MIN_MEMORY_BYTES, "corrupted-test").unwrap();
        corrupted.push(b"existing".to_vec()).unwrap();
        corrupted.push(b"new".to_vec()).unwrap();
        corrupted.push(b"new".to_vec()).unwrap();
        let result = compare_sorted_multisets(expected, corrupted.finish().unwrap()).unwrap();
        assert_eq!(result.source_rows, result.target_rows);
        assert!(!result.equal);
    }
}
