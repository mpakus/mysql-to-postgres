//! Private run evidence and console output; every write failure reaches the caller.
pub mod artifact_io;
mod console;
pub use console::{Console, redact, visible};

use crate::model::{Diagnostic, MigrationPlan, RawValue, RowLocator, RunReport};
use serde::{Serialize, Serializer, ser::SerializeSeq};
use std::{
    collections::BTreeMap,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

pub struct RunArtifacts {
    pub run_id: String,
    pub directory: PathBuf,
    rejects: Mutex<BTreeMap<String, RejectFiles>>,
    reject_failed: AtomicBool,
}

struct RejectFiles {
    data: File,
    metadata: File,
    offset: u64,
}

impl RunArtifacts {
    pub fn create(base: &Path) -> io::Result<Self> {
        fs::create_dir_all(base)?;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..10 {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(io::Error::other)?
                .as_nanos();
            let run_id = format!(
                "run-{nanos:x}-{:x}-{:x}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let directory = base.join(&run_id);
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&directory) {
                Ok(()) => {
                    sync_directory(base)?;
                    return Ok(Self {
                        run_id,
                        directory,
                        rejects: Mutex::new(BTreeMap::new()),
                        reject_failed: AtomicBool::new(false),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "cannot allocate an exclusive run directory",
        ))
    }

    pub fn write_plan(&self, plan: &MigrationPlan) -> io::Result<()> {
        let path = self.directory.join("plan.json");
        write_document(private_new(&path)?, plan)?;
        sync_directory(&self.directory)
    }

    /// A report replaces its prior snapshot atomically only after durable writing.
    pub fn write_report(&self, report: &RunReport) -> io::Result<()> {
        let path = self.directory.join("report.json");
        let temporary = self.directory.join("report.json.tmp");
        let mut created = false;
        let result = (|| {
            let file = private_new(&temporary)?;
            created = true;
            write_document(file, report)?;
            fs::rename(&temporary, &path)?;
            sync_directory(&self.directory)
        })();
        if result.is_err() && created {
            let _ = fs::remove_file(temporary);
        }
        result
    }

    pub fn reject_conversion(
        &self,
        table_id: &str,
        locator: &RowLocator,
        values: &[RawValue],
        reason: &Diagnostic,
    ) -> io::Result<()> {
        #[derive(Serialize)]
        struct Record<'a> {
            version: u32,
            kind: &'static str,
            table_id: &'a str,
            locator: &'a RowLocator,
            values: StoredValues<'a>,
            reason: &'a Diagnostic,
        }
        let record = Record {
            version: 1,
            kind: "conversion",
            table_id,
            locator,
            values: StoredValues(values),
            reason,
        };
        self.with_reject(table_id, |files| write_record(&mut files.metadata, &record))
    }

    pub fn reject_copy(
        &self,
        table_id: &str,
        locator: &RowLocator,
        row: &[u8],
        reason: &Diagnostic,
    ) -> io::Result<()> {
        #[derive(Serialize)]
        struct Record<'a> {
            version: u32,
            kind: &'static str,
            encoding: &'static str,
            table_id: &'a str,
            locator: &'a RowLocator,
            offset: u64,
            length: u64,
            reason: &'a Diagnostic,
        }
        self.with_reject(table_id, |files| {
            let length = u64::try_from(row.len()).map_err(io::Error::other)?;
            let next = files
                .offset
                .checked_add(length)
                .ok_or_else(|| io::Error::other("reject offset overflow"))?;
            files.data.write_all(row)?;
            files.data.flush()?;
            files.data.sync_all()?;
            let record = Record {
                version: 1,
                kind: "copy",
                encoding: "postgresql_copy_text",
                table_id,
                locator,
                offset: files.offset,
                length,
                reason,
            };
            write_record(&mut files.metadata, &record)?;
            files.offset = next;
            Ok(())
        })
    }

    fn with_reject(
        &self,
        table_id: &str,
        operation: impl FnOnce(&mut RejectFiles) -> io::Result<()>,
    ) -> io::Result<()> {
        if self.reject_failed.load(Ordering::Acquire) {
            return Err(io::Error::other(
                "reject artifact previously failed; stop the run",
            ));
        }
        let result = (|| {
            if table_id.len() != 18
                || !table_id.starts_with("t_")
                || !table_id[2..]
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "reject table ID must be generated t_<16 lowercase hex>",
                ));
            }
            let mut rejects = self
                .rejects
                .lock()
                .map_err(|_| io::Error::other("reject writer failed"))?;
            if !rejects.contains_key(table_id) {
                let data = private_new(&self.directory.join(format!("{table_id}.reject.copy")))?;
                let metadata =
                    private_new(&self.directory.join(format!("{table_id}.reject.jsonl")))?;
                sync_directory(&self.directory)?;
                rejects.insert(
                    table_id.to_owned(),
                    RejectFiles {
                        data,
                        metadata,
                        offset: 0,
                    },
                );
            }
            operation(rejects.get_mut(table_id).expect("inserted reject files"))
        })();
        if result.is_err() {
            self.reject_failed.store(true, Ordering::Release);
        }
        result
    }

    pub fn flush(&self) -> io::Result<()> {
        if self.reject_failed.load(Ordering::Acquire) {
            return Err(io::Error::other(
                "reject artifact previously failed; stop the run",
            ));
        }
        let mut rejects = self
            .rejects
            .lock()
            .map_err(|_| io::Error::other("reject writer failed"))?;
        for files in rejects.values_mut() {
            files.data.flush()?;
            files.data.sync_all()?;
            files.metadata.flush()?;
            files.metadata.sync_all()?;
        }
        sync_directory(&self.directory)
    }
}

fn private_new(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}
fn write_document(file: File, document: &impl Serialize) -> io::Result<()> {
    let mut writer = BufWriter::with_capacity(16 * 1024, file);
    serde_json::to_writer_pretty(&mut writer, document)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    writer.get_ref().sync_all()
}
fn write_record(file: &mut File, record: &impl Serialize) -> io::Result<()> {
    // One temporary fixed buffer under the reject mutex, rather than a retained
    // per-table allocation or a syscall for every two hex characters.
    {
        let mut writer = BufWriter::with_capacity(16 * 1024, &mut *file);
        serde_json::to_writer(&mut writer, record)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
    }
    file.sync_all()
}
fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

struct Hex<'a>(&'a [u8]);
impl fmt::Display for Hex<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}
impl Serialize for Hex<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

struct StoredValues<'a>(&'a [RawValue]);
impl Serialize for StoredValues<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        enum StoredValue<'a> {
            Bytes {
                encoding: &'static str,
                value: Hex<'a>,
            },
            Float {
                encoding: &'static str,
                bits: u32,
            },
            Double {
                encoding: &'static str,
                bits: u64,
            },
            Typed {
                value: &'a RawValue,
            },
        }
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for value in self.0 {
            let stored = match value {
                RawValue::Bytes(bytes) => StoredValue::Bytes {
                    encoding: "hex",
                    value: Hex(bytes),
                },
                RawValue::Float(number) => StoredValue::Float {
                    encoding: "ieee754_bits",
                    bits: number.to_bits(),
                },
                RawValue::Double(number) => StoredValue::Double {
                    encoding: "ieee754_bits",
                    bits: number.to_bits(),
                },
                value => StoredValue::Typed { value },
            };
            sequence.serialize_element(&stored)?;
        }
        sequence.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Severity;

    fn artifacts() -> (PathBuf, RunArtifacts) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let base = std::env::temp_dir().join(format!(
            "my2pg-artifacts-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let artifacts = RunArtifacts::create(&base).unwrap();
        (base, artifacts)
    }
    fn reason() -> Diagnostic {
        Diagnostic {
            code: "ROW_REJECTED".into(),
            stage: "convert".into(),
            object: None,
            severity: Severity::Error,
            message: "row violates the resolved column policy".into(),
        }
    }
    fn locator(ordinal: u64) -> RowLocator {
        RowLocator { ordinal, key: None }
    }
    const TABLE: &str = "t_0123456789abcdef";

    #[test]
    fn private_exclusive_runs_and_exact_copy_offsets() {
        let (base, first) = artifacts();
        let second = RunArtifacts::create(&base).unwrap();
        assert_ne!(first.run_id, second.run_id);
        let a = b"1\t\\x0001ff\n";
        let b = b"2\t\\N\n";
        first.reject_copy(TABLE, &locator(1), a, &reason()).unwrap();
        first.reject_copy(TABLE, &locator(2), b, &reason()).unwrap();
        first.flush().unwrap();
        assert_eq!(
            fs::read(first.directory.join(format!("{TABLE}.reject.copy"))).unwrap(),
            [a.as_slice(), b.as_slice()].concat()
        );
        let text =
            fs::read_to_string(first.directory.join(format!("{TABLE}.reject.jsonl"))).unwrap();
        let records: Vec<serde_json::Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records[0]["offset"], 0);
        assert_eq!(records[0]["length"], a.len());
        assert_eq!(records[1]["offset"], a.len());
        assert_eq!(records[1]["length"], b.len());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&first.directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(first.directory.join(format!("{TABLE}.reject.copy")))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn raw_values_preserve_every_byte_unsigned_max_zero_date_and_float_bits() {
        let (base, run) = artifacts();
        let bits = 0x7ff8000000000001;
        let values = [
            RawValue::Bytes((0..=255).collect()),
            RawValue::UInt(u64::MAX),
            RawValue::Double(f64::from_bits(bits)),
            RawValue::Float(-0.0),
            RawValue::Date {
                year: 0,
                month: 0,
                day: 0,
                hour: 0,
                minute: 0,
                second: 0,
                micros: 0,
            },
            RawValue::Time {
                negative: true,
                days: 34,
                hour: 22,
                minute: 59,
                second: 59,
                micros: 999999,
            },
            RawValue::Null,
        ];
        run.reject_conversion(TABLE, &locator(7), &values, &reason())
            .unwrap();
        let record: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(run.directory.join(format!("{TABLE}.reject.jsonl"))).unwrap(),
        )
        .unwrap();
        assert_eq!(record["values"][0]["encoding"], "hex");
        let hex = record["values"][0]["value"].as_str().unwrap();
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        assert_eq!(bytes, (0..=255).collect::<Vec<u8>>());
        assert_eq!(
            record["values"][1]["value"]["value"].as_u64(),
            Some(u64::MAX)
        );
        assert_eq!(record["values"][2]["bits"].as_u64(), Some(bits));
        assert_eq!(
            record["values"][3]["bits"].as_u64(),
            Some((-0.0_f32).to_bits().into())
        );
        assert_eq!(record["values"][4]["value"]["value"]["year"], 0);
        assert_eq!(record["values"][5]["value"]["value"]["negative"], true);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn invalid_ids_and_real_filesystem_faults_stop_the_writer() {
        let (base, run) = artifacts();
        assert!(
            run.reject_copy("../../outside", &locator(1), b"row\n", &reason())
                .is_err()
        );
        assert!(run.flush().is_err());
        fs::remove_dir_all(&base).unwrap();
        let (base, run) = artifacts();
        fs::create_dir(run.directory.join(format!("{TABLE}.reject.copy"))).unwrap();
        assert!(
            run.reject_copy(TABLE, &locator(1), b"row\n", &reason())
                .is_err()
        );
        assert!(
            run.reject_conversion(TABLE, &locator(2), &[RawValue::Null], &reason())
                .is_err()
        );
        assert!(run.flush().is_err());
        let file = base.join("not-a-directory");
        fs::write(&file, b"x").unwrap();
        assert!(RunArtifacts::create(&file).is_err());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn plan_is_exclusive_and_report_replacement_preserves_prior_evidence_on_failure() {
        let (base, run) = artifacts();
        let plan: MigrationPlan =
            serde_json::from_str(include_str!("../../tests/contracts/plan.json")).unwrap();
        run.write_plan(&plan).unwrap();
        assert!(run.write_plan(&plan).is_err());
        let mut report: RunReport =
            serde_json::from_str(include_str!("../../tests/contracts/report.json")).unwrap();
        report.run_id = run.run_id.clone();
        run.write_report(&report).unwrap();
        let prior = fs::read(run.directory.join("report.json")).unwrap();
        fs::write(
            run.directory.join("report.json.tmp"),
            b"preexisting evidence",
        )
        .unwrap();
        report.elapsed_millis += 1;
        assert!(run.write_report(&report).is_err());
        assert_eq!(fs::read(run.directory.join("report.json")).unwrap(), prior);
        assert_eq!(
            fs::read(run.directory.join("report.json.tmp")).unwrap(),
            b"preexisting evidence"
        );
        fs::remove_file(run.directory.join("report.json.tmp")).unwrap();
        run.write_report(&report).unwrap();
        let stored: RunReport =
            serde_json::from_slice(&fs::read(run.directory.join("report.json")).unwrap()).unwrap();
        assert_eq!(stored.elapsed_millis, report.elapsed_millis);
        assert!(!run.directory.join("report.json.tmp").exists());
        fs::remove_dir_all(base).unwrap();
    }
}
