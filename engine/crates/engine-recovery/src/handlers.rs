//! 8 个 RecoveryAction mock handler(复刻 `reference/app/recovery/handlers/*.py`)。
//!
//! 每个 handler 是纯函数 `fn(&mut ResolvedNode, &Value params, &ExecutionContext) -> Value`,
//! 返回 flat result dict(`{"success": bool, "error"?: str, ...action-specific}`)。
//!
//! ## mock 双向:mutate twin + 返 result
//!
//! reference mock handler 改 DSS 节点 properties(供 verifier 读)+ 返 result dict。本 port
//! 对齐:handler 接 `&mut ResolvedNode`,把动作生效后的属性写回 `attributes_json`(模拟
//! 生效),**同时**返 result dict。这样 3.3 verifier 读 mutated attrs(faithful),rollback
//! 反向 handler 读 post-action 状态(正确反转,非重应用到原状态)。
//!
//! ## 与 reference 的差异
//!
//! - **只 mock 模式**:reference mock/real 双模式;本 port 3.2/3.3 仅 mock(real 待 write-
//!   capability WIT)。mock 不调真实 K8s/MySQL/Redis。
//! - **twin 即入参 `&mut ResolvedNode`**:reference 改全局 DSS store;本 port 改调用方传入的
//!   topology twin(orchestration 3.6 应传 materialized topology 的 clone,避免污染真相源)。
//! - handler 校验 target 类型 + 参数,违例返 `{success:false,error}`(不抛,对齐 reference)。
//! - result 字段含 verifier 期望:new_replicas / new_restart_count / new_revision / new_version /
//!   endpoints_refresh_count / cordoned。

#![allow(missing_docs)]

use engine_identity::ResolvedNode;
use serde_json::{json, Value};

use crate::models::ExecutionContext;

/// handler 函数指针类型(取 `&mut ResolvedNode` 以 mutate twin)。
pub type HandlerFn = fn(&mut ResolvedNode, &Value, &ExecutionContext) -> Value;

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

/// 解析 + mutate + 写回 `attributes_json`(模拟动作生效)。
fn with_attrs_mut(node: &mut ResolvedNode, f: impl FnOnce(&mut serde_json::Map<String, Value>)) {
    let mut m = attrs(node);
    f(&mut m);
    node.attributes_json = Value::Object(m).to_string();
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

// ===== 8 handlers(mock + mutate twin)=====

fn scale_deployment(target: &mut ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> Value {
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
    // mutate twin:desired = available = new(对齐 reference mock _execute_mock)
    with_attrs_mut(target, |m| {
        m.insert("desired_replicas".into(), json!(new));
        m.insert("available_replicas".into(), json!(new));
    });
    json!({
        "success": true,
        "old_replicas": old,
        "new_replicas": new,
        "delta_applied": delta,
        "note": format!("Deployment scaled from {old} to {new} replicas (mock execution)"),
    })
}

fn restart_pod(target: &mut ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> Value {
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
    with_attrs_mut(target, |m| {
        m.insert("restart_count".into(), json!(new));
        // warning -> normal(对齐 reference _apply_dss)
        if m.get("health_status").and_then(Value::as_str) == Some("warning") {
            m.insert("health_status".into(), json!("normal"));
        }
    });
    json!({
        "success": true,
        "old_restart_count": old,
        "new_restart_count": new,
        "graceful": graceful,
        "grace_period_seconds": grace_period,
        "note": format!("Pod restarted (mock execution, count={new})"),
    })
}

fn rollback_deployment(target: &mut ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> Value {
    if target.resource_type != "Deployment" {
        return err(format!("target is {}, not Deployment", target.resource_type));
    }
    let a = attrs(target);
    let old = attr_i64(&a, "current_revision", 1);
    let new = params
        .get("revision")
        .and_then(Value::as_i64)
        .unwrap_or_else(|| (old - 1).max(1));
    if new < 1 {
        return err(format!("revision must be >= 1 (got {new})"));
    }
    with_attrs_mut(target, |m| {
        m.insert("current_revision".into(), json!(new));
    });
    json!({
        "success": true,
        "old_revision": old,
        "new_revision": new,
        "note": format!("Deployment rolled back from revision {old} to {new} (mock execution)"),
    })
}

fn refresh_secret(target: &mut ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> Value {
    if target.resource_type != "Secret" {
        return err(format!("target is {}, not Secret", target.resource_type));
    }
    let trigger = param_bool(params, "trigger_pod_restart", true);
    let a = attrs(target);
    let old = attr_i64(&a, "secret_version", 1);
    let new = old + 1;
    with_attrs_mut(target, |m| {
        m.insert("secret_version".into(), json!(new));
    });
    json!({
        "success": true,
        "old_version": old,
        "new_version": new,
        "trigger_pod_restart": trigger,
        "note": format!("Secret refreshed from version {old} to {new} (mock execution)"),
    })
}

fn drain_node(target: &mut ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> Value {
    if target.resource_type != "KubernetesNode" {
        return err(format!("target is {}, not KubernetesNode", target.resource_type));
    }
    let ignore_daemonsets = param_bool(params, "ignore_daemonsets", true);
    let delete_local_data = param_bool(params, "delete_local_data", false);
    let force = param_bool(params, "force", false);
    with_attrs_mut(target, |m| {
        m.insert("cordoned".into(), json!(true));
    });
    json!({
        "success": true,
        "cordoned": true,
        "ignore_daemonsets": ignore_daemonsets,
        "delete_local_data": delete_local_data,
        "force": force,
        "note": "Node cordoned + pods marked for eviction (mock execution; real evict deferred)",
    })
}

fn kill_query(target: &mut ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> Value {
    if target.resource_type != "MySQL" {
        return err(format!("target is {}, not MySQL", target.resource_type));
    }
    let query_id = match params.get("query_id").and_then(Value::as_str) {
        Some(q) if !q.is_empty() => q.to_string(),
        _ => return err("query_id is required"),
    };
    let min_duration = param_i64(params, "min_duration_seconds", 30);
    // 一次性动作,无持续副作用可 mutate(对齐 reference verify_kill_query not_supported)
    json!({
        "success": true,
        "killed_query_id": query_id,
        "min_duration_seconds": min_duration,
        "note": "MySQL query killed (mock execution)",
    })
}

fn restart_service(target: &mut ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> Value {
    if target.resource_type != "Service" {
        return err(format!("target is {}, not Service", target.resource_type));
    }
    let drop_idle = param_i64(params, "drop_idle_seconds", 0);
    let a = attrs(target);
    let old = attr_i64(&a, "endpoints_refresh_count", 0);
    let new = old + 1;
    with_attrs_mut(target, |m| {
        m.insert("endpoints_refresh_count".into(), json!(new));
    });
    json!({
        "success": true,
        "endpoints_regenerated": true,
        "endpoints_refresh_count": new,
        "drop_idle_seconds": drop_idle,
        "note": "Service endpoints regenerated (mock execution)",
    })
}

fn clear_cache(target: &mut ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> Value {
    if target.resource_type != "Redis" {
        return err(format!("target is {}, not Redis", target.resource_type));
    }
    let scope = param_str(params, "scope", "pattern");
    let db_index = param_i64(params, "db_index", 0);
    let key_pattern = param_str(params, "key_pattern", "");
    // 一次性动作,无持续副作用(对齐 reference verify_clear_cache not_supported)
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
    /// 读 mutate 后的 attr。
    fn attr_after(node: &ResolvedNode, key: &str) -> Option<Value> {
        attrs(node).get(key).cloned()
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
    fn scale_deployment_mutates_and_validates() {
        let mut deploy = node("deploy:a", "Deployment", r#"{"desired_replicas":3}"#);
        let r = scale_deployment(&mut deploy, &json!({"replicas_delta":2}), &ctx());
        assert_eq!(r["success"], true);
        assert_eq!(r["old_replicas"], 3);
        assert_eq!(r["new_replicas"], 5);
        // twin 被 mutate
        assert_eq!(attr_after(&deploy, "desired_replicas"), Some(json!(5)));
        assert_eq!(attr_after(&deploy, "available_replicas"), Some(json!(5)));
        // delta=0 -> error,twin 不动
        let mut d2 = node("deploy:a", "Deployment", r#"{"desired_replicas":3}"#);
        let r0 = scale_deployment(&mut d2, &json!({"replicas_delta":0}), &ctx());
        assert_eq!(r0["success"], false);
        assert_eq!(attr_after(&d2, "desired_replicas"), Some(json!(3))); // 未 mutate
        // 类型不匹配
        let mut pod = node("pod:a", "Pod", "{}");
        let rt = scale_deployment(&mut pod, &json!({"replicas_delta":1}), &ctx());
        assert_eq!(rt["success"], false);
    }

    #[test]
    fn restart_pod_mutates_count_and_clears_warning() {
        let mut pod = node("pod:a", "Pod", r#"{"restart_count":2,"health_status":"warning"}"#);
        let r = restart_pod(&mut pod, &json!({}), &ctx());
        assert_eq!(r["success"], true);
        assert_eq!(r["new_restart_count"], 3);
        assert_eq!(attr_after(&pod, "restart_count"), Some(json!(3)));
        assert_eq!(attr_after(&pod, "health_status"), Some(json!("normal"))); // warning -> normal
    }

    #[test]
    fn kill_query_no_mutation() {
        let mut mysql = node("mysql:a", "MySQL", "{}");
        let r = kill_query(&mut mysql, &json!({"query_id":"q-42"}), &ctx());
        assert_eq!(r["success"], true);
        assert_eq!(r["killed_query_id"], "q-42");
        // 无持续副作用 -> attributes 不变(仍空 object)
        assert!(attrs(&mysql).is_empty());
    }

    #[test]
    fn drain_node_cordons() {
        let mut n = node("node:a", "KubernetesNode", "{}");
        let r = drain_node(&mut n, &json!({}), &ctx());
        assert_eq!(r["success"], true);
        assert_eq!(r["cordoned"], true);
        assert_eq!(attr_after(&n, "cordoned"), Some(json!(true)));
    }

    #[test]
    fn restart_service_increments_refresh_count() {
        let mut s = node("svc:a", "Service", r#"{"endpoints_refresh_count":4}"#);
        let r = restart_service(&mut s, &json!({}), &ctx());
        assert_eq!(r["success"], true);
        assert_eq!(r["endpoints_refresh_count"], 5);
        assert_eq!(attr_after(&s, "endpoints_refresh_count"), Some(json!(5)));
    }
}
