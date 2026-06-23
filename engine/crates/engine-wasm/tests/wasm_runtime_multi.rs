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

fn read_manifest() -> Option<ManifestFile> {
    let path = modules_root().join("manifest.toml");
    let text = std::fs::read_to_string(&path).ok()?;
    ManifestFile::from_toml_str(&text).ok()
}

#[tokio::test]
async fn wasm_runtime_loads_hello_world_from_manifest() {
    let Some(manifest) = read_manifest() else {
        eprintln!("skipping: modules/manifest.toml not found");
        return;
    };
    let wasm_path = modules_root().join("target/wasm32-wasip2/release/hello_world.wasm");
    if !wasm_path.exists() {
        eprintln!(
            "skipping: hello_world.wasm not built. Run `cd modules && cargo wasi-build` first."
        );
        return;
    }

    let rt = WasmRuntime::from_manifest(&modules_root(), &manifest)
        .await
        .expect("from_manifest");

    assert_eq!(rt.connector_count(), 1);
    assert_eq!(rt.connector_names(), vec!["hello-world"]);
    assert!(
        rt.load_errors.is_empty(),
        "no load errors expected, got: {:?}",
        rt.load_errors
    );

    let summary = rt.sync_all("{}").await;
    assert_eq!(summary.per_connector.len(), 1);
    assert_eq!(summary.per_connector[0].name, "hello-world");
    assert_eq!(summary.per_connector[0].fact_count, 1);
    assert!(summary.per_connector[0].errors.is_empty());
    assert_eq!(summary.batch.len(), 1);
    assert_eq!(summary.total_errors, 0);

    // canonical Fact 字段平移正确
    let fact = &summary.batch.as_slice()[0];
    assert_eq!(fact.id, "hello-world-fact-1");
    assert_eq!(fact.source, "hello-world");
    assert_eq!(fact.kind, "topology-node");

    // Arrow RecordBatch 转换正确
    let rb = summary.batch.to_record_batch().expect("to_record_batch");
    assert_eq!(rb.num_rows(), 1);
    assert_eq!(rb.num_columns(), 7);

    eprintln!(
        "✓ WasmRuntime: {} connector(s), {} fact(s), arrow {} rows × {} cols",
        rt.connector_count(),
        summary.batch.len(),
        rb.num_rows(),
        rb.num_columns(),
    );
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
