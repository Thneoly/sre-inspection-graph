//! Headless dump of the materialized topology as the rendered `GraphResponse`.
//!
//! Exercises the exact Phase 2.5 `get_graph` read path against an on-disk SQLite
//! file: `SqliteStorage::connect` + `materialized_topology()` +
//! `engine_identity::topology_to_graph()`. Useful for GUI-less verification.
//!
//! ```bash
//! cargo run -p engine-storage --example dump_topology -- /path/to/db.sqlite
//! ```

use engine_storage::SqliteStorage;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .expect("usage: dump_topology <sqlite-path>");
    let storage = SqliteStorage::connect(&path).await?;
    storage.migrate().await?;
    let topology = storage.materialized_topology().await?;
    let graph = engine_identity::topology_to_graph(&topology);
    println!("{}", serde_json::to_string_pretty(&graph)?);
    eprintln!(
        "nodes={} edges={} risk={:?} health={:?}",
        graph.summary.total_nodes,
        graph.summary.total_edges,
        graph.summary.risk_counts,
        graph.summary.health_counts,
    );
    Ok(())
}
