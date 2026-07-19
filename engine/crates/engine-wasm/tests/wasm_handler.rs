//! WasmHandler 测试(Phase 3.9a-3b2a)。
//!
//! 加载 scale-deploy.wasm(handler-world export)+ execute(mock params,
//! real_mode=false)+ 验证 ExecResult{success, attributes_json}。
//!
//! **前置**:需 `cd modules && cargo wasi-build` 产 scale_deploy.wasm。产物不在时 skip。

use std::collections::HashSet;

use engine_wasm::WasmHandler;

fn scale_deploy_wasm_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../modules/target/wasm32-wasip2/release/scale_deploy.wasm")
}

fn handler_caps() -> HashSet<String> {
    let mut caps = HashSet::new();
    caps.insert("logging".to_string());
    caps.insert("clock".to_string());
    caps.insert("http-client".to_string());
    caps.insert("http-write".to_string());
    caps
}

fn k8s_handler_wasm_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../modules/target/wasm32-wasip2/release/k8s_handler.wasm")
}

fn proxy_ready() -> bool {
    std::net::TcpStream::connect("127.0.0.1:8001").is_ok()
}

#[tokio::test]
async fn handler_execute_mock_returns_attributes_json() {
    let wasm = scale_deploy_wasm_path();
    if !wasm.exists() {
        eprintln!(
            "skip: scale_deploy.wasm not found at {} (run `cd modules && cargo wasi-build -p scale-deploy`)",
            wasm.display()
        );
        return;
    }

    let mut handler = WasmHandler::load(&wasm, handler_caps())
        .await
        .expect("load WasmHandler");

    // mock params:real_mode=false,old_replicas=3,delta=1 -> new=4
    let params_json = r#"{"replicas_delta":1,"_config":{"real_mode":false,"old_replicas":3}}"#;
    let raw = handler
        .execute("scale_deployment", "deploy:vm:otel-demo:test-deploy", params_json, "test")
        .await
        .expect("call_execute");
    let ok = raw.expect("guest returned ExecError");
    assert!(ok.success, "execute should succeed in mock mode");
    assert!(
        ok.attributes_json.contains("\"desired_replicas\":4"),
        "attributes_json should contain desired_replicas=4, got: {}",
        ok.attributes_json
    );
    assert!(
        ok.attributes_json.contains("\"available_replicas\":4"),
        "attributes_json should contain available_replicas=4"
    );
}

#[tokio::test]
async fn handler_execute_rejects_invalid_target() {
    let wasm = scale_deploy_wasm_path();
    if !wasm.exists() {
        eprintln!("skip: scale_deploy.wasm not found");
        return;
    }

    let mut handler = WasmHandler::load(&wasm, handler_caps())
        .await
        .expect("load WasmHandler");

    // invalid target(not deploy:...)
    let params_json = r#"{"replicas_delta":1,"_config":{"real_mode":false,"old_replicas":3}}"#;
    let raw = handler
        .execute("scale_deployment", "pod:invalid", params_json, "test")
        .await
        .expect("call_execute");
    assert!(raw.is_err(), "guest should return ExecError for invalid target");
}

#[tokio::test]
async fn handler_execute_real_patches_scale_and_rolls_back() {
    // Phase 3.9a-3b2c:真集群验证。real_mode=true,经 http-write PATCH K8s scale。
    // 可逆:scale +1 (1->2) -> rollback -1 (2->1)。需 kubectl proxy 8001 + otel-demo。
    let wasm = scale_deploy_wasm_path();
    if !wasm.exists() {
        eprintln!("skip: scale_deploy.wasm not found");
        return;
    }
    // 检查 kubectl proxy 8001
    if std::net::TcpStream::connect("127.0.0.1:8001").is_err() {
        eprintln!("skip: kubectl proxy 8001 not running (run `kubectl proxy --port=8001`)");
        return;
    }

    let mut handler = WasmHandler::load(&wasm, handler_caps())
        .await
        .expect("load WasmHandler");
    let target = "deploy:vm-cluster:otel-demo:otel-demo-frontend";

    // scale +1 (1->2)
    let params_up = r#"{"replicas_delta":1,"_config":{"api_base":"http://127.0.0.1:8001","real_mode":true,"old_replicas":1}}"#;
    let raw_up = handler
        .execute("scale_deployment", target, params_up, "test")
        .await
        .expect("call_execute scale up");
    let ok_up = raw_up.expect("scale up returned ExecError");
    assert!(ok_up.success, "real scale up should succeed");
    assert!(
        ok_up.attributes_json.contains("\"desired_replicas\":2"),
        "attributes should show desired_replicas=2, got: {}",
        ok_up.attributes_json
    );

    // 等待 K8s controller 处理 PATCH
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // rollback -1 (2->1)
    let params_dn = r#"{"replicas_delta":-1,"_config":{"api_base":"http://127.0.0.1:8001","real_mode":true,"old_replicas":2}}"#;
    let raw_dn = handler
        .execute("scale_deployment", target, params_dn, "test")
        .await
        .expect("call_execute scale down");
    let ok_dn = raw_dn.expect("scale down returned ExecError");
    assert!(ok_dn.success, "real scale down should succeed");
    assert!(
        ok_dn.attributes_json.contains("\"desired_replicas\":1"),
        "attributes should show desired_replicas=1 (rollback), got: {}",
        ok_dn.attributes_json
    );
}

// ===== k8s-handler(6 action)tests =====

#[tokio::test]
async fn k8s_handler_mock_all_6_actions() {
    let wasm = k8s_handler_wasm_path();
    if !wasm.exists() {
        eprintln!("skip: k8s_handler.wasm not found");
        return;
    }
    let mut handler = WasmHandler::load(&wasm, handler_caps())
        .await
        .expect("load WasmHandler");

    // mock 模式:6 action 都 success(不调 http)
    let cases = [
        ("scale_deployment", "deploy:vm:otel-demo:test", r#"{"replicas_delta":1,"_config":{"real_mode":false,"old_replicas":3}}"#),
        ("restart_pod", "pod:vm:otel-demo:test-pod", r#"{"_config":{"real_mode":false}}"#),
        ("rollback_deployment", "deploy:vm:otel-demo:test", r#"{"_config":{"real_mode":false}}"#),
        ("refresh_secret", "secret:vm:otel-demo:test-secret", r#"{"_config":{"real_mode":false}}"#),
        ("drain_node", "node:vm:vm1", r#"{"_config":{"real_mode":false}}"#),
        ("restart_service", "service:vm:otel-demo:test-svc", r#"{"_config":{"real_mode":false}}"#),
    ];
    for (action, target, params) in &cases {
        let raw = handler.execute(action, target, params, "test").await.expect("call");
        let ok = raw.unwrap_or_else(|_| panic!("{action} returned ExecError"));
        assert!(ok.success, "mock {action} should succeed");
    }
}

#[tokio::test]
async fn k8s_handler_real_drain_node_and_uncordon() {
    let wasm = k8s_handler_wasm_path();
    if !wasm.exists() || !proxy_ready() {
        eprintln!("skip: k8s_handler.wasm or proxy not available");
        return;
    }
    let mut handler = WasmHandler::load(&wasm, handler_caps())
        .await
        .expect("load WasmHandler");

    // drain_node(cordon vm1)
    let params = r#"{"_config":{"api_base":"http://127.0.0.1:8001","real_mode":true}}"#;
    let raw = handler
        .execute("drain_node", "node:vm-cluster:vm1", params, "test")
        .await
        .expect("call drain_node");
    let ok = raw.expect("drain_node ExecError");
    assert!(ok.success, "real drain_node should succeed");

    // uncordon(恢复:PATCH unschedulable=false)
    let req = reqwest::Client::new();
    let _ = req
        .patch("http://127.0.0.1:8001/api/v1/nodes/vm1")
        .header("content-type", "application/merge-patch+json")
        .body(r#"{"spec":{"unschedulable":false}}"#)
        .send()
        .await;
}

#[tokio::test]
async fn k8s_handler_real_restart_service() {
    let wasm = k8s_handler_wasm_path();
    if !wasm.exists() || !proxy_ready() {
        eprintln!("skip: k8s_handler.wasm or proxy not available");
        return;
    }
    let mut handler = WasmHandler::load(&wasm, handler_caps())
        .await
        .expect("load WasmHandler");

    // restart_service(DELETE endpoints,控制器重建)
    let params = r#"{"_config":{"api_base":"http://127.0.0.1:8001","real_mode":true}}"#;
    let raw = handler
        .execute("restart_service", "service:vm-cluster:otel-demo:otel-demo-frontend", params, "test")
        .await
        .expect("call restart_service");
    let ok = raw.expect("restart_service ExecError");
    assert!(ok.success, "real restart_service should succeed");

    // 等待控制器重建 endpoints
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
}
