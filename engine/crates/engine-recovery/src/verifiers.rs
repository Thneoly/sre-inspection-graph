//! Recovery action verifiers(复刻 `reference/app/recovery/verifiers.py`)。
//!
//! 每个 verifier `fn(&ResolvedNode, &Value params, &Value exec_result, &ExecutionContext)
//! -> VerifierVerdict`,查 **twin 的 mutated attributes**(handler 刚写回)是否符合 predicate。
//!
//! ## 与 reference 的差异
//!
//! - **读 twin attrs**:reference 读全局 DSS 节点 properties(handler mutate 后);本 port 读
//!   入参 `&ResolvedNode` 的 `attributes_json`(handler 经 `&mut ResolvedNode` 写回)。语义一致
//!   (查动作生效后的状态),只是 twin 显式传入而非全局 DSS。
//! - **verifier set 可注入**:`run_verifier` 接 `verifiers: &[(&str, VerifierFn)]` 参数,
//!   默认 [`VERIFIERS`];测试可注入 fake failing verifier 触发 auto-rollback(对齐 reference
//!   测试 monkeypatch VERIFIERS)。Rust 无异常,verifier 不 panic,故无 try-catch。
//! - real 模式(查真实 K8s/MySQL/Redis 状态)留 write-capability WIT,延后。

#![allow(missing_docs)]

use engine_identity::ResolvedNode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::models::ExecutionContext;

/// verifier 函数指针类型。
pub type VerifierFn = fn(&ResolvedNode, &Value, &Value, &ExecutionContext) -> VerifierVerdict;

/// verifier 结论(对齐 reference `{passed, predicate, actual, expected, message}`)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifierVerdict {
    /// 是否通过。
    pub passed: bool,
    /// predicate 名(action_id / "skipped" / "not_supported" / "error")。
    pub predicate: String,
    /// 实际值。
    pub actual: Value,
    /// 期望值。
    pub expected: Value,
    /// 人读消息。
    pub message: String,
}

impl VerifierVerdict {
    pub(crate) fn make(passed: bool, predicate: &str, actual: Value, expected: Value, message: impl Into<String>) -> Self {
        Self {
            passed,
            predicate: predicate.to_string(),
            actual,
            expected,
            message: message.into(),
        }
    }

    fn not_supported(action_id: &str) -> Self {
        Self::make(true, "not_supported", json!(null), json!(null), format!("{action_id} has no observable side effect to verify"))
    }
}

/// 8 个 verifier 注册表(与 HANDLERS 平行)。
pub static VERIFIERS: &[(&str, VerifierFn)] = &[
    ("scale_deployment", verify_scale_deployment),
    ("restart_pod", verify_restart_pod),
    ("restart_service", verify_restart_service),
    ("refresh_secret", verify_refresh_secret),
    ("rollback_deployment", verify_rollback_deployment),
    ("drain_node", verify_drain_node),
    ("kill_query", verify_kill_query),
    ("clear_cache", verify_clear_cache),
];

/// 取 verifier(默认表);未注册返 None(verify_status=skipped)。
pub fn get_verifier(action_id: &str) -> Option<VerifierFn> {
    VERIFIERS.iter().find(|(id, _)| *id == action_id).map(|(_, f)| *f)
}

/// 统一入口。`verifiers` 可注入(默认 [`VERIFIERS`]);未注册 -> skipped(passed=true)。
pub fn run_verifier(
    action_id: &str,
    target: &ResolvedNode,
    params: &Value,
    exec_result: &Value,
    ctx: &ExecutionContext,
    verifiers: &[(&str, VerifierFn)],
) -> VerifierVerdict {
    match verifiers.iter().find(|(id, _)| *id == action_id) {
        Some((_, f)) => f(target, params, exec_result, ctx),
        None => VerifierVerdict::make(true, "skipped", json!(null), json!(null), format!("no verifier registered for {action_id}")),
    }
}

// ===== helpers =====

fn attrs(node: &ResolvedNode) -> serde_json::Map<String, Value> {
    match serde_json::from_str::<Value>(&node.attributes_json) {
        Ok(Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    }
}

fn attr_i64(attrs: &serde_json::Map<String, Value>, key: &str, default: i64) -> i64 {
    attrs.get(key).and_then(Value::as_i64).unwrap_or(default)
}

// ===== 8 verifiers =====

fn verify_scale_deployment(target: &ResolvedNode, _params: &Value, exec_result: &Value, _ctx: &ExecutionContext) -> VerifierVerdict {
    let expected = exec_result.get("new_replicas").and_then(Value::as_i64);
    let Some(expected) = expected else {
        return VerifierVerdict::make(false, "scale_deployment", json!(null), json!(null), "exec_result missing new_replicas");
    };
    let a = attrs(target);
    let actual_desired = attr_i64(&a, "desired_replicas", -1);
    let actual_avail = attr_i64(&a, "available_replicas", -1);
    let passed = actual_desired == expected && actual_avail == expected;
    VerifierVerdict::make(
        passed,
        "scale_deployment",
        json!({ "desired": actual_desired, "available": actual_avail }),
        json!({ "desired": expected, "available": expected }),
        if passed { String::new() } else { format!("replicas mismatch (desired={actual_desired}/{expected}, available={actual_avail}/{expected})") },
    )
}

fn verify_restart_pod(target: &ResolvedNode, _params: &Value, exec_result: &Value, _ctx: &ExecutionContext) -> VerifierVerdict {
    let expected_count = exec_result.get("new_restart_count").and_then(Value::as_i64);
    let a = attrs(target);
    let actual_count = attr_i64(&a, "restart_count", 0);
    let health = a.get("health_status").and_then(Value::as_str).unwrap_or("").to_string();
    let passed = expected_count.is_some_and(|ec| actual_count >= ec && matches!(health.as_str(), "" | "normal" | "healthy"));
    VerifierVerdict::make(
        passed,
        "restart_pod",
        json!({ "restart_count": actual_count, "health_status": health }),
        json!({ "restart_count_min": expected_count, "health_status_not": "warning" }),
        if passed { String::new() } else { format!("pod not yet restarted or still warning (count={actual_count}/{expected_count:?}, health={health})") },
    )
}

fn verify_restart_service(target: &ResolvedNode, _params: &Value, exec_result: &Value, _ctx: &ExecutionContext) -> VerifierVerdict {
    let expected = exec_result.get("endpoints_refresh_count").and_then(Value::as_i64);
    let a = attrs(target);
    let actual = attr_i64(&a, "endpoints_refresh_count", 0);
    let passed = expected.is_some_and(|e| actual >= e);
    VerifierVerdict::make(
        passed,
        "restart_service",
        json!(actual),
        json!(expected),
        if passed { String::new() } else { format!("endpoints not refreshed (count={actual}/{expected:?})") },
    )
}

fn verify_refresh_secret(target: &ResolvedNode, _params: &Value, exec_result: &Value, _ctx: &ExecutionContext) -> VerifierVerdict {
    let expected = exec_result.get("new_version").and_then(Value::as_i64);
    let a = attrs(target);
    let actual = attr_i64(&a, "secret_version", -1);
    let passed = expected.is_some_and(|e| actual >= e);
    VerifierVerdict::make(
        passed,
        "refresh_secret",
        json!(actual),
        json!(expected),
        if passed { String::new() } else { format!("secret_version mismatch ({actual}/{expected:?})") },
    )
}

fn verify_rollback_deployment(target: &ResolvedNode, _params: &Value, exec_result: &Value, _ctx: &ExecutionContext) -> VerifierVerdict {
    let expected = exec_result.get("new_revision").and_then(Value::as_i64);
    let a = attrs(target);
    let actual = attr_i64(&a, "current_revision", -1);
    let passed = expected == Some(actual);
    VerifierVerdict::make(
        passed,
        "rollback_deployment",
        json!(actual),
        json!(expected),
        if passed { String::new() } else { format!("revision mismatch ({actual} != {expected:?})") },
    )
}

fn verify_drain_node(target: &ResolvedNode, _params: &Value, _exec_result: &Value, _ctx: &ExecutionContext) -> VerifierVerdict {
    let a = attrs(target);
    let cordoned = a.get("cordoned").and_then(Value::as_bool).unwrap_or(false);
    let passed = cordoned;
    VerifierVerdict::make(
        passed,
        "drain_node",
        json!(cordoned),
        json!(true),
        if passed { String::new() } else { "node not cordoned".to_string() },
    )
}

fn verify_kill_query(_target: &ResolvedNode, _params: &Value, _exec_result: &Value, _ctx: &ExecutionContext) -> VerifierVerdict {
    VerifierVerdict::not_supported("kill_query")
}

fn verify_clear_cache(_target: &ResolvedNode, _params: &Value, _exec_result: &Value, _ctx: &ExecutionContext) -> VerifierVerdict {
    VerifierVerdict::not_supported("clear_cache")
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_identity::ResolvedNode;

    fn node(rtype: &str, attrs: &str) -> ResolvedNode {
        ResolvedNode { resource_id: "x".into(), resource_type: rtype.into(), label: "x".into(), attributes_json: attrs.into() }
    }
    fn ctx() -> ExecutionContext {
        ExecutionContext { execution_id: "e".into(), initiated_by: "t".into(), auto_rollback: false }
    }

    #[test]
    fn scale_passes_when_mutated() {
        // handler 已 mutate desired=available=5
        let n = node("Deployment", r#"{"desired_replicas":5,"available_replicas":5}"#);
        let v = verify_scale_deployment(&n, &json!({}), &json!({"new_replicas":5}), &ctx());
        assert!(v.passed);
    }

    #[test]
    fn scale_fails_when_mismatch() {
        let n = node("Deployment", r#"{"desired_replicas":5,"available_replicas":3}"#);
        let v = verify_scale_deployment(&n, &json!({}), &json!({"new_replicas":5}), &ctx());
        assert!(!v.passed);
    }

    #[test]
    fn restart_pod_passes_when_count_incremented() {
        let n = node("Pod", r#"{"restart_count":3,"health_status":"normal"}"#);
        let v = verify_restart_pod(&n, &json!({}), &json!({"new_restart_count":3}), &ctx());
        assert!(v.passed);
    }

    #[test]
    fn restart_pod_fails_when_still_warning() {
        let n = node("Pod", r#"{"restart_count":3,"health_status":"warning"}"#);
        let v = verify_restart_pod(&n, &json!({}), &json!({"new_restart_count":3}), &ctx());
        assert!(!v.passed);
    }

    #[test]
    fn drain_node_passes_when_cordoned() {
        let n = node("KubernetesNode", r#"{"cordoned":true}"#);
        assert!(verify_drain_node(&n, &json!({}), &json!({}), &ctx()).passed);
        let n2 = node("KubernetesNode", r#"{}"#);
        assert!(!verify_drain_node(&n2, &json!({}), &json!({}), &ctx()).passed);
    }

    #[test]
    fn kill_query_and_clear_cache_not_supported() {
        let n = node("MySQL", "{}");
        let v = verify_kill_query(&n, &json!({}), &json!({}), &ctx());
        assert!(v.passed);
        assert_eq!(v.predicate, "not_supported");
        let n2 = node("Redis", "{}");
        let v2 = verify_clear_cache(&n2, &json!({}), &json!({}), &ctx());
        assert!(v2.passed);
        assert_eq!(v2.predicate, "not_supported");
    }

    #[test]
    fn run_verifier_skipped_when_not_registered() {
        let n = node("Pod", "{}");
        let v = run_verifier("nonexistent", &n, &json!({}), &json!({}), &ctx(), VERIFIERS);
        assert!(v.passed);
        assert_eq!(v.predicate, "skipped");
    }

    #[test]
    fn run_verifier_injectable_for_test() {
        // 注入一个 fake failing verifier(对齐 reference test monkeypatch VERIFIERS)
        fn fail_verify(_t: &ResolvedNode, _p: &Value, _r: &Value, _c: &ExecutionContext) -> VerifierVerdict {
            VerifierVerdict::make(false, "fake", json!(null), json!(null), "injected failure")
        }
        let custom: &[(&str, VerifierFn)] = &[("scale_deployment", fail_verify)];
        let n = node("Deployment", r#"{"desired_replicas":5,"available_replicas":5}"#);
        let v = run_verifier("scale_deployment", &n, &json!({}), &json!({"new_replicas":5}), &ctx(), custom);
        assert!(!v.passed);
        assert_eq!(v.predicate, "fake");
    }
}
