//! Snapshot the current latest facts from SQLite into a Parquet archive.
//!
//! Reads `latest_topology_facts` + `latest_metric_facts` from a SQLite DB, wraps
//! them in a `FactBatch`, and appends them to a `(date, source)`-partitioned
//! Parquet archive via [`ParquetStorage`]. Repeated runs accumulate snapshots
//! into history (data-lake style: one file per run per partition).
//!
//! Until Parquet archival is wired into the live sync path, this is the manual
//! snapshot / migration-to-columnar tool. Point-in-time state capture; run on a
//! schedule (cron / `make engine-archive`) to build a historical Fact archive.
//!
//! ```bash
//! cargo run -p engine-storage --example archive_facts -- /path/db.sqlite /path/archive_dir
//! ```

use engine_core::FactBatch;
use engine_storage::{ParquetStorage, SqliteStorage};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = std::env::args()
        .nth(1)
        .expect("usage: archive_facts <sqlite-path> <archive-dir>");
    let archive_dir = std::env::args()
        .nth(2)
        .expect("usage: archive_facts <sqlite-path> <archive-dir>");

    let storage = SqliteStorage::connect(&db).await?;
    storage.migrate().await?;

    // Union of current latest topology + metric facts = a point-in-time snapshot.
    let mut facts = storage.latest_topology_facts().await?;
    let topo_n = facts.len();
    let metrics = storage.latest_metric_facts().await?;
    let metric_n = metrics.len();
    facts.extend(metrics);
    let total = facts.len();

    let batch = FactBatch::from_vec(facts);
    let parquet = ParquetStorage::open(&archive_dir)?;
    let written = parquet.archive_batch(&batch)?;

    println!(
        "archived {} facts (topology={}, metrics={}) -> {} partition file(s) under {}",
        total, topo_n, metric_n, written.len(), archive_dir
    );
    for p in &written {
        println!("  {}", p.display());
    }
    Ok(())
}
