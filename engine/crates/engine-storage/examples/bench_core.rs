//! 图谱内核路径基准:对真实 SQLite 库计 resolve(含 correlation 合并)/
//! facts_to_graph / diff / get_graph 读路径的耗时。
//!
//! ```bash
//! cargo run -p engine-storage --release --example bench_core -- \
//!   ~/.local/share/io.sregraph.desktop/sre-graph.sqlite 50
//! ```

use engine_storage::SqliteStorage;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).expect("usage: bench_core <sqlite> [iters]");
    let iters: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);

    let storage = SqliteStorage::connect(&path).await?;
    storage.migrate().await?;
    let facts = storage.latest_topology_facts().await?;
    let topo = engine_identity::resolve(&facts);
    println!(
        "input: {} facts -> {} nodes / {} edges",
        facts.len(),
        topo.nodes.len(),
        topo.edges.len()
    );

    // 1. get_graph 读路径(materialized 表 -> topology_to_graph)
    let mut s = Vec::new();
    for _ in 0..iters {
        let t = Instant::now();
        let m = storage.materialized_topology().await?;
        let g = engine_identity::topology_to_graph(&m);
        s.push(t.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(&g);
    }
    stat("get_graph read (materialized + to_graph)", &mut s);

    // 2. resolve 全路径(correlation pre-rewrite + facts_to_graph)
    let mut s = Vec::new();
    for _ in 0..iters {
        let t = Instant::now();
        let r = engine_identity::resolve(&facts);
        s.push(t.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(&r);
    }
    stat("resolve (correlation + facts_to_graph)", &mut s);

    // 3. 基线:仅 facts_to_graph(差值 ≈ correlation 合并的开销)
    let mut s = Vec::new();
    for _ in 0..iters {
        let t = Instant::now();
        let g = engine_core::facts_to_graph(&facts);
        s.push(t.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(&g);
    }
    stat("facts_to_graph only (baseline)", &mut s);

    // 4. diff(增量同步判定;两份相同拓扑 = 无变化的最坏全比对)
    let a = engine_identity::resolve(&facts);
    let b = engine_identity::resolve(&facts);
    let mut s = Vec::new();
    for _ in 0..iters {
        let t = Instant::now();
        let cs = engine_identity::diff(&a, &b);
        s.push(t.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(&cs);
    }
    stat("diff (identical topologies, no-change worst case)", &mut s);

    Ok(())
}

fn stat(name: &str, s: &mut [f64]) {
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean: f64 = s.iter().sum::<f64>() / s.len() as f64;
    println!(
        "{name}: mean={mean:.2}ms p50={:.2}ms max={:.2}ms (n={})",
        s[s.len() / 2],
        s[s.len() - 1],
        s.len()
    );
}
