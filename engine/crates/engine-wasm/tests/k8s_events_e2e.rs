//! 端到端集成测试 —— k8s-events connector 走 `http-client` capability,验证有状态 guest:
//! 首次 sync baseline(种 UID 不发),二次 sync 对新 UID 发 `kind="change"` Fact。
//! + deny-by-default。找不到 wasm 跳过(先 `cd modules && cargo wasi-build`)。

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
    let m = parsed.modules.iter().find(|m| m.name == "k8s-events")?;
    let wasm = modules.join(&m.wasm_path);
    wasm.exists().then_some(wasm)
}

/// 第 1 次请求返 [A];之后返 [A, B](B 是新 UID)。用 call counter 切换。
const EVENT_A: &str = r#"{"metadata":{"uid":"a1"},"reason":"ScalingReplicaSet","message":"scaled up","involvedObject":{"kind":"Deployment","namespace":"otel-demo","name":"frontend"}}"#;
const EVENT_B: &str = r#"{"metadata":{"uid":"b1"},"reason":"ScalingReplicaSet","message":"scaled up","involvedObject":{"kind":"Deployment","namespace":"otel-demo","name":"cart"}}"#;

fn spawn_mock_events() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let count = AtomicU64::new(0);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let n = count.fetch_add(1, Ordering::SeqCst);
            let body = if n == 0 {
                format!(r#"{{"items":[{}]}}"#, EVENT_A)
            } else {
                format!(r#"{{"items":[{},{}]}}"#, EVENT_A, EVENT_B)
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
async fn k8s_events_baseline_then_emits_new_event() {
    let Some(wasm_path) = locate_wasm() else {
        eprintln!("skipping: k8s_events.wasm not built. Run `cd modules && cargo wasi-build`.");
        return;
    };
    let base = spawn_mock_events();
    let caps: HashSet<String> = ["logging", "clock", "http-client"].into_iter().map(String::from).collect();
    let mut conn = WasmConnector::load(&wasm_path, caps).await.expect("load k8s_events.wasm");
    let config = format!(r#"{{"api_base":"{base}","cluster":"local","namespace":"otel-demo"}}"#);

    // 1) 首次 sync = baseline:种 a1,不发(即使 a1 是 interesting reason)。
    let o1 = conn.sync(&config).await.expect("sync");
    assert!(o1.facts.is_empty(), "baseline emits nothing: {:?}", o1.facts);
    assert!(o1.errors.is_empty(), "no errors: {:?}", o1.errors);

    // 2) 二次 sync:mock 返 [A, B];a1 已 seen,b1 新 -> 1 change-fact。
    let o2 = conn.sync(&config).await.expect("sync");
    assert_eq!(o2.facts.len(), 1, "one new event -> one change fact");
    let f = &o2.facts[0];
    assert_eq!(f.kind, "change");
    assert_eq!(f.source, "k8s-events");
    assert_eq!(f.resource_type, "ChangeEvent");
    let attrs: serde_json::Value = serde_json::from_str(&f.attributes_json).expect("attrs json");
    assert_eq!(attrs["change_type"], "deployment_rolled");
    assert_eq!(attrs["target_resource_id"], "deploy:local:otel-demo:cart");
    assert_eq!(attrs["source"], "k8s_api"); // ChangeRequest source, validated by record_change
    assert_eq!(attrs["changed_by"], "k8s");
    assert!(f.timestamp > 1_700_000_000);
}

#[tokio::test]
async fn k8s_events_without_capability_is_denied() {
    let Some(wasm_path) = locate_wasm() else {
        eprintln!("skipping: k8s_events.wasm not built.");
        return;
    };
    let base = spawn_mock_events();
    let caps: HashSet<String> = ["logging", "clock"].into_iter().map(String::from).collect();
    let mut conn = WasmConnector::load(&wasm_path, caps).await.expect("load");
    let config = format!(r#"{{"api_base":"{base}"}}"#);
    let o = conn.sync(&config).await.expect("sync");
    assert!(o.facts.is_empty(), "denied -> no facts");
    assert!(o.errors.iter().any(|e| e.contains("unauthorized")), "unauthorized note: {:?}", o.errors);
}
