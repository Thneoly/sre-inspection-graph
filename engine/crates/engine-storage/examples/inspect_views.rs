//! Headless verification of all 6 inspection views against an on-disk SQLite
//! materialized topology.
//!
//! Mirrors the **exact code path** the desktop `commands/views.rs` commands use
//! (`materialized_topology()` → [`engine_identity::views::subgraph`] →
//! [`engine_identity::topology_to_graph`]), so a non-empty result here is strong
//! evidence the GUI command returns real data without launching the app.
//!
//! Picks a representative start node per view (first node of the expected type)
//! and prints node/edge counts. `alert-aggregation` has no live alert source in
//! headless mode → fed an empty `AlertRegistry` (expected empty, documented gap).
//!
//! ```bash
//! cargo run -p engine-storage --example inspect_views -- /path/to/db.sqlite
//! ```

use std::collections::BTreeMap;

use engine_changes::{alert_aggregation_graph, AlertRegistry, DEFAULT_ALERT_AGGREGATION_DEPTH};
use engine_identity::{
    subgraph, topology_to_graph, TraversalDir, ACCESS_LINK_EDGES, CONFIG_IMPACT_EDGES,
    IMAGE_RISK_EDGES, NODE_IMPACT_EDGES,
};
use engine_storage::SqliteStorage;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .expect("usage: inspect_views <sqlite-path>");
    let storage = SqliteStorage::connect(&path).await?;
    storage.migrate().await?;
    let topo = storage.materialized_topology().await?;

    // --- topology overview ---
    let mut by_type: BTreeMap<&str, usize> = BTreeMap::new();
    for n in &topo.nodes {
        *by_type.entry(n.resource_type.as_str()).or_default() += 1;
    }
    let mut by_edge: BTreeMap<&str, usize> = BTreeMap::new();
    for e in &topo.edges {
        *by_edge.entry(e.edge_type.as_str()).or_default() += 1;
    }
    println!(
        "topology: {} nodes / {} edges",
        topo.nodes.len(),
        topo.edges.len()
    );
    println!("  node types:  {by_type:?}");
    println!("  edge types:  {by_edge:?}");
    println!();

    // --- helper: first node id of a given resource_type ---
    let first_of = |t: &str| {
        topo.nodes
            .iter()
            .find(|n| n.resource_type == t)
            .map(|n| n.resource_id.as_str())
    };

    // --- 4 graph-traversal views (subgraph primitive) ---
    // (name, start resource_type, edge whitelist, direction, default depth)
    let views: &[(&str, &str, &[&str], TraversalDir, usize)] = &[
        ("node-impact  ", "Node", NODE_IMPACT_EDGES, TraversalDir::Reverse, 4),
        ("config-impact", "ConfigMap", CONFIG_IMPACT_EDGES, TraversalDir::Reverse, 4),
        ("access-link  ", "Application", ACCESS_LINK_EDGES, TraversalDir::Both, 5),
        ("image-risk   ", "ContainerImage", IMAGE_RISK_EDGES, TraversalDir::Reverse, 4),
    ];
    for (name, start_type, edges, dir, depth) in views {
        match first_of(start_type) {
            None => println!(
                "{name}: NO `{start_type}` node in topology (empty — expected if connector 产该类型)"
            ),
            Some(start) => {
                let sub = subgraph(&topo, start, *depth, edges, *dir);
                let g = topology_to_graph(&sub);
                println!(
                    "{name}: start=`{start}` ({start_type}) -> {} nodes / {} edges",
                    g.summary.total_nodes, g.summary.total_edges
                );
            }
        }
    }

    // --- alert-aggregation (no live source headless -> empty registry) ---
    let reg = AlertRegistry::new();
    let g = alert_aggregation_graph(&reg, &topo, None, DEFAULT_ALERT_AGGREGATION_DEPTH);
    println!(
        "alert-agg    : empty AlertRegistry -> {} nodes / {} edges (no live source; record alerts in-app to populate)",
        g.summary.total_nodes, g.summary.total_edges
    );

    Ok(())
}
