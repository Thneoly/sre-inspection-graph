//! 端到端集成测试 —— host wasmtime 真加载 hello_world.wasm Component,
//! 调 guest 的 sync() / health_check(),验证全链路:
//!
//! WIT → wit-bindgen (guest) → wasm32-wasip2 build → wasmtime Component →
//! ConnectorWorld::instantiate_async → call_sync → SyncOutcome (Vec<HostFact>)
//!
//! 这是 Phase 1 → Phase 2 的核心交接点。此测试通过即证明:
//! - host bindgen 与 guest bindgen WIT 一致
//! - capability 注入(logging / clock / http-client)能正常 link
//! - sync 函数能拿回真实 Fact 数据
//!
//! 测试 **可选**:找不到 hello_world.wasm 时跳过(本地需先 `cd modules &&
//! cargo wasi-build`)。

use std::fs;
use std::path::PathBuf;

use engine_wasm::WasmConnector;

fn modules_root() -> PathBuf {
    let crate_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(crate_dir).join("../../../modules")
}

fn locate_hello_world_wasm() -> Option<PathBuf> {
    let modules = modules_root();
    let manifest_path = modules.join("manifest.toml");
    let text = fs::read_to_string(&manifest_path).ok()?;
    let parsed = engine_wasm::ManifestFile::from_toml_str(&text).ok()?;
    let hello = parsed.modules.iter().find(|m| m.name == "hello-world")?;
    let wasm = modules.join(&hello.wasm_path);
    if wasm.exists() {
        Some(wasm)
    } else {
        None
    }
}

#[tokio::test]
async fn loads_and_calls_hello_world_sync() {
    let Some(wasm_path) = locate_hello_world_wasm() else {
        eprintln!(
            "skipping: hello_world.wasm not built. \
             Run `cd modules && cargo wasi-build` first."
        );
        return;
    };

    let mut conn = WasmConnector::load(&wasm_path)
        .await
        .expect("load hello_world.wasm");

    // 1) health-check 应当返 true
    let healthy = conn.health_check().await.expect("health-check");
    assert!(healthy, "hello-world should report healthy");

    // 2) sync() 应当回 1 条 Fact,errors 空
    let outcome = conn.sync("{}").await.expect("sync");
    assert_eq!(outcome.facts.len(), 1, "should emit exactly one fact");
    assert!(outcome.errors.is_empty(), "should have no errors");

    let fact = &outcome.facts[0];
    assert_eq!(fact.id, "hello-world-fact-1");
    assert_eq!(fact.kind, "topology-node");
    assert_eq!(fact.source, "hello-world");
    assert_eq!(fact.resource_id, "demo:placeholder:default:hello");
    assert_eq!(fact.resource_type, "Placeholder");
    // timestamp 由 host 的 clock capability 实现给(SystemTime::now)
    assert!(
        fact.timestamp > 1_700_000_000,
        "timestamp should be a real unix epoch (got {})",
        fact.timestamp
    );

    // attributes_json 是 guest 内嵌的字符串
    let attrs: serde_json::Value =
        serde_json::from_str(&fact.attributes_json).expect("valid json attrs");
    assert_eq!(attrs["greeting"], "hello, world");

    eprintln!(
        "✓ host loaded + called hello_world.wasm: 1 fact emitted, timestamp={}",
        fact.timestamp
    );
}

#[tokio::test]
async fn sync_twice_emits_consistent_facts() {
    let Some(wasm_path) = locate_hello_world_wasm() else {
        return;
    };
    let mut conn = WasmConnector::load(&wasm_path).await.unwrap();

    let first = conn.sync("{}").await.unwrap();
    let second = conn.sync("{}").await.unwrap();

    assert_eq!(first.facts.len(), 1);
    assert_eq!(second.facts.len(), 1);
    // 同一 instance 多次 sync 是允许的(stateless connector)
    assert_eq!(first.facts[0].id, second.facts[0].id);
    // 时间戳应单调不减(同秒内可能相等)
    assert!(second.facts[0].timestamp >= first.facts[0].timestamp);
}
