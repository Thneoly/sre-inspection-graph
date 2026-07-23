//! Parquet archive backend — append-only Fact history, partitioned by `(date, source)`.
//!
//! Complements `SqliteStorage`: SQLite holds the **latest** materialized state
//! (`topology_nodes` / `topology_edges` + registries); Parquet holds the **full
//! historical Fact stream** for trend / audit queries. Reuses `engine_core`'s
//! canonical Arrow path: `FactBatch::to_record_batch()` → `RecordBatch` →
//! `parquet::arrow::ArrowWriter`.
//!
//! ## Layout
//!
//! ```text
//! {root}/
//! └── dt=2026-07-23/
//!     ├── k8s-1780000000.parquet         # one file per archived (date, source) batch
//!     ├── k8s-1780000120.parquet
//!     └── prometheus-1780000000.parquet
//! ```
//!
//! Append-only / immutable files (data-lake style): each `archive_batch` call
//! writes one file per `(date, source)` group present in the batch. Readers union
//! files across time. This avoids Parquet's awkward in-place append semantics.
//!
//! ## Date partitioning
//!
//! The date (`dt=YYYY-MM-DD`) is derived from each Fact's own `timestamp` (UTC),
//! so a batch spanning midnight lands in two partitions — correct data-lake behavior.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use arrow::array::{Array, StringArray, UInt64Array};
use arrow::record_batch::RecordBatch;
use engine_core::{fact_schema, Fact, FactBatch};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ArrowWriter;

use crate::{Storage, StorageError};

/// Parquet archive — append-only Fact history partitioned by `(date, source)`.
///
/// Cheap to construct; `open()` is idempotent (creates the root dir if absent).
#[derive(Debug, Clone)]
pub struct ParquetStorage {
    /// Archive root directory.
    root: PathBuf,
}

impl ParquetStorage {
    /// Open (or create) an archive rooted at `root`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// Archive root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Archive a batch: one Parquet file per `(date, source)` group in the batch.
    ///
    /// Empty batch / empty groups write nothing. Returns the paths written
    /// (empty vec for an empty batch).
    pub fn archive_batch(&self, batch: &FactBatch) -> Result<Vec<PathBuf>, StorageError> {
        if batch.is_empty() {
            return Ok(Vec::new());
        }
        // Group facts by (date, source) — each group is one partition file.
        let mut groups: BTreeMap<(String, String), Vec<&Fact>> = BTreeMap::new();
        for f in batch.as_slice() {
            let date = utc_date_string(f.timestamp);
            groups
                .entry((date, f.source.clone()))
                .or_default()
                .push(f);
        }

        let mut written = Vec::with_capacity(groups.len());
        for ((date, source), facts) in groups {
            let path = self.partition_file(&date, &source, facts.iter().map(|f| f.timestamp).max());
            self.write_group(&path, &facts)?;
            written.push(path);
        }
        Ok(written)
    }

    /// Read all Facts archived for a given `(date, source)` partition.
    pub fn read_partition(
        &self,
        date: &str,
        source: &str,
    ) -> Result<Vec<Fact>, StorageError> {
        let dir = self.partition_dir(date);
        let prefix = format!("{source}-");
        let mut out = Vec::new();
        if !dir.exists() {
            return Ok(out);
        }
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(&prefix) && name.ends_with(".parquet") {
                out.extend(read_parquet_file(&entry.path())?);
            }
        }
        Ok(out)
    }

    /// Read **all** archived Facts across every partition (recursive). Order is
    /// directory-then-file traversal order (not chronological); callers sorting
    /// for display should sort by `timestamp` themselves.
    pub fn read_all(&self) -> Result<Vec<Fact>, StorageError> {
        let mut out = Vec::new();
        if !self.root.exists() {
            return Ok(out);
        }
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                for f in std::fs::read_dir(&path)? {
                    let f = f?;
                    if f.path().extension().and_then(|e| e.to_str()) == Some("parquet") {
                        out.extend(read_parquet_file(&f.path())?);
                    }
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some("parquet") {
                out.extend(read_parquet_file(&path)?);
            }
        }
        Ok(out)
    }

    fn partition_dir(&self, date: &str) -> PathBuf {
        self.root.join(format!("dt={date}"))
    }

    fn partition_file(
        &self,
        date: &str,
        source: &str,
        max_ts: Option<u64>,
    ) -> PathBuf {
        let ts = max_ts.unwrap_or(0);
        self.partition_dir(date)
            .join(format!("{source}-{ts:020}.parquet"))
    }

    fn write_group(&self, path: &Path, facts: &[&Fact]) -> Result<(), StorageError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Build a FactBatch view of this group, then reuse the canonical Arrow path.
        let group_batch = FactBatch::from_vec(facts.iter().map(|f| (*f).clone()).collect());
        let record = group_batch
            .to_record_batch()
            .map_err(|e| StorageError::Parquet(e.to_string()))?;
        let file = File::create(path)?;
        let mut writer =
            ArrowWriter::try_new(file, fact_schema(), None).map_err(parquet_err)?;
        writer.write(&record).map_err(parquet_err)?;
        writer.close().map_err(parquet_err)?;
        Ok(())
    }
}

impl Storage for ParquetStorage {
    fn backend_name(&self) -> &'static str {
        "parquet"
    }
}

/// Read a single Parquet file back into `Vec<Fact>` (union of all row groups).
fn read_parquet_file(path: &Path) -> Result<Vec<Fact>, StorageError> {
    let file = File::open(path)?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(parquet_err)?
        .build()
        .map_err(parquet_err)?;
    let mut out = Vec::new();
    for batch in reader {
        let batch = batch.map_err(parquet_err)?;
        out.extend(record_batch_to_facts(&batch)?);
    }
    Ok(out)
}

/// Inverse of `FactBatch::to_record_batch`: reconstruct `Fact` rows from columns.
fn record_batch_to_facts(rb: &RecordBatch) -> Result<Vec<Fact>, StorageError> {
    let col = |idx: usize| -> Result<&StringArray, StorageError> {
        rb.column(idx)
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| StorageError::Parquet(format!("column {idx} not Utf8")))
    };
    let id = col(0)?;
    let kind = col(1)?;
    let source = col(2)?;
    let resource_id = col(3)?;
    let resource_type = col(4)?;
    let timestamp = rb
        .column(5)
        .as_any()
        .downcast_ref::<UInt64Array>()
        .ok_or_else(|| StorageError::Parquet("timestamp column not UInt64".into()))?;
    let attributes_json = col(6)?;

    let mut out = Vec::with_capacity(rb.num_rows());
    for i in 0..rb.num_rows() {
        out.push(Fact::new(
            id.value(i),
            kind.value(i),
            source.value(i),
            resource_id.value(i),
            resource_type.value(i),
            timestamp.value(i),
            attributes_json.value(i),
        ));
    }
    Ok(out)
}

fn parquet_err<E: std::fmt::Display>(e: E) -> StorageError {
    StorageError::Parquet(e.to_string())
}

/// UTC `YYYY-MM-DD` for a Unix-seconds timestamp (dependency-free civil-from-days).
///
/// Howard Hinnant's `civil_from_days` algorithm; `secs / 86400` truncates to the
/// UTC calendar day. Used only for partition folder names.
fn utc_date_string(secs: u64) -> String {
    let days = (secs / 86400) as i64; // days since 1970-01-01
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::Fact;

    fn fact(id: &str, source: &str, ts: u64) -> Fact {
        Fact::new(
            id,
            "topology-node",
            source,
            format!("demo:ns:{id}"),
            "Node",
            ts,
            r#"{"k":"v"}"#,
        )
    }

    #[test]
    fn utc_date_known_values() {
        // 2026-07-23T00:00:00Z ≈ 1784937600 (verified via the same algorithm).
        // Spot-check a few known epochs.
        assert_eq!(utc_date_string(0), "1970-01-01");
        assert_eq!(utc_date_string(86_400), "1970-01-02");
        // 2000-03-01 is the classic leap-edge test for civil_from_days.
        // 2000-03-01T00:00:00Z = 951868800
        assert_eq!(utc_date_string(951_868_800), "2000-03-01");
        // 2024-02-29 (leap day) = 1709164800
        assert_eq!(utc_date_string(1_709_164_800), "2024-02-29");
        // Pin the epochs the partition tests rely on (off-by-one-day epoch comments
        // caused a prior test failure — these assertions lock the mapping).
        // 2026-07-23T00:00:00Z = 1784764800 ; 2026-07-24 = 1784851200.
        assert_eq!(utc_date_string(1_784_764_800), "2026-07-23");
        assert_eq!(utc_date_string(1_784_851_200), "2026-07-24");
    }

    #[test]
    fn empty_batch_writes_nothing() {
        let tmp = tempfile_dir();
        let store = ParquetStorage::open(&tmp).unwrap();
        let written = store.archive_batch(&FactBatch::new()).unwrap();
        assert!(written.is_empty());
        assert!(store.read_all().unwrap().is_empty());
    }

    #[test]
    fn roundtrips_multi_source_batch_across_partitions() {
        let tmp = tempfile_dir();
        let store = ParquetStorage::open(&tmp).unwrap();
        // Two sources; k8s spans two UTC dates (day boundary), prometheus one date.
        // 2026-07-23T00:00:00Z = 1784764800 ; 2026-07-24 = 1784851200.
        let batch = FactBatch::from_vec(vec![
            fact("a", "k8s", 1_784_764_800),        // 2026-07-23
            fact("b", "k8s", 1_784_764_899),        // 2026-07-23
            fact("c", "k8s", 1_784_851_200),        // 2026-07-24 (next day)
            fact("d", "prometheus", 1_784_764_800), // 2026-07-23
        ]);
        let written = store.archive_batch(&batch).unwrap();
        // 3 partition files: (07-23,k8s) (07-24,k8s) (07-23,prometheus)
        assert_eq!(written.len(), 3);

        // read_all returns every fact (order not guaranteed -> sort by id).
        let mut all = store.read_all().unwrap();
        all.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(all.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), vec!["a", "b", "c", "d"]);
        // Field-faithful round-trip (not just ids).
        assert_eq!(all[0].source, "k8s");
        assert_eq!(all[0].resource_type, "Node");
        assert_eq!(all[0].attributes_json, r#"{"k":"v"}"#);

        // Partition read: only k8s 07-23 facts (a, b), not c (07-24) or d (prometheus).
        let p = store.read_partition("2026-07-23", "k8s").unwrap();
        let mut pids = p.iter().map(|f| f.id.as_str()).collect::<Vec<_>>();
        pids.sort();
        assert_eq!(pids, vec!["a", "b"]);
    }

    #[test]
    fn appends_second_batch_same_partition() {
        let tmp = tempfile_dir();
        let store = ParquetStorage::open(&tmp).unwrap();
        let b1 = FactBatch::from_vec(vec![fact("a", "k8s", 1_784_764_800)]);
        let b2 = FactBatch::from_vec(vec![fact("b", "k8s", 1_784_764_900)]);
        store.archive_batch(&b1).unwrap();
        store.archive_batch(&b2).unwrap();
        let p = store.read_partition("2026-07-23", "k8s").unwrap();
        let mut ids = p.iter().map(|f| f.id.as_str()).collect::<Vec<_>>();
        ids.sort();
        // Two batches, distinct max_ts -> two files -> both facts present (append, no overwrite).
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn storage_backend_name() {
        let tmp = tempfile_dir();
        let store = ParquetStorage::open(&tmp).unwrap();
        let s: Box<dyn Storage> = Box::new(store);
        assert_eq!(s.backend_name(), "parquet");
    }

    /// Unique temp dir per test (no shared fixture). Returns a path under env
    /// TMPDIR; test owns cleanup (leaks are harmless under /tmp).
    fn tempfile_dir() -> PathBuf {
        // Use std::process::id + a static counter via atomic would need sync; instead
        // rely on the test thread id through thread_name for uniqueness.
        let name = format!(
            "sre-parquet-test-{}-{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("test")
                .replace(':', "_")
        );
        let p = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&p);
        p
    }
}
