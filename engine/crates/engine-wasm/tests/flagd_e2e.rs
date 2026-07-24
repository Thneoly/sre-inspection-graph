//! 端到端集成测试 —— flagd connector 经 `http-write` capability POST ResolveAll,
//! 验证有状态 guest:首次 baseline(存快照不发),二次 flip -> 带 scenario 富化的
//! `kind="change"` Fact。+ deny-by-default。找不到 wasm 跳过(先 `cd modules && cargo wasi-build`)。

use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use engine_wasm::WasmConnector;

fn modules_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../modules")
}

fn locate_wasm() -> Option<PathBuf> {
    let modules = modules_root();
    let text = fs::read_to_string(modules.join("manifest.toml")).ok()?;
    let parsed = engine_wasm::ManifestFile::from_toml_str(&text).ok()?;
    let m = parsed.modules.iter().find(|m| m.name == "flagd")?;
    let wasm = modules.join(&m.wasm_path);
    wasm.exists().then_some(wasm)
}

/// 第 1 次请求返 productCatalogFailure=off;之后返 on(翻转)。call counter 切换。
fn pcf(state: &str, val: bool) -> String {
    format!(r#"{{"variant":"{state}","boolValue":{val}}}"#)
}
fn snapshot_off() -> String {
    format!(r#"{{"flags":{{"productCatalogFailure":{}}}}}"#, pcf("off", false))
}
fn snapshot_on() -> String {
    format!(r#"{{"flags":{{"productCatalogFailure":{}}}}}"#, pcf("on", true))
}

fn spawn_mock_flagd() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let count = AtomicU64::new(0);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let body = if count.fetch_add(1, Ordering::SeqCst) == 0 {
                snapshot_off()
            } else {
                snapshot_on()
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn flagd_baseline_then_emits_scenario_enriched_change() {
    let Some(wasm_path) = locate_wasm() else {
        eprintln!("skipping: flagd.wasm not built. Run `cd modules && cargo wasi-build`.");
        return;
    };
    let base = spawn_mock_flagd();
    // flagd 只 write(POST),不 get -> 只需 http-write。
    let caps: HashSet<String> = ["logging", "clock", "http-write"].into_iter().map(String::from).collect();
    let mut conn = WasmConnector::load(&wasm_path, caps).await.expect("load flagd.wasm");
    let config = format!(
        r#"{{"flagd_url":"{base}","cluster":"local","namespace":"otel-demo","flagd_configmap_name":"otel-demo-flagd-config"}}"#
    );

    // 1) 首次 sync = baseline:存 off 快照,不发。
    let o1 = conn.sync(&config).await.expect("sync");
    assert!(o1.facts.is_empty(), "baseline emits nothing: {:?}", o1.facts);
    assert!(o1.errors.is_empty(), "no errors: {:?}", o1.errors);

    // 2) 二次 sync:mock 返 on;off->on 翻转 -> 1 change-fact,带 scenario 富化。
    let o2 = conn.sync(&config).await.expect("sync");
    assert_eq!(o2.facts.len(), 1, "one flip -> one change fact");
    let f = &o2.facts[0];
    assert_eq!(f.kind, "change");
    assert_eq!(f.source, "flagd");
    assert_eq!(f.resource_type, "ChangeEvent");
    let attrs: serde_json::Value = serde_json::from_str(&f.attributes_json).expect("attrs json");
    assert_eq!(attrs["change_type"], "configmap_updated");
    assert_eq!(attrs["source"], "flagd");
    assert_eq!(
        attrs["target_resource_id"],
        "configmap:local:otel-demo:otel-demo-flagd-config"
    );
    // scenario 富化(productCatalogFailure -> restart_pod / product-catalog)
    assert_eq!(attrs["diff_summary"]["scenario"]["recommended_action"], "restart_pod");
    assert_eq!(attrs["diff_summary"]["scenario"]["target_component"], "product-catalog");
    assert!(
        attrs["description"].as_str().unwrap().contains("scenario="),
        "description should carry scenario suffix: {}",
        attrs["description"]
    );
    assert!(f.timestamp > 1_700_000_000);
}

#[tokio::test]
async fn flagd_without_write_capability_is_denied() {
    let Some(wasm_path) = locate_wasm() else {
        eprintln!("skipping: flagd.wasm not built.");
        return;
    };
    let base = spawn_mock_flagd();
    // 缺 http-write -> write 被拒。
    let caps: HashSet<String> = ["logging", "clock"].into_iter().map(String::from).collect();
    let mut conn = WasmConnector::load(&wasm_path, caps).await.expect("load");
    let config = format!(r#"{{"flagd_url":"{base}"}}"#);
    let o = conn.sync(&config).await.expect("sync");
    assert!(o.facts.is_empty(), "denied -> no facts");
    assert!(
        o.errors.iter().any(|e| e.contains("unauthorized")),
        "unauthorized note: {:?}",
        o.errors
    );
}
