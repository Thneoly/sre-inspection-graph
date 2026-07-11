//! 8 个 RecoveryAction mock handler(复刻 `reference/app/recovery/handlers/*.py`)。
//!
//! 每个 handler 是纯函数 `fn(&ResolvedNode, &Value params, &ExecutionContext) -> Value`,
//! 返回 flat result dict(`{"success": bool, "error"?: str, ...action-specific}`),
//! 对齐 reference handler 返回形状。
//!
//! ## 与 reference 的差异
//!
//! - **只 mock 模式**:reference 有 mock/real 双模式(`RECOVERY_HANDLER_MODE`);本 port
//!   3.2 仅 mock(real handler 待 write-capability WIT,延后)。mock 不调真实 K8s/MySQL/Redis。
//! - **不 mutate DSS 孪生**:reference mock 改 `store` 节点 properties(供 verifier 读);
//!   本 port handler 纯返 result dict,不动 topology(topology 是 connector 的只读快照)。
//!   3.3 verifier 据本 result dict 判(非读 DSS props)。old/new 值从 target `attributes_json`
//!   读默认值(desired_replicas=3 / restart_count=0 等),与 reference 默认一致。
//! - handler 校验 target 类型 + 参数,前置违例返 `{success:false, error}`(不抛异常,
//!   对齐 reference「handler 内部失败不抛」)。

#![allow(missing_docs)]

use engine_identity::ResolvedNode;
use serde_json::{json, Value};

use crate::models::ExecutionContext;

/// handler 函数指针类型。
pub type HandlerFn = fn(&ResolvedNode, &Value, &ExecutionContext) -> Value;

/// handler 注册表(action_id -> fn)。
pub static HANDLERS: &[(&str, HandlerFn)] = &[
    ("scale_deployment", scale_deployment),
    ("kill_query", kill_query),
    ("restart_service", restart_service),
    ("restart_pod", restart_pod),
    ("rollback_deployment", rollback_deployment),
    ("refresh_secret", refresh_secret),
    ("drain_node", drain_node),
    ("clear_cache", clear_cache),
];

/// 取 handler;未实现返 None。
pub fn get_handler(action_id: &str) -> Option<HandlerFn> {
    HANDLERS
        .iter()
        .find(|(id, _)| *id == action_id)
        .map(|(_, f)| *f)
}

/// 动作是否已实现执行(8 种均 true)。
pub fn is_executable(action_id: &str) -> bool {
    HANDLERS.iter().any(|(id, _)| *id == action_id)
}

// ===== helpers =====

/// 解 target `attributes_json` 成 object;非法返空 map。
fn attrs(node: &ResolvedNode) -> serde_json::Map<String, Value> {
    match serde_json::from_str::<Value>(&node.attributes_json) {
        Ok(Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    }
}

fn attr_i64(attrs: &serde_json::Map<String, Value>, key: &str, default: i64) -> i64 {
    attrs.get(key).and_then(Value::as_i64).unwrap_or(default)
}

fn param_i64(params: &Value, key: &str, default: i64) -> i64 {
    params.get(key).and_then(Value::as_i64).unwrap_or(default)
}

fn param_bool(params: &Value, key: &str, default: bool) -> bool {
    params.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn param_str(params: &Value, key: &str, default: &str) -> String {
    params
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_string()
}

fn err(msg: impl Into<String>) -> Value {
    json!({ "success": false, "error": msg.into() })
}

// ===== 8 handlers(mock)=====

fn scale_deployment(target: &ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> Value {
    if target.resource_type != "Deployment" {
        return err(format!("target is {}, not Deployment", target.resource_type));
    }
    let delta = match params.get("replicas_delta").and_then(Value::as_i64) {
        Some(d) => d,
        None => return err("replicas_delta must be non-zero"),
    };
    if delta == 0 {
        return err("replicas_delta must be non-zero");
    }
    let a = attrs(target);
    let old = attr_i64(&a, "desired_replicas", 3);
    let new = old + delta;
    if new < 0 {
        return err(format!("new replicas would be negative ({new})"));
    }
    if new > 100 {
        return err(format!("new replicas exceeds limit ({new} > 100)"));
    }
    json!({
        "success": true,
        "old_replicas": old,
        "new_replicas": new,
        "delta_applied": delta,
        "note": format!("Deployment scaled from {old} to {new} replicas (mock execution)"),
    })
}

fn restart_pod(target: &ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> Value {
    if target.resource_type != "Pod" {
        return err(format!("target is {}, not Pod", target.resource_type));
    }
    let graceful = param_bool(params, "graceful", true);
    let grace_period = param_i64(params, "grace_period_seconds", 30);
    if !(0..=300).contains(&grace_period) {
        return err(format!("grace_period_seconds out of range: {grace_period}"));
    }
    let a = attrs(target);
    let old = attr_i64(&a, "restart_count", 0);
    let new = old + 1;
    json!({
        "success": true,
        "old_restart_count": old,
        "new_restart_count": new,
        "graceful": graceful,
        "grace_period_seconds": grace_period,
        "note": format!("Pod restarted (mock execution, count={new})"),
    })
}

fn rollback_deployment(target: &ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> Value {
    if target.resource_type != "Deployment" {
        return err(format!("target is {}, not Deployment", target.resource_type));
    }
    let a = attrs(target);
    let old = attr_i64(&a, "current_revision", 1);
    // revision 显式给则用,否则回退到上一版(最少 1)
    let new = params
        .get("revision")
        .and_then(Value::as_i64)
        .unwrap_or_else(|| (old - 1).max(1));
    if new < 1 {
        return err(format!("revision must be >= 1 (got {new})"));
    }
    json!({
        "success": true,
        "old_revision": old,
        "new_revision": new,
        "note": format!("Deployment rolled back from revision {old} to {new} (mock execution)"),
    })
}

fn refresh_secret(target: &ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> Value {
    if target.resource_type != "Secret" {
        return err(format!("target is {}, not Secret", target.resource_type));
    }
    let trigger = param_bool(params, "trigger_pod_restart", true);
    let a = attrs(target);
    let old = attr_i64(&a, "secret_version", 1);
    let new = old + 1;
    json!({
        "success": true,
        "old_version": old,
        "new_version": new,
        "trigger_pod_restart": trigger,
        "note": format!("Secret refreshed from version {old} to {new} (mock execution)"),
    })
}

fn drain_node(target: &ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> Value {
    if target.resource_type != "KubernetesNode" {
        return err(format!("target is {}, not KubernetesNode", target.resource_type));
    }
    let ignore_daemonsets = param_bool(params, "ignore_daemonsets", true);
    let delete_local_data = param_bool(params, "delete_local_data", false);
    let force = param_bool(params, "force", false);
    json!({
        "success": true,
        "cordoned": true,
        "ignore_daemonsets": ignore_daemonsets,
        "delete_local_data": delete_local_data,
        "force": force,
        "note": "Node cordoned + pods marked for eviction (mock execution; real evict deferred)",
    })
}

fn kill_query(target: &ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> Value {
    if target.resource_type != "MySQL" {
        return err(format!("target is {}, not MySQL", target.resource_type));
    }
    let query_id = match params.get("query_id").and_then(Value::as_str) {
        Some(q) if !q.is_empty() => q.to_string(),
        _ => return err("query_id is required"),
    };
    let min_duration = param_i64(params, "min_duration_seconds", 30);
    json!({
        "success": true,
        "killed_query_id": query_id,
        "min_duration_seconds": min_duration,
        "note": "MySQL query killed (mock execution)",
    })
}

fn restart_service(target: &ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> Value {
    if target.resource_type != "Service" {
        return err(format!("target is {}, not Service", target.resource_type));
    }
    let drop_idle = param_i64(params, "drop_idle_seconds", 0);
    json!({
        "success": true,
        "endpoints_regenerated": true,
        "drop_idle_seconds": drop_idle,
        "note": "Service endpoints regenerated (mock execution)",
    })
}

fn clear_cache(target: &ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> Value {
    if target.resource_type != "Redis" {
        return err(format!("target is {}, not Redis", target.resource_type));
    }
    let scope = param_str(params, "scope", "pattern");
    let db_index = param_i64(params, "db_index", 0);
    let key_pattern = param_str(params, "key_pattern", "");
    json!({
        "success": true,
        "scope": scope,
        "db_index": db_index,
        "key_pattern": key_pattern,
        "cleared": true,
        "note": "Redis cache cleared (mock execution)",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_identity::ResolvedNode;

    fn node(rid: &str, rtype: &str, attrs: &str) -> ResolvedNode {
        ResolvedNode {
            resource_id: rid.into(),
            resource_type: rtype.into(),
            label: rid.into(),
            attributes_json: attrs.into(),
        }
    }
    fn ctx() -> ExecutionContext {
        ExecutionContext {
            execution_id: "e1".into(),
            initiated_by: "tester".into(),
            auto_rollback: false,
        }
    }

    #[test]
    fn eight_handlers_registered() {
        assert_eq!(HANDLERS.len(), 8);
        for id in [
            "scale_deployment",
            "kill_query",
            "restart_service",
            "restart_pod",
            "rollback_deployment",
            "refresh_secret",
            "drain_node",
            "clear_cache",
        ] {
            assert!(is_executable(id), "{id} should be executable");
            assert!(get_handler(id).is_some());
        }
        assert!(!is_executable("nonexistent"));
    }

    #[test]
    fn scale_deployment_success_and_validations() {
        let deploy = node("deploy:a", "Deployment", r#"{"desired_replicas":3}"#);
        // +2 -> 5
        let r = scale_deployment(&deploy, &json!({"replicas_delta":2}), &ctx());
        assert_eq!(r["success"], true);
        assert_eq!(r["old_replicas"], 3);
        assert_eq!(r["new_replicas"], 5);
        // delta=0 -> error
        let r0 = scale_deployment(&deploy, &json!({"replicas_delta":0}), &ctx());
        assert_eq!(r0["success"], false);
        // 缺 delta -> error
        let rm = scale_deployment(&deploy, &json!({}), &ctx());
        assert_eq!(rm["success"], false);
        // 类型不匹配
        let pod = node("pod:a", "Pod", "{}");
        let rt = scale_deployment(&pod, &json!({"replicas_delta":1}), &ctx());
        assert_eq!(rt["success"], false);
    }

    #[test]
    fn scale_deployment_defaults_desired_replicas_3() {
        // attributes 无 desired_replicas -> 默认 3
        let deploy = node("deploy:a", "Deployment", "{}");
        let r = scale_deployment(&deploy, &json!({"replicas_delta":1}), &ctx());
        assert_eq!(r["old_replicas"], 3);
        assert_eq!(r["new_replicas"], 4);
    }

    #[test]
    fn restart_pod_increments_restart_count() {
        let pod = node("pod:a", "Pod", r#"{"restart_count":2}"#);
        let r = restart_pod(&pod, &json!({}), &ctx());
        assert_eq!(r["success"], true);
        assert_eq!(r["old_restart_count"], 2);
        assert_eq!(r["new_restart_count"], 3);
        assert_eq!(r["graceful"], true); // default
    }

    #[test]
    fn kill_query_requires_query_id() {
        let mysql = node("mysql:a", "MySQL", "{}");
        let r = kill_query(&mysql, &json!({}), &ctx());
        assert_eq!(r["success"], false);
        let r2 = kill_query(&mysql, &json!({"query_id":"q-42"}), &ctx());
        assert_eq!(r2["success"], true);
        assert_eq!(r2["killed_query_id"], "q-42");
    }

    #[test]
    fn drain_node_cordons() {
        let n = node("node:a", "KubernetesNode", "{}");
        let r = drain_node(&n, &json!({}), &ctx());
        assert_eq!(r["success"], true);
        assert_eq!(r["cordoned"], true);
    }
}
