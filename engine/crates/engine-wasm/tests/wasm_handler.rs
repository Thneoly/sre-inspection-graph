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
        .join("../../modules/target/wasm32-wasip2/release/scale_deploy.wasm")
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
