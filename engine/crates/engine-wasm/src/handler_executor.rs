//! WasmHandlerExecutor - impl async HandlerExecutor,调 WasmHandler(Phase 3.9a-3b2)。
//!
//! 6 k8s action 走 WasmHandler(scale_deployment/restart_pod/rollback_deployment/
//! refresh_secret/drain_node/restart_service);kill_query/clear_cache 留 Mock(无 K8s
//! 副作)。real_mode 经 http-client write 真改集群;mock 只返 attrs。
//!
//! ## 3.9a-3b2 修三个 bug(对齐 verifier + twin 语义)
//!
//! 1. **attrs 合并**:k8s-handler 只返 `{desired_replicas, available_replicas}` 等动作
//!    生效字段;若直接当 `HandlerOutcome.attributes_json` 返,`run_handler` 整体替换
//!    twin attrs 会擦掉 connector 写的 cluster/ns/name/replicas_desired/...。故 host
//!    读 target 现有 attrs,overlay WASM 返字段,返**合并后全量**(对齐 mock handler
//!    的 read-modify 语义)。
//! 2. **result 合成 verifier 字段**:verifier 从 `exec_result` 读 `new_replicas`/
//!    `new_restart_count`/`new_version`/`new_revision`/`endpoints_refresh_count`,但
//!    WASM 只把这些放 attributes_json。host 据 action_id 从合并 attrs 合成对应 result
//!    字段(否则 WASM 模式下 verifier 全 fail -> 误触 auto-rollback)。
//! 3. **old_replicas 名字**:connector twin 写 `replicas_desired`(不是
//!    `desired_replicas`);读 old_replicas 优先 `replicas_desired`,回退
//!    `desired_replicas`(handler 写的,回滚读 mutated twin 时命中),再回退 3,避免
//!    首次 scale 基准错。

#![allow(missing_docs)]

use std::sync::Arc;

use async_trait::async_trait;
use engine_identity::ResolvedNode;
use engine_recovery::{ExecutionContext, HandlerExecutor, HandlerOutcome, MockHandlerExecutor};
use serde_json::{json, Map, Value};
use tokio::sync::Mutex;

use crate::WasmHandler;

/// 6 个 k8s action 走 WasmHandler;kill_query/clear_cache 留 Mock。
const K8S_ACTIONS: &[&str] = &[
    "scale_deployment",
    "restart_pod",
    "rollback_deployment",
    "refresh_secret",
    "drain_node",
    "restart_service",
];

/// WASM handler 执行器:6 k8s action 走 WasmHandler,其他走 MockHandlerExecutor。
pub struct WasmHandlerExecutor {
    handler: Arc<Mutex<WasmHandler>>,
    mock: MockHandlerExecutor,
    api_base: String,
    real_mode: bool,
}

impl WasmHandlerExecutor {
    /// 构造。`handler` 是 `WasmRuntime.handlers` 里 k8s-handler 的 `Arc<Mutex<WasmHandler>>`。
    pub fn new(handler: Arc<Mutex<WasmHandler>>, api_base: String, real_mode: bool) -> Self {
        Self { handler, mock: MockHandlerExecutor, api_base, real_mode }
    }
}

#[async_trait]
impl HandlerExecutor for WasmHandlerExecutor {
    async fn execute(
        &self,
        action_id: &str,
        target: &ResolvedNode,
        params: &Value,
        ctx: &ExecutionContext,
    ) -> HandlerOutcome {
        // kill_query/clear_cache 非 k8s -> Mock fallback
        if !K8S_ACTIONS.contains(&action_id) {
            return self.mock.execute(action_id, target, params, ctx).await;
        }

        let target_attrs: Value =
            serde_json::from_str(&target.attributes_json).unwrap_or(json!({}));
        // old_replicas:connector 写 replicas_desired;回滚读 mutated twin 时 handler 写过 desired_replicas
        let old_replicas = target_attrs
            .get("replicas_desired")
            .and_then(Value::as_i64)
            .or_else(|| target_attrs.get("desired_replicas").and_then(Value::as_i64))
            .unwrap_or(3);
        let config = json!({
            "api_base": self.api_base,
            "real_mode": self.real_mode,
            "old_replicas": old_replicas,
        });
        let mut params_with_config = params.clone();
        if let Value::Object(m) = &mut params_with_config {
            m.insert("_config".to_string(), config);
        }
        let params_json = params_with_config.to_string();

        let mut handler = self.handler.lock().await;
        let wasm = match handler
            .execute(action_id, &target.resource_id, &params_json, &ctx.initiated_by)
            .await
        {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => return HandlerOutcome::err(e.0),
            Err(e) => return HandlerOutcome::err(format!("wasm execute failed: {e}")),
        };

        // 合并 attrs(WASM 返字段 overlay 进 target 现有 attrs,不擦 connector 字段)
        let wasm_attrs: Value = serde_json::from_str(&wasm.attributes_json).unwrap_or(json!({}));
        let merged = merge_values(target_attrs, wasm_attrs);
        // 合成 verifier 期望的 result 字段(从合并 attrs)
        let mut result = json!({ "success": wasm.success, "message": wasm.message });
        if let Value::Object(m) = &mut result {
            if let Some(Value::Object(fm)) = synthesize_result_field(action_id, &merged) {
                for (k, v) in fm {
                    m.insert(k, v);
                }
            }
        }
        HandlerOutcome::ok(result, Some(merged.to_string()))
    }
}

/// `overlay` 的字段盖到 `base` 上(同 key 以 overlay 为准),返合并 Value。
fn merge_values(base: Value, overlay: Value) -> Value {
    let mut m = match base {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    if let Value::Object(o) = overlay {
        for (k, v) in o {
            m.insert(k, v);
        }
    }
    Value::Object(m)
}

/// 据 action_id 从合并 attrs 合成 verifier 期望的 result 字段(单字段 object)。
/// drain_node verifier 只读 attrs.cordoned 无 result 字段;kill_query/clear_cache
/// Mock fallback 不走这里 -> None。
fn synthesize_result_field(action_id: &str, merged_attrs: &Value) -> Option<Value> {
    let pick = |k: &str| merged_attrs.get(k).cloned();
    match action_id {
        "scale_deployment" => pick("desired_replicas").map(|v| json!({ "new_replicas": v })),
        "restart_pod" => pick("restart_count").map(|v| json!({ "new_restart_count": v })),
        "restart_service" => pick("endpoints_refresh_count").map(|v| json!({ "endpoints_refresh_count": v })),
        "refresh_secret" => pick("secret_version").map(|v| json!({ "new_version": v })),
        "rollback_deployment" => pick("current_revision").map(|v| json!({ "new_revision": v })),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_values_overlays_and_preserves() {
        let base = json!({"cluster":"vm","name":"frontend","replicas_desired":3});
        let overlay = json!({"desired_replicas":4,"available_replicas":4});
        let merged = merge_values(base, overlay);
        assert_eq!(merged["cluster"], "vm");       // 保留
        assert_eq!(merged["name"], "frontend");    // 保留
        assert_eq!(merged["replicas_desired"], 3); // 保留(connector 名)
        assert_eq!(merged["desired_replicas"], 4); // overlay
        assert_eq!(merged["available_replicas"], 4);
    }

    #[test]
    fn merge_values_handles_invalid_base() {
        let merged = merge_values(json!("not an object"), json!({"desired_replicas":4}));
        assert_eq!(merged["desired_replicas"], 4);
    }

    #[test]
    fn synthesize_result_field_maps_each_action() {
        let attrs = json!({
            "desired_replicas": 4, "restart_count": 1, "endpoints_refresh_count": 1,
            "secret_version": 2, "current_revision": 1
        });
        assert_eq!(synthesize_result_field("scale_deployment", &attrs), Some(json!({"new_replicas":4})));
        assert_eq!(synthesize_result_field("restart_pod", &attrs), Some(json!({"new_restart_count":1})));
        assert_eq!(synthesize_result_field("restart_service", &attrs), Some(json!({"endpoints_refresh_count":1})));
        assert_eq!(synthesize_result_field("refresh_secret", &attrs), Some(json!({"new_version":2})));
        assert_eq!(synthesize_result_field("rollback_deployment", &attrs), Some(json!({"new_revision":1})));
        assert_eq!(synthesize_result_field("drain_node", &attrs), None);
        assert_eq!(synthesize_result_field("kill_query", &attrs), None);
        assert_eq!(synthesize_result_field("unknown", &attrs), None);
    }

    #[test]
    fn synthesize_result_field_none_when_attr_missing() {
        // scale 但 attrs 无 desired_replicas -> None(不合成,verifier 会报 missing)
        assert_eq!(synthesize_result_field("scale_deployment", &json!({})), None);
    }
}
