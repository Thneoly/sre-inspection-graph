//! 真集群端到端 —— k8s connector 经本地 `kubectl proxy` 拉真实 otel-demo 拓扑。
//!
//! **gated**:需 env `K8S_PROXY_BASE`(如 `http://127.0.0.1:8001`)+ 一个在跑的
//! `kubectl proxy`。未设则 skip(CI / 无集群环境零依赖)。mapper 纯逻辑由
//! `modules/connectors/k8s` 的 host 单测覆盖;本测试验证 http-client capability
//! 真打 K8s API + 真实数据映射。
//!
//! 跑法:
//! ```bash
//! kubectl proxy --port=8001 &
//! K8S_PROXY_BASE=http://127.0.0.1:8001 K8S_NAMESPACE=otel-demo \
//!   cargo test -p engine-wasm --test k8s_live_e2e -- --nocapture
//! ```

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use engine_wasm::WasmConnector;

fn modules_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../modules")
}

fn locate_k8s_wasm() -> Option<PathBuf> {
    let modules = modules_root();
    let text = fs::read_to_string(modules.join("manifest.toml")).ok()?;
    let parsed = engine_wasm::ManifestFile::from_toml_str(&text).ok()?;
    let m = parsed.modules.iter().find(|m| m.name == "k8s")?;
    let wasm = modules.join(&m.wasm_path);
    wasm.exists().then_some(wasm)
}

#[tokio::test]
async fn k8s_maps_real_cluster_topology_via_proxy() {
    let Ok(base) = std::env::var("K8S_PROXY_BASE") else {
        eprintln!("skipping: set K8S_PROXY_BASE (run `kubectl proxy --port=8001`) to enable.");
        return;
    };
    let Some(wasm_path) = locate_k8s_wasm() else {
        eprintln!("skipping: k8s.wasm not built. Run `cd modules && cargo wasi-build`.");
        return;
    };
    let ns = std::env::var("K8S_NAMESPACE").unwrap_or_else(|_| "otel-demo".to_string());

    let caps: HashSet<String> = ["logging", "clock", "http-client"]
        .into_iter()
        .map(String::from)
        .collect();
    let mut conn = WasmConnector::load(&wasm_path, caps)
        .await
        .expect("load k8s.wasm");

    let config = format!(r#"{{"api_base":"{base}","cluster":"vm","namespace":"{ns}"}}"#);
    let outcome = conn.sync(&config).await.expect("sync");

    eprintln!(
        "k8s live: facts={} errors={:?}",
        outcome.facts.len(),
        outcome.errors
    );
    assert!(outcome.errors.is_empty(), "GET errors: {:?}", outcome.errors);

    let by_type = |t: &str| outcome.facts.iter().filter(|f| f.resource_type == t).count();
    let has = |rid: &str| outcome.facts.iter().any(|f| f.resource_id == rid);

    assert!(has("cluster:vm"), "cluster node present");
    assert!(has(&format!("ns:vm:{ns}")), "namespace node present");
    assert!(by_type("Node") >= 1, "at least one Node");
    assert!(by_type("Deployment") >= 1, "at least one Deployment");
    assert!(by_type("Pod") >= 1, "at least one Pod");
    assert!(by_type("Service") >= 1, "at least one Service");

    // 每个 Pod 的 parent 要么是某 Deployment 要么退化到 Namespace —— 不能悬空
    let node_ids: HashSet<&str> = outcome.facts.iter().map(|f| f.resource_id.as_str()).collect();
    for f in outcome.facts.iter().filter(|f| f.resource_type == "Pod") {
        let attrs: serde_json::Value = serde_json::from_str(&f.attributes_json).unwrap();
        let parent = attrs["parent_resource_id"].as_str().unwrap_or("");
        assert!(
            node_ids.contains(parent),
            "pod {} parent {parent} must resolve to an emitted node",
            f.resource_id
        );
    }

    eprintln!(
        "k8s live counts: Node={} Deployment={} Pod={} Service={}",
        by_type("Node"),
        by_type("Deployment"),
        by_type("Pod"),
        by_type("Service")
    );

    // 收尾:真实 facts 走 engine_core::facts_to_graph,确认能成可渲染 GraphResponse。
    // 这是 desktop get_graph(2.5 后读 materialized,但派生逻辑同源)的领域核心。
    let core_facts: Vec<engine_core::Fact> = outcome
        .facts
        .iter()
        .map(|f| {
            engine_core::Fact::new(
                f.id.clone(),
                f.kind.clone(),
                f.source.clone(),
                f.resource_id.clone(),
                f.resource_type.clone(),
                f.timestamp,
                f.attributes_json.clone(),
            )
        })
        .collect();
    let graph = engine_core::facts_to_graph(&core_facts);
    eprintln!(
        "graph: nodes={} edges={} risk={:?} health={:?}",
        graph.summary.total_nodes,
        graph.summary.total_edges,
        graph.summary.risk_counts,
        graph.summary.health_counts
    );
    // 真实拓扑:节点全部成图,CONTAINS 边非空(parent 链都解析了)
    assert_eq!(graph.summary.total_nodes, outcome.facts.len());
    assert!(graph.summary.total_edges > 0, "parent links → CONTAINS edges");
    // otel-demo 有不健康 pod(cartservice 等)→ critical/warning 计数非零
    let unhealthy = graph.summary.health_counts.get("critical").copied().unwrap_or(0)
        + graph.summary.health_counts.get("warning").copied().unwrap_or(0);
    assert!(unhealthy > 0, "otel-demo should have unhealthy pods");
}
