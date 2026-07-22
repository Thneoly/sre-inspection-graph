//! WasmHandlerExecutor 测试(Phase 3.9a-3b2 收尾)。
//!
//! 覆盖 executor 胶水(区别于 wasm_handler.rs 只测 `WasmHandler` 原语):
//! - `_config{api_base,real_mode,old_replicas}` 注入
//! - WASM 返字段 overlay 进 target attrs(合并,不擦 connector 字段)
//! - 据 action_id 从合并 attrs 合成 verifier 期望 result 字段
//!
//! 两个测试:
//! 1. mock(无集群):real_mode=false,scale +1,assert result.new_replicas==4
//!    + attributes_json 合并后保留 cluster/name/replicas_desired 且含 desired_replicas/available_replicas=4。
//! 2. real(proxy-gated):real_mode=true,scale +1 再 -1,覆盖完整
//!    execute->executor->WasmHandler->集群 胶水 + 可逆。
//!
//! **前置**:需 `cd modules && cargo wasi-build` 产 k8s_handler.wasm。产物不在时 skip。

#![allow(missing_docs)]

use std::collections::HashSet;
use std::sync::Arc;

use engine_identity::ResolvedNode;
use engine_recovery::{ExecutionContext, HandlerExecutor};
use engine_wasm::{WasmHandler, WasmHandlerExecutor};
use tokio::sync::Mutex;

fn k8s_handler_wasm_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../modules/target/wasm32-wasip2/release/k8s_handler.wasm")
}

fn handler_caps() -> HashSet<String> {
    let mut caps = HashSet::new();
    caps.insert("logging".to_string());
    caps.insert("clock".to_string());
    caps.insert("http-client".to_string());
    caps.insert("http-write".to_string());
    caps
}

fn proxy_ready() -> bool {
    std::net::TcpStream::connect("127.0.0.1:8001").is_ok()
}

fn node(resource_id: &str, rtype: &str, attrs: &str) -> ResolvedNode {
    ResolvedNode {
        resource_id: resource_id.into(),
        resource_type: rtype.into(),
        label: resource_id.into(),
        attributes_json: attrs.into(),
    }
}

fn ctx() -> ExecutionContext {
    ExecutionContext {
        execution_id: "exec-test".into(),
        initiated_by: "test".into(),
        auto_rollback: false,
    }
}

#[tokio::test]
async fn executor_mock_merges_attrs_and_synthesizes_result() {
    // 无集群:real_mode=false。验证 executor 三件事:
    // 1. result.new_replicas 合成自合并 attrs(desired_replicas)
    // 2. attributes_json 合并:WASM 返字段 overlay 进 target,保留 connector 字段
    // 3. _config 注入(real_mode/old_replicas 从 target attrs.replicas_desired 读)
    let wasm = k8s_handler_wasm_path();
    if !wasm.exists() {
        eprintln!(
            "skip: k8s_handler.wasm not found at {} (run `cd modules && cargo wasi-build -p k8s-handler`)",
            wasm.display()
        );
        return;
    }
    let handler = WasmHandler::load(&wasm, handler_caps())
        .await
        .expect("load WasmHandler");
    let exec = WasmHandlerExecutor::new(
        Arc::new(Mutex::new(handler)),
        "http://127.0.0.1:8001".into(),
        false, // mock
    );

    // target 带 connector 写的字段(replicas_desired=3, cluster, name)
    let target = node(
        "deploy:vm-cluster:otel-demo:otel-demo-frontend",
        "Deployment",
        r#"{"cluster":"vm-cluster","name":"otel-demo-frontend","replicas_desired":3}"#,
    );
    let params = serde_json::json!({ "replicas_delta": 1 });

    let outcome = exec
        .execute("scale_deployment", &target, &params, &ctx())
        .await;

    assert!(outcome.success, "mock scale should succeed");

    // 1. result 合成 verifier 字段:new_replicas==4(3 + delta 1)
    assert_eq!(
        outcome.result["new_replicas"].as_i64(),
        Some(4),
        "result.new_replicas should be synthesized as 4, got: {}",
        outcome.result
    );

    // 2. attributes_json 合并:含 desired_replicas/available_replicas=4,且保留 connector 字段
    let attrs = outcome
        .attributes_json
        .as_ref()
        .expect("attributes_json present");
    let parsed: serde_json::Value = serde_json::from_str(attrs).expect("merged attrs valid JSON");
    assert_eq!(parsed["desired_replicas"].as_i64(), Some(4), "desired_replicas=4 (overlay)");
    assert_eq!(parsed["available_replicas"].as_i64(), Some(4), "available_replicas=4 (overlay)");
    // connector 字段未被擦(修 bug #1)
    assert_eq!(parsed["cluster"], "vm-cluster", "cluster preserved (not wiped)");
    assert_eq!(parsed["name"], "otel-demo-frontend", "name preserved (not wiped)");
    assert_eq!(parsed["replicas_desired"].as_i64(), Some(3), "replicas_desired preserved");
}

#[tokio::test]
async fn executor_mock_kill_query_falls_back_to_mock() {
    // kill_query 非 K8s action -> 走 MockHandlerExecutor fallback(不调 WASM)。
    let wasm = k8s_handler_wasm_path();
    if !wasm.exists() {
        eprintln!("skip: k8s_handler.wasm not found");
        return;
    }
    let handler = WasmHandler::load(&wasm, handler_caps())
        .await
        .expect("load WasmHandler");
    let exec = WasmHandlerExecutor::new(Arc::new(Mutex::new(handler)), "http://127.0.0.1:8001".into(), true);

    let target = node("mysql:vm:otel-demo:accounting-db", "MySQL", r#"{}"#);
    // mock kill_query handler 需 query_id 参数(否则 err);给有效参数验 fallback 走通
    let outcome = exec
        .execute("kill_query", &target, &serde_json::json!({ "query_id": "q-42" }), &ctx())
        .await;
    assert!(outcome.success, "kill_query should fall back to mock handler");
    assert_eq!(
        outcome.result["killed_query_id"], "q-42",
        "mock handler result echoes killed_query_id"
    );
    assert!(
        outcome.attributes_json.is_none(),
        "kill_query mock produces no attrs (one-shot action)"
    );
}

#[tokio::test]
async fn executor_real_scale_pipeline_reversible() {
    // 真集群:real_mode=true。完整 execute->executor->WasmHandler->K8s 胶水 + 可逆。
    // scale +1 (1->2) -> executor 读 old_replicas=target.replicas_desired=1 -> new=2;
    // 然后 rollback -1,target attrs 更成 2 -> new=1 复原。
    let wasm = k8s_handler_wasm_path();
    if !wasm.exists() || !proxy_ready() {
        eprintln!("skip: k8s_handler.wasm or kubectl proxy 8001 not available");
        return;
    }
    let handler = WasmHandler::load(&wasm, handler_caps())
        .await
        .expect("load WasmHandler");
    let exec = WasmHandlerExecutor::new(
        Arc::new(Mutex::new(handler)),
        "http://127.0.0.1:8001".into(),
        true, // real
    );

    let target_id = "deploy:vm-cluster:otel-demo:otel-demo-frontend";
    let params_up = serde_json::json!({ "replicas_delta": 1 });

    // scale +1:target replicas_desired=1(集群当前态) -> executor 注入 old_replicas=1 -> new=2
    let target_up = node(target_id, "Deployment", r#"{"replicas_desired":1}"#);
    let outcome_up = exec
        .execute("scale_deployment", &target_up, &params_up, &ctx())
        .await;
    assert!(outcome_up.success, "real scale up should succeed");
    assert_eq!(
        outcome_up.result["new_replicas"].as_i64(),
        Some(2),
        "result.new_replicas==2 after scale up, got: {}",
        outcome_up.result
    );
    let attrs_up: serde_json::Value =
        serde_json::from_str(outcome_up.attributes_json.as_deref().unwrap_or("{}"))
            .expect("attrs valid");
    assert_eq!(attrs_up["desired_replicas"].as_i64(), Some(2));

    // 等 K8s controller 处理 PATCH
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // rollback -1:target attrs 更成 replicas_desired=2(镜像 mutated twin) -> new=1 复原
    let params_dn = serde_json::json!({ "replicas_delta": -1 });
    let target_dn = node(target_id, "Deployment", r#"{"replicas_desired":2}"#);
    let outcome_dn = exec
        .execute("scale_deployment", &target_dn, &params_dn, &ctx())
        .await;
    assert!(outcome_dn.success, "real scale down should succeed");
    assert_eq!(
        outcome_dn.result["new_replicas"].as_i64(),
        Some(1),
        "result.new_replicas==1 after rollback, got: {}",
        outcome_dn.result
    );
}
