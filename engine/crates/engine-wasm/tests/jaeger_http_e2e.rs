//! 端到端集成测试 —— jaeger connector 走 `http-client` capability 真发 HTTP。
//!
//! 起本地 mock HTTP server 按 path 路由(`/api/services` vs `/api/traces`),host
//! 加载 jaeger.wasm 注入 http-client capability,调 sync()。验证:
//! - guest 发 GET /api/services + /api/traces 拿到 bytes
//! - Jaeger JSON → CALLS topology-edge Fact 聚合正确(端点 = comp 节点 id + call_count)
//! - **deny-by-default**:不申明 http-client 时 host 拒绝,整轮 0 fact + error note
//!
//! 找不到 jaeger.wasm 时跳过(本地需先 `cd modules && cargo wasi-build`)。

use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;

use engine_wasm::WasmConnector;

fn modules_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../modules")
}

fn locate_jaeger_wasm() -> Option<PathBuf> {
    let modules = modules_root();
    let text = fs::read_to_string(modules.join("manifest.toml")).ok()?;
    let parsed = engine_wasm::ManifestFile::from_toml_str(&text).ok()?;
    let m = parsed.modules.iter().find(|m| m.name == "jaeger")?;
    let wasm = modules.join(&m.wasm_path);
    wasm.exists().then_some(wasm)
}

/// /api/services —— 两个 otel-demo 服务。
const SERVICES_JSON: &str = r#"{"data":["frontend","cartservice"],"total":2,"limit":0,"offset":0,"errors":null}"#;

/// /api/traces —— 一条 frontend(parent)→ cartservice(child)trace,3 个 child span
/// 各带一条 CHILD_OF→parent(故 (frontend,cartservice) 计数 3/trace)。两个服务都查
/// 这条 trace → 聚合后 call_count_5m = 3 + 3 = 6。
const TRACES_JSON: &str = r#"{"data":[{"traceID":"t1","spans":[{"traceID":"t1","spanID":"s1","operationName":"GET /","references":null,"startTime":0,"duration":1,"tags":[],"processID":"p1","warnings":null},{"traceID":"t1","spanID":"s2","operationName":"GET /cart","references":[{"refType":"CHILD_OF","traceID":"t1","spanID":"s1"}],"startTime":0,"duration":1,"tags":[],"processID":"p2","warnings":null},{"traceID":"t1","spanID":"s3","operationName":"GET /cart","references":[{"refType":"CHILD_OF","traceID":"t1","spanID":"s1"}],"startTime":0,"duration":1,"tags":[],"processID":"p2","warnings":null},{"traceID":"t1","spanID":"s4","operationName":"GET /cart","references":[{"refType":"CHILD_OF","traceID":"t1","spanID":"s1"}],"startTime":0,"duration":1,"tags":[],"processID":"p2","warnings":null}],"processes":{"p1":{"serviceName":"frontend","tags":[]},"p2":{"serviceName":"cartservice","tags":[]}}}],"total":1,"limit":0,"offset":0,"errors":null}"#;

/// 按 path 路由的 mock HTTP server:含 `/api/traces` 返 trace JSON,否则返 services JSON。
fn spawn_mock_jaeger() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock jaeger");
    let addr = listener.local_addr().expect("local addr");
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            // 请求首行:`GET <path> HTTP/1.1`
            let path = req.lines().next().unwrap_or("").split_whitespace().nth(1).unwrap_or("");
            let body = if path.contains("/api/traces") {
                TRACES_JSON
            } else {
                SERVICES_JSON
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
async fn jaeger_emits_calls_edges_via_http_capability() {
    let Some(wasm_path) = locate_jaeger_wasm() else {
        eprintln!("skipping: jaeger.wasm not built. Run `cd modules && cargo wasi-build`.");
        return;
    };
    let base = spawn_mock_jaeger();

    let caps: HashSet<String> = ["logging", "clock", "http-client"]
        .into_iter()
        .map(String::from)
        .collect();
    let mut conn = WasmConnector::load(&wasm_path, caps)
        .await
        .expect("load jaeger.wasm");

    // 两个服务各查一次同一 trace → 聚合 (frontend,cartservice) = 6,threshold 2 → 1 边。
    let config = format!(
        r#"{{"jaeger_url":"{base}","cluster":"local","namespace":"otel-demo","release_prefix":"otel-demo","lookback_seconds":300,"call_count_threshold":2,"limit_per_service":100}}"#
    );
    let outcome = conn.sync(&config).await.expect("sync");

    assert!(outcome.errors.is_empty(), "unexpected errors: {:?}", outcome.errors);
    assert_eq!(outcome.facts.len(), 1, "one CALLS edge");

    let f = &outcome.facts[0];
    assert_eq!(f.kind, "topology-edge");
    assert_eq!(f.source, "jaeger");
    assert_eq!(f.resource_type, "Edge");
    assert_eq!(
        f.resource_id,
        "edge:CALLS:comp:local:otel-demo:frontend->comp:local:otel-demo:cart"
    );
    let attrs: serde_json::Value = serde_json::from_str(&f.attributes_json).expect("attrs json");
    assert_eq!(attrs["edge_type"], "CALLS");
    assert_eq!(attrs["source"], "comp:local:otel-demo:frontend");
    assert_eq!(attrs["target"], "comp:local:otel-demo:cart");
    assert_eq!(attrs["call_count_5m"], 6);
    assert_eq!(attrs["discovery_method"], "jaeger_connector");
    assert!(f.timestamp > 1_700_000_000, "host clock stamps ts");
}

#[tokio::test]
async fn jaeger_without_capability_is_denied() {
    let Some(wasm_path) = locate_jaeger_wasm() else {
        eprintln!("skipping: jaeger.wasm not built.");
        return;
    };
    let base = spawn_mock_jaeger();

    // 不申明 http-client → deny by default;首个 GET /api/services 被拒 → 整轮早退。
    let caps: HashSet<String> = ["logging", "clock"].into_iter().map(String::from).collect();
    let mut conn = WasmConnector::load(&wasm_path, caps)
        .await
        .expect("load jaeger.wasm");

    let config = format!(r#"{{"jaeger_url":"{base}","call_count_threshold":2}}"#);
    let outcome = conn.sync(&config).await.expect("sync");

    assert!(outcome.facts.is_empty(), "denied → no facts");
    assert_eq!(outcome.errors.len(), 1, "one denied-services error note");
    assert!(
        outcome.errors[0].contains("unauthorized"),
        "error should mention unauthorized: {:?}",
        outcome.errors
    );
}
