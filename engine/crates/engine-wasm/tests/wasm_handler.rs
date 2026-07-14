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
