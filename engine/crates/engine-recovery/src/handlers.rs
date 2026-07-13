//! 8 个 RecoveryAction mock handler(复刻 `reference/app/recovery/handlers/*.py`)。
//!
//! Phase 3.9a-2:handler 签名从 `fn(&mut ResolvedNode, ...) -> Value`(mutate twin)改成
//! `fn(&ResolvedNode, ...) -> HandlerOutcome`(不 mutate,返 `attributes_json`)。`run_handler`
//! 据 `HandlerOutcome.attributes_json` 更新 twin。这样 handler 经 `HandlerExecutor` trait
//! 注入(3.9a-3 `WasmHandlerExecutor` 真改集群,3.9a-2 `MockHandlerExecutor` 调本文件)。
//!
//! ## mock 计算 attrs(不 mutate)
//!
//! reference mock handler 改 DSS 节点 properties(供 verifier 读)+ 返 result dict。本 port
//! 对齐:handler 接 `&ResolvedNode`(不可变),计算动作生效后的新 attrs,塞进
//! [`HandlerOutcome::attributes_json`] 返回(不直接 mutate)。`run_handler` 据此更新 twin。
//!
//! ## 与 reference 的差异
//!
//! - **只 mock 模式**:reference mock/real 双模式;本 port 3.2/3.3 仅 mock(real 3.9a-3 WASM handler)。
//! - **handler 不 mutate**:返 [`HandlerOutcome`] { attributes_json },`run_handler` 更新 twin。
//! - handler 校验 target 类型 + 参数,违例返 [`HandlerOutcome::err`](不抛,对齐 reference)。
//! - result 字段含 verifier 期望:new_replicas / new_restart_count / new_revision / new_version /
//!   endpoints_refresh_count / cordoned。

#![allow(missing_docs)]

use engine_identity::ResolvedNode;
use serde_json::{json, Value};

use crate::executor::HandlerOutcome;
use crate::models::ExecutionContext;

/// handler 函数指针类型(取 `&ResolvedNode` 计算 attrs,返 [`HandlerOutcome`])。
pub type HandlerFn = fn(&ResolvedNode, &Value, &ExecutionContext) -> HandlerOutcome;

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

/// 基于当前 attrs 计算 mutated attrs,序列化成 JSON 字符串返(供 [`HandlerOutcome::attributes_json`])。
fn with_attrs(node: &ResolvedNode, f: impl FnOnce(&mut serde_json::Map<String, Value>)) -> Option<String> {
    let mut m = attrs(node);
    f(&mut m);
    Some(Value::Object(m).to_string())
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

// ===== 8 handlers(mock,计算 attrs 返 HandlerOutcome)=====

fn scale_deployment(target: &ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> HandlerOutcome {
    if target.resource_type != "Deployment" {
        return HandlerOutcome::err(format!("target is {}, not Deployment", target.resource_type));
    }
    let delta = match params.get("replicas_delta").and_then(Value::as_i64) {
        Some(d) => d,
        None => return HandlerOutcome::err("replicas_delta must be non-zero"),
    };
    if delta == 0 {
        return HandlerOutcome::err("replicas_delta must be non-zero");
    }
    let a = attrs(target);
    let old = attr_i64(&a, "desired_replicas", 3);
    let new = old + delta;
    if new < 0 {
        return HandlerOutcome::err(format!("new replicas would be negative ({new})"));
    }
    if new > 100 {
        return HandlerOutcome::err(format!("new replicas exceeds limit ({new} > 100)"));
    }
    let attributes_json = with_attrs(target, |m| {
        m.insert("desired_replicas".into(), json!(new));
        m.insert("available_replicas".into(), json!(new));
    });
    HandlerOutcome::ok(
        json!({
            "success": true,
            "old_replicas": old,
            "new_replicas": new,
            "delta_applied": delta,
            "note": format!("Deployment scaled from {old} to {new} replicas (mock execution)"),
        }),
        attributes_json,
    )
}

fn restart_pod(target: &ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> HandlerOutcome {
    if target.resource_type != "Pod" {
        return HandlerOutcome::err(format!("target is {}, not Pod", target.resource_type));
    }
    let graceful = param_bool(params, "graceful", true);
    let grace_period = param_i64(params, "grace_period_seconds", 30);
    if !(0..=300).contains(&grace_period) {
        return HandlerOutcome::err(format!("grace_period_seconds out of range: {grace_period}"));
    }
    let a = attrs(target);
    let old = attr_i64(&a, "restart_count", 0);
    let new = old + 1;
    let attributes_json = with_attrs(target, |m| {
        m.insert("restart_count".into(), json!(new));
        // warning -> normal(对齐 reference _apply_dss)
        if m.get("health_status").and_then(Value::as_str) == Some("warning") {
            m.insert("health_status".into(), json!("normal"));
        }
    });
    HandlerOutcome::ok(
        json!({
            "success": true,
            "old_restart_count": old,
            "new_restart_count": new,
            "graceful": graceful,
            "grace_period_seconds": grace_period,
            "note": format!("Pod restarted (mock execution, count={new})"),
        }),
        attributes_json,
    )
}

fn rollback_deployment(target: &ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> HandlerOutcome {
    if target.resource_type != "Deployment" {
        return HandlerOutcome::err(format!("target is {}, not Deployment", target.resource_type));
    }
    let a = attrs(target);
    let old = attr_i64(&a, "current_revision", 1);
    let new = params
        .get("revision")
        .and_then(Value::as_i64)
        .unwrap_or_else(|| (old - 1).max(1));
    if new < 1 {
        return HandlerOutcome::err(format!("revision must be >= 1 (got {new})"));
    }
    let attributes_json = with_attrs(target, |m| {
        m.insert("current_revision".into(), json!(new));
    });
    HandlerOutcome::ok(
        json!({
            "success": true,
            "old_revision": old,
            "new_revision": new,
            "note": format!("Deployment rolled back from revision {old} to {new} (mock execution)"),
        }),
        attributes_json,
    )
}

fn refresh_secret(target: &ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> HandlerOutcome {
    if target.resource_type != "Secret" {
        return HandlerOutcome::err(format!("target is {}, not Secret", target.resource_type));
    }
    let trigger = param_bool(params, "trigger_pod_restart", true);
    let a = attrs(target);
    let old = attr_i64(&a, "secret_version", 1);
    let new = old + 1;
    let attributes_json = with_attrs(target, |m| {
        m.insert("secret_version".into(), json!(new));
    });
    HandlerOutcome::ok(
        json!({
            "success": true,
            "old_version": old,
            "new_version": new,
            "trigger_pod_restart": trigger,
            "note": format!("Secret refreshed from version {old} to {new} (mock execution)"),
        }),
        attributes_json,
    )
}

fn drain_node(target: &ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> HandlerOutcome {
    if target.resource_type != "KubernetesNode" {
        return HandlerOutcome::err(format!("target is {}, not KubernetesNode", target.resource_type));
    }
    let ignore_daemonsets = param_bool(params, "ignore_daemonsets", true);
    let delete_local_data = param_bool(params, "delete_local_data", false);
    let force = param_bool(params, "force", false);
    let attributes_json = with_attrs(target, |m| {
        m.insert("cordoned".into(), json!(true));
    });
    HandlerOutcome::ok(
        json!({
            "success": true,
            "cordoned": true,
            "ignore_daemonsets": ignore_daemonsets,
            "delete_local_data": delete_local_data,
            "force": force,
            "note": "Node cordoned + pods marked for eviction (mock execution; real evict deferred)",
        }),
        attributes_json,
    )
}

fn kill_query(target: &ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> HandlerOutcome {
    if target.resource_type != "MySQL" {
        return HandlerOutcome::err(format!("target is {}, not MySQL", target.resource_type));
    }
    let query_id = match params.get("query_id").and_then(Value::as_str) {
        Some(q) if !q.is_empty() => q.to_string(),
        _ => return HandlerOutcome::err("query_id is required"),
    };
    let min_duration = param_i64(params, "min_duration_seconds", 30);
    // 一次性动作,无持续副作用 -> 不产 attrs(对齐 reference verify_kill_query not_supported)
    HandlerOutcome::ok(
        json!({
            "success": true,
            "killed_query_id": query_id,
            "min_duration_seconds": min_duration,
            "note": "MySQL query killed (mock execution)",
        }),
        None,
    )
}

fn restart_service(target: &ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> HandlerOutcome {
    if target.resource_type != "Service" {
        return HandlerOutcome::err(format!("target is {}, not Service", target.resource_type));
    }
    let drop_idle = param_i64(params, "drop_idle_seconds", 0);
    let a = attrs(target);
    let old = attr_i64(&a, "endpoints_refresh_count", 0);
    let new = old + 1;
    let attributes_json = with_attrs(target, |m| {
        m.insert("endpoints_refresh_count".into(), json!(new));
    });
    HandlerOutcome::ok(
        json!({
            "success": true,
            "endpoints_regenerated": true,
            "endpoints_refresh_count": new,
            "drop_idle_seconds": drop_idle,
            "note": "Service endpoints regenerated (mock execution)",
        }),
        attributes_json,
    )
}

fn clear_cache(target: &ResolvedNode, params: &Value, _ctx: &ExecutionContext) -> HandlerOutcome {
    if target.resource_type != "Redis" {
        return HandlerOutcome::err(format!("target is {}, not Redis", target.resource_type));
    }
    let scope = param_str(params, "scope", "pattern");
    let db_index = param_i64(params, "db_index", 0);
    let key_pattern = param_str(params, "key_pattern", "");
    // 一次性动作,无持续副作用(对齐 reference verify_clear_cache not_supported)
    HandlerOutcome::ok(
        json!({
            "success": true,
            "scope": scope,
            "db_index": db_index,
            "key_pattern": key_pattern,
            "cleared": true,
            "note": "Redis cache cleared (mock execution)",
        }),
        None,
    )
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
    /// 从 HandlerOutcome.attributes_json 读 mutated attr。
    fn attr_after(outcome: &HandlerOutcome, key: &str) -> Option<Value> {
        outcome
            .attributes_json
            .as_ref()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .and_then(|v| v.get(key).cloned())
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
    fn scale_deployment_computes_attrs_and_validates() {
        let deploy = node("deploy:a", "Deployment", r#"{"desired_replicas":3}"#);
        let r = scale_deployment(&deploy, &json!({"replicas_delta":2}), &ctx());
        assert!(r.success);
        assert_eq!(r.result["old_replicas"], 3);
        assert_eq!(r.result["new_replicas"], 5);
        // attributes_json 含新 attrs
        assert_eq!(attr_after(&r, "desired_replicas"), Some(json!(5)));
        assert_eq!(attr_after(&r, "available_replicas"), Some(json!(5)));
        // delta=0 -> err,无 attrs
        let r0 = scale_deployment(&deploy, &json!({"replicas_delta":0}), &ctx());
        assert!(!r0.success);
        assert!(r0.attributes_json.is_none());
        // 类型不匹配
        let pod = node("pod:a", "Pod", "{}");
        let rt = scale_deployment(&pod, &json!({"replicas_delta":1}), &ctx());
        assert!(!rt.success);
    }

    #[test]
    fn restart_pod_computes_count_and_clears_warning() {
        let pod = node("pod:a", "Pod", r#"{"restart_count":2,"health_status":"warning"}"#);
        let r = restart_pod(&pod, &json!({}), &ctx());
        assert!(r.success);
        assert_eq!(r.result["new_restart_count"], 3);
        assert_eq!(attr_after(&r, "restart_count"), Some(json!(3)));
        assert_eq!(attr_after(&r, "health_status"), Some(json!("normal")));
    }

    #[test]
    fn kill_query_no_attrs() {
        let mysql = node("mysql:a", "MySQL", "{}");
        let r = kill_query(&mysql, &json!({"query_id":"q-42"}), &ctx());
        assert!(r.success);
        assert_eq!(r.result["killed_query_id"], "q-42");
        assert!(r.attributes_json.is_none());
    }

    #[test]
    fn drain_node_cordons() {
        let n = node("node:a", "KubernetesNode", "{}");
        let r = drain_node(&n, &json!({}), &ctx());
        assert!(r.success);
        assert_eq!(r.result["cordoned"], json!(true));
        assert_eq!(attr_after(&r, "cordoned"), Some(json!(true)));
    }

    #[test]
    fn restart_service_increments_refresh_count() {
        let s = node("svc:a", "Service", r#"{"endpoints_refresh_count":4}"#);
        let r = restart_service(&s, &json!({}), &ctx());
        assert!(r.success);
        assert_eq!(r.result["endpoints_refresh_count"], 5);
        assert_eq!(attr_after(&r, "endpoints_refresh_count"), Some(json!(5)));
    }
}
