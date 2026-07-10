//! `WasmRuntime` 集成测试 —— 复用 `modules/manifest.toml`,起一个多 connector
//! 编排器实例(实际只有 1 个 hello-world,但流程同 N 个)。
//!
//! 与 `host_loads_guest_e2e.rs` 的区别:那个测单 `WasmConnector::sync`,
//! 这个测 `WasmRuntime::from_manifest` + `sync_all` + 转 Arrow RecordBatch
//! 的聚合路径。

use std::path::PathBuf;

use engine_wasm::{ManifestFile, WasmRuntime};

fn modules_root() -> PathBuf {
    let crate_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(crate_dir).join("../../../modules")
}

#[tokio::test]
async fn wasm_runtime_loads_hello_world_from_manifest() {
    let wasm_path = modules_root().join("target/wasm32-wasip2/release/hello_world.wasm");
    if !wasm_path.exists() {
        eprintln!(
            "skipping: hello_world.wasm not built. Run `cd modules && cargo wasi-build` first."
        );
        return;
    }

    // 合成 manifest(只启 hello-world)-- repo manifest Phase 2.7 起为真集群配置
    // (hello-world disabled),不适合此确定性 from_manifest+sync_all+Arrow 验证。
    let toml = r#"
schema_version = "1"

[[modules]]
name = "hello-world"
type = "connector"
wasm_path = "target/wasm32-wasip2/release/hello_world.wasm"
version = "0.1.0"
capabilities = []
sync_interval_seconds = 60
"#;
    let manifest = ManifestFile::from_toml_str(toml).expect("parse manifest");
    let rt = WasmRuntime::from_manifest(&modules_root(), &manifest)
        .await
        .expect("from_manifest");

    // 合成 manifest 只启 hello-world,恰好 1 个 connector
    assert_eq!(rt.connector_count(), 1, "should load exactly hello-world");
    assert!(
        rt.connector_names().contains(&"hello-world"),
        "hello-world should be among loaded connectors, got: {:?}",
        rt.connector_names()
    );
    // 至少这次没有 hello-world 自身的加载失败
    for (name, err) in &rt.load_errors {
        assert_ne!(
            name, "hello-world",
            "hello-world should not fail to load: {err}"
        );
    }

    let summary = rt.sync_all("{}").await;
    // 找到 hello-world 那行单独断言
    let hw = summary
        .per_connector
        .iter()
        .find(|s| s.name == "hello-world")
        .expect("hello-world should appear in sync summary");
    assert_eq!(hw.fact_count, 1);
    assert!(hw.errors.is_empty());

    // canonical Fact 字段平移正确 —— 从 batch 里找 hello-world source 的 Fact
    let fact = summary
        .batch
        .as_slice()
        .iter()
        .find(|f| f.source == "hello-world")
        .expect("hello-world fact in batch");
    assert_eq!(fact.id, "hello-world-fact-1");
    assert_eq!(fact.kind, "topology-node");

    // Arrow RecordBatch 转换 — 行数应等于聚合 fact 总数
    let rb = summary.batch.to_record_batch().expect("to_record_batch");
    assert_eq!(rb.num_rows(), summary.batch.len());
    assert_eq!(rb.num_columns(), 7);

    eprintln!(
        "✓ WasmRuntime: {} connector(s), {} fact(s), arrow {} rows × {} cols",
        rt.connector_count(),
        summary.batch.len(),
        rb.num_rows(),
        rb.num_columns(),
    );
}

/// `WasmRuntime::empty` —— Tauri F path fallback,modules 没 build / manifest 缺失
/// 时让 host 起得来。
#[tokio::test]
async fn wasm_runtime_empty_is_inert_but_callable() {
    let rt = WasmRuntime::empty(modules_root());
    assert_eq!(rt.connector_count(), 0);
    assert!(rt.connector_names().is_empty());
    assert!(rt.load_errors.is_empty());

    let summary = rt.sync_all("{}").await;
    assert_eq!(summary.batch.len(), 0);
    assert_eq!(summary.per_connector.len(), 0);
    assert_eq!(summary.total_errors, 0);
}

#[tokio::test]
async fn wasm_runtime_records_load_errors_for_missing_wasm() {
    // 不依赖真实 wasm,造一个指向不存在文件的 manifest
    let toml = r#"
schema_version = "1"

[[modules]]
name = "ghost-connector"
type = "connector"
wasm_path = "does/not/exist.wasm"
version = "0.1.0"
"#;
    let manifest = ManifestFile::from_toml_str(toml).expect("parse manifest");
    let rt = WasmRuntime::from_manifest(&modules_root(), &manifest)
        .await
        .expect("from_manifest");

    // 加载失败应记入 load_errors,而不是把整个 runtime 干废
    assert_eq!(rt.connector_count(), 0);
    assert_eq!(rt.load_errors.len(), 1);
    assert_eq!(rt.load_errors[0].0, "ghost-connector");
    assert!(
        rt.load_errors[0].1.contains("wasm file not found"),
        "error msg should explain: {}",
        rt.load_errors[0].1
    );

    // sync_all 在 0 connector 时返空 batch,而不是 panic
    let summary = rt.sync_all("{}").await;
    assert_eq!(summary.batch.len(), 0);
    assert_eq!(summary.per_connector.len(), 0);
}

#[tokio::test]
async fn wasm_runtime_skips_non_connector_modules() {
    let toml = r#"
schema_version = "1"

[[modules]]
name = "future-rule"
type = "rule"
wasm_path = "does/not/exist.wasm"
version = "0.1.0"
"#;
    let manifest = ManifestFile::from_toml_str(toml).expect("parse manifest");
    let rt = WasmRuntime::from_manifest(&modules_root(), &manifest)
        .await
        .expect("from_manifest");

    // type=rule 的模块本期跳过(留 Phase 3 加 rule world),既不算 entry 也不
    // 报 load error
    assert_eq!(rt.connector_count(), 0);
    assert!(rt.load_errors.is_empty());
}

/// k8s-mini 是否已经 wasm-build —— 测试在 CI 上随 modules/cargo wasi-build 全量
/// 起跑,本地手跑 cargo test 时若未 build 则 skip。
fn k8s_mini_wasm_built() -> bool {
    modules_root()
        .join("target/wasm32-wasip2/release/k8s_mini.wasm")
        .exists()
}

#[tokio::test]
async fn k8s_mini_emits_per_namespace_facts_from_config_json() {
    if !k8s_mini_wasm_built() {
        eprintln!(
            "skipping: k8s_mini.wasm not built. Run `cd modules && cargo wasi-build` first."
        );
        return;
    }

    // 单独造个只含 k8s-mini 的 manifest,避免 hello-world 的 fact 干扰断言。
    let toml = r#"
schema_version = "1"

[[modules]]
name = "k8s-mini"
type = "connector"
wasm_path = "target/wasm32-wasip2/release/k8s_mini.wasm"
version = "0.1.0"
capabilities = ["logging", "clock"]
sync_interval_seconds = 30
"#;
    let manifest = ManifestFile::from_toml_str(toml).expect("parse manifest");
    let rt = WasmRuntime::from_manifest(&modules_root(), &manifest)
        .await
        .expect("from_manifest");
    assert_eq!(rt.connector_count(), 1);
    assert!(rt.load_errors.is_empty(), "{:?}", rt.load_errors);

    // 传入 3-namespace 配置,期望产 3 条 Fact
    let cfg = r#"{"cluster":"vm-cluster","namespaces":["default","kube-system","otel-demo"]}"#;
    let summary = rt.sync_all(cfg).await;
    assert_eq!(summary.per_connector.len(), 1);
    assert_eq!(summary.per_connector[0].name, "k8s-mini");
    assert_eq!(
        summary.per_connector[0].fact_count, 3,
        "should emit 3 facts for 3 namespaces"
    );
    assert!(summary.per_connector[0].errors.is_empty());

    // 每条 Fact 的 schema 都对得上
    let facts = summary.batch.as_slice();
    assert_eq!(facts.len(), 3);
    for fact in facts {
        assert_eq!(fact.source, "k8s-mini");
        assert_eq!(fact.kind, "topology-node");
        assert_eq!(fact.resource_type, "Namespace");
        assert!(fact.resource_id.starts_with("ns:vm-cluster:"));
        // attributes_json 里有 cluster=vm-cluster
        assert!(fact.attributes_json.contains(r#""cluster":"vm-cluster""#));
        // timestamp 是 host clock 注入的真 Unix epoch
        assert!(fact.timestamp > 1_700_000_000);
    }

    let resource_ids: Vec<&str> = facts.iter().map(|f| f.resource_id.as_str()).collect();
    assert!(resource_ids.contains(&"ns:vm-cluster:default"));
    assert!(resource_ids.contains(&"ns:vm-cluster:kube-system"));
    assert!(resource_ids.contains(&"ns:vm-cluster:otel-demo"));
}

#[tokio::test]
async fn k8s_mini_falls_back_to_defaults_for_empty_config() {
    if !k8s_mini_wasm_built() {
        return;
    }
    let toml = r#"
schema_version = "1"

[[modules]]
name = "k8s-mini"
type = "connector"
wasm_path = "target/wasm32-wasip2/release/k8s_mini.wasm"
version = "0.1.0"
"#;
    let manifest = ManifestFile::from_toml_str(toml).expect("parse manifest");
    let rt = WasmRuntime::from_manifest(&modules_root(), &manifest)
        .await
        .expect("from_manifest");

    let summary = rt.sync_all("").await; // 完全空 config — guest 端走 Config::default()
    assert_eq!(summary.per_connector[0].fact_count, 1);
    let f = &summary.batch.as_slice()[0];
    assert_eq!(f.resource_id, "ns:local:default");
    assert!(f.attributes_json.contains(r#""cluster":"local""#));
}

#[tokio::test]
async fn k8s_mini_returns_runtime_error_on_invalid_json() {
    if !k8s_mini_wasm_built() {
        return;
    }
    let toml = r#"
schema_version = "1"

[[modules]]
name = "k8s-mini"
type = "connector"
wasm_path = "target/wasm32-wasip2/release/k8s_mini.wasm"
version = "0.1.0"
"#;
    let manifest = ManifestFile::from_toml_str(toml).expect("parse manifest");
    let rt = WasmRuntime::from_manifest(&modules_root(), &manifest)
        .await
        .expect("from_manifest");

    // 非法 JSON 串 → guest 返 SyncError::Config → host 在 run_sync 里折成 errors
    let summary = rt.sync_all("not-json").await;
    assert_eq!(summary.per_connector[0].fact_count, 0);
    assert_eq!(summary.per_connector[0].errors.len(), 1);
    assert!(
        summary.per_connector[0].errors[0].contains("sync failed"),
        "got: {}",
        summary.per_connector[0].errors[0]
    );
}

#[tokio::test]
async fn multi_connector_aggregates_into_single_batch() {
    if !modules_root()
        .join("target/wasm32-wasip2/release/hello_world.wasm")
        .exists()
        || !k8s_mini_wasm_built()
    {
        eprintln!("skipping: need both hello_world.wasm and k8s_mini.wasm built");
        return;
    }

    // 双 connector manifest,验证 sync_all 把两个 connector 的 fact 揉进一个 batch
    let toml = r#"
schema_version = "1"

[[modules]]
name = "hello-world"
type = "connector"
wasm_path = "target/wasm32-wasip2/release/hello_world.wasm"
version = "0.1.0"

[[modules]]
name = "k8s-mini"
type = "connector"
wasm_path = "target/wasm32-wasip2/release/k8s_mini.wasm"
version = "0.1.0"
"#;
    let manifest = ManifestFile::from_toml_str(toml).expect("parse manifest");
    let rt = WasmRuntime::from_manifest(&modules_root(), &manifest)
        .await
        .expect("from_manifest");
    assert_eq!(rt.connector_count(), 2);
    assert_eq!(rt.connector_names(), vec!["hello-world", "k8s-mini"]);

    let summary = rt.sync_all(r#"{"namespaces":["a","b"]}"#).await;
    // hello-world 不读 config,出 1 个;k8s-mini 读 2 个 namespace,出 2 个
    assert_eq!(summary.per_connector.len(), 2);
    assert_eq!(summary.batch.len(), 3);

    let hw = summary
        .per_connector
        .iter()
        .find(|s| s.name == "hello-world")
        .expect("hello-world present");
    let km = summary
        .per_connector
        .iter()
        .find(|s| s.name == "k8s-mini")
        .expect("k8s-mini present");
    assert_eq!(hw.fact_count, 1);
    assert_eq!(km.fact_count, 2);

    // Arrow batch 一次拿全 — Phase 3 起 engine-storage 一次写 parquet
    let rb = summary.batch.to_record_batch().expect("to_record_batch");
    assert_eq!(rb.num_rows(), 3);
    assert_eq!(rb.num_columns(), 7);
}

/// Phase 1 Step 2:`with_topology=true` 触发 k8s-mini 吐分层 mock 拓扑
/// (Cluster + 2 Node + N Namespace + 2N Pod + N Service)。供桌面 Cytoscape
/// 视图渲染。N=2 → 1+2+2+4+2 = 11 Fact;N=1 → 1+2+1+2+1 = 7 Fact。
///
/// 这里同时验证 `attributes_json` 含 `parent_resource_id` 字段 —— 前端按此
/// 字段建 edge。约定会被 Phase 2 真 K8s connector 继承。
#[tokio::test]
async fn k8s_mini_emits_full_topology_when_with_topology_true() {
    if !k8s_mini_wasm_built() {
        eprintln!("skipping: k8s_mini.wasm not built");
        return;
    }
    let toml = r#"
schema_version = "1"

[[modules]]
name = "k8s-mini"
type = "connector"
wasm_path = "target/wasm32-wasip2/release/k8s_mini.wasm"
version = "0.1.0"
capabilities = ["logging", "clock"]
"#;
    let manifest = ManifestFile::from_toml_str(toml).expect("parse manifest");
    let rt = WasmRuntime::from_manifest(&modules_root(), &manifest)
        .await
        .expect("from_manifest");

    let cfg = r#"{"cluster":"demo","namespaces":["default","app"],"with_topology":true}"#;
    let summary = rt.sync_all(cfg).await;
    let facts = summary.batch.as_slice();

    // 11 = 1 Cluster + 2 Node + 2 Namespace + 4 Pod + 2 Service
    assert_eq!(facts.len(), 11, "expected 11 hierarchical facts, got {}", facts.len());
    assert!(summary.per_connector[0].errors.is_empty());

    // 按 resource_type 数一下
    let mut by_type = std::collections::HashMap::<&str, usize>::new();
    for f in facts {
        *by_type.entry(f.resource_type.as_str()).or_default() += 1;
    }
    assert_eq!(by_type.get("Cluster").copied().unwrap_or(0), 1);
    assert_eq!(by_type.get("Node").copied().unwrap_or(0), 2);
    assert_eq!(by_type.get("Namespace").copied().unwrap_or(0), 2);
    assert_eq!(by_type.get("Pod").copied().unwrap_or(0), 4);
    assert_eq!(by_type.get("Service").copied().unwrap_or(0), 2);

    // Cluster 是顶层节点 — 不含 parent_resource_id
    let cluster = facts.iter().find(|f| f.resource_type == "Cluster").unwrap();
    assert_eq!(cluster.resource_id, "cluster:demo");
    assert!(
        !cluster.attributes_json.contains("parent_resource_id"),
        "Cluster should be root (no parent_resource_id), got: {}",
        cluster.attributes_json
    );

    // Node / Namespace 的 parent 是 cluster
    for kind in ["Node", "Namespace"] {
        for f in facts.iter().filter(|f| f.resource_type == kind) {
            assert!(
                f.attributes_json.contains(r#""parent_resource_id":"cluster:demo""#),
                "{} {} should parent cluster:demo, got: {}",
                kind, f.resource_id, f.attributes_json
            );
        }
    }

    // Pod / Service 的 parent 是 Namespace
    for f in facts.iter().filter(|f| f.resource_type == "Pod" || f.resource_type == "Service") {
        assert!(
            f.attributes_json.contains(r#""parent_resource_id":"ns:demo:"#),
            "{} {} should parent some ns:demo:*, got: {}",
            f.resource_type, f.resource_id, f.attributes_json
        );
    }
}
