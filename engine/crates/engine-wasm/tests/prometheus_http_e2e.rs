//! 端到端集成测试 —— prometheus connector 走 `http-client` capability 真发 HTTP。
//!
//! 起一个本地 mock HTTP server 返回 canned Prom JSON,host 加载 prometheus.wasm
//! 并注入 http-client capability,调 sync()。验证:
//! - guest 通过 capability 发出 GET /api/v1/query 并拿到 bytes
//! - Prom JSON → metric Fact 解析正确(resource_id 反查 + value)
//! - **deny-by-default**:不申明 http-client capability 时,host 拒绝网络调用,
//!   guest 收到 Unauthorized,整轮 0 fact + error note
//!
//! 测试可选:找不到 prometheus.wasm 时跳过(本地需先 `cd modules && cargo wasi-build`)。

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

fn locate_prometheus_wasm() -> Option<PathBuf> {
    let modules = modules_root();
    let text = fs::read_to_string(modules.join("manifest.toml")).ok()?;
    let parsed = engine_wasm::ManifestFile::from_toml_str(&text).ok()?;
    let m = parsed.modules.iter().find(|m| m.name == "prometheus")?;
    let wasm = modules.join(&m.wasm_path);
    wasm.exists().then_some(wasm)
}

const PROM_JSON: &str = r#"{"status":"success","data":{"resultType":"vector","result":[{"metric":{"service_name":"cartservice"},"value":[1700000000,"42.5"]},{"metric":{"service_name":"frontend"},"value":[1700000000,"7"]}]}}"#;

/// 起一个极简 mock HTTP server,对任何请求返回固定 Prom JSON。返回监听地址。
/// 线程 detached —— 进程结束自然回收。
fn spawn_mock_prom() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock prom");
    let addr = listener.local_addr().expect("local addr");
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            // 读到 header 结束即可(GET 无 body)
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                PROM_JSON.len(),
                PROM_JSON
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn prometheus_emits_metric_facts_via_http_capability() {
    let Some(wasm_path) = locate_prometheus_wasm() else {
        eprintln!("skipping: prometheus.wasm not built. Run `cd modules && cargo wasi-build`.");
        return;
    };
    let base = spawn_mock_prom();

    // http-client capability 申明 → host 放行
    let caps: HashSet<String> = ["logging", "clock", "http-client"]
        .into_iter()
        .map(String::from)
        .collect();
    let mut conn = WasmConnector::load(&wasm_path, caps)
        .await
        .expect("load prometheus.wasm");

    // 单条 service 查询(mock 忽略 query 内容,返固定两条 sample)
    let config = format!(
        r#"{{"prometheus_url":"{base}","cluster":"local","namespace":"otel-demo","queries":[{{"name":"span_p99_ms","promql":"up","unit":"ms","target":"service"}}]}}"#
    );
    let outcome = conn.sync(&config).await.expect("sync");

    assert!(outcome.errors.is_empty(), "unexpected errors: {:?}", outcome.errors);
    assert_eq!(outcome.facts.len(), 2, "two samples → two metric facts");

    let by_rid: std::collections::HashMap<&str, &_> = outcome
        .facts
        .iter()
        .map(|f| (f.resource_id.as_str(), f))
        .collect();

    let cart = by_rid
        .get("service:local:otel-demo:cartservice")
        .expect("cartservice fact");
    assert_eq!(cart.kind, "metric");
    assert_eq!(cart.source, "prometheus");
    assert_eq!(cart.resource_type, "Service");
    assert!(cart.timestamp > 1_700_000_000, "host clock stamps ts");
    let attrs: serde_json::Value = serde_json::from_str(&cart.attributes_json).expect("attrs json");
    assert_eq!(attrs["metric"], "span_p99_ms");
    assert_eq!(attrs["value"], 42.5);
    assert_eq!(attrs["unit"], "ms");
    assert_eq!(attrs["labels"]["service_name"], "cartservice");

    assert!(by_rid.contains_key("service:local:otel-demo:frontend"));
}

#[tokio::test]
async fn prometheus_without_capability_is_denied() {
    let Some(wasm_path) = locate_prometheus_wasm() else {
        eprintln!("skipping: prometheus.wasm not built.");
        return;
    };
    let base = spawn_mock_prom();

    // 不申明 http-client → deny by default
    let caps: HashSet<String> = ["logging", "clock"].into_iter().map(String::from).collect();
    let mut conn = WasmConnector::load(&wasm_path, caps)
        .await
        .expect("load prometheus.wasm");

    let config = format!(
        r#"{{"prometheus_url":"{base}","queries":[{{"name":"q","promql":"up","unit":"","target":"service"}}]}}"#
    );
    let outcome = conn.sync(&config).await.expect("sync");

    assert!(outcome.facts.is_empty(), "denied → no facts");
    assert_eq!(outcome.errors.len(), 1, "one denied-query error note");
    assert!(
        outcome.errors[0].contains("unauthorized"),
        "error should mention unauthorized: {:?}",
        outcome.errors
    );
}
