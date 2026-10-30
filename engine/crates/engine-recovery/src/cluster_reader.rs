//! ClusterReader —— real 模式 verifier 的集群回读通道(消灭「verify 读 twin 自报」)。
//!
//! ## 背景
//!
//! v0 的 verifier 只读 handler 写回 twin 的自报 attrs —— real 模式下 handler 说
//! 成功就成功,verify 几乎必过。本模块给 verifier 提供**第二条真相源**:经
//! kubectl proxy 回读真实集群状态。与 `HandlerExecutor` 同一注入模式:engine
//! 定义 trait(I/O-free 纪律不破),desktop 层实装(reqwest GET)。
//!
//! ## 双源语义(`run_verifier_dual_source`)
//!
//! - `cluster = None`(mock/演练):纯 twin 自报 —— 既有行为不变。
//! - `cluster = Some`(real):**集群回读优先**,twin 回落 ——
//!   - 动作有集群判定(scale 等 6 个 k8s action)且回读成功 → 集群 verdict
//!     (predicate 带 `/cluster` 后缀,UI 可见判定来源)
//!   - 动作无集群判定(kill_query / clear_cache)或 resource_id 不在 K8s →
//!     回落 twin 自报(verifiers.rs 既有逻辑)
//!
//! ## poll 语义
//!
//! K8s 控制器生效是异步的(scale PATCH 后 available_replicas 要等 reconcile,
//! DELETE pod 后重建要等调度),verify 一次回读可能还没生效。故逐动作 poll:
//! 通过即返,全部尝试(`POLL_ATTEMPTS`)用尽才判 failed。
//!
//! ## 6 个 k8s action 的判定(与 k8s-handler real 实现一一对应)
//!
//! - scale_deployment: GET deployment → `spec.replicas == exec_result.new_replicas`
//! - restart_pod: GET pod → 存在且 `status.phase == Running`(controller 重建完成)
//! - rollback_deployment: GET deployment → annotations 含 `kubectl.kubernetes.io/restartedAt`
//!   (real handler 是 restartedAt 滚动重启,见 ActionDef description 的简化语义)
//! - refresh_secret: GET secret → annotations 含 `rotatedAt`
//! - drain_node: GET node → `spec.unschedulable == true`
//! - restart_service: GET endpoints(subsets[].addresses 非空;controller 重建)

use serde_json::Value;

use engine_identity::ResolvedNode;

use crate::models::ExecutionContext;
use crate::verifiers::VerifierVerdict;

/// 集群资源回读通道(desktop 实装:resource_id → K8s API GET)。
#[async_trait::async_trait]
pub trait ClusterReader: Send + Sync {
    /// 回读 canonical resource_id(`deploy:{c}:{ns}:{n}` 等)对应的 K8s API JSON。
    /// Err = 回读失败(网络 / 资源不存在);调用方决定回落或判 error。
    async fn get_resource_json(&self, resource_id: &str) -> Result<Value, String>;
}

/// poll 次数 × 间隔(4 × 1.5s ≈ 6s 窗口;通过即提前返回)。
const POLL_ATTEMPTS: u32 = 4;
const POLL_INTERVAL_MS: u64 = 1500;

/// 判定结果:该动作是否具备集群回读判定(否则调用方回落 twin)。
enum ClusterCheck {
    /// 有判定,给出 verdict。
    Verdict(VerifierVerdict),
    /// 该动作 / 该 target 不适用集群回读(回落 twin)。
    NotApplicable,
}

/// 集群回读判定入口(poll 包裹)。`reader` 错误 → verdict error(不静默通过)。
pub async fn verify_against_cluster(
    action_id: &str,
    reader: &dyn ClusterReader,
    target: &ResolvedNode,
    exec_result: &Value,
    ctx: &ExecutionContext,
) -> VerifierVerdict {
    for attempt in 1..=POLL_ATTEMPTS {
        match check_once(action_id, reader, target, exec_result, ctx).await {
            ClusterCheck::NotApplicable => {
                // 调用方(dual_source)负责回落,这里给个通过的占位 —— 实际不会
                // 走到(NotApplicable 在 dual_source 已分流);防御性返回。
                return VerifierVerdict::make(
                    true,
                    "not_supported",
                    Value::Null,
                    Value::Null,
                    "no cluster check for action",
                );
            }
            ClusterCheck::Verdict(v) if v.passed => {
                if attempt > 1 {
                    tracing::debug!(action = action_id, attempt, "cluster verify passed after poll");
                }
                return v;
            }
            ClusterCheck::Verdict(v) if attempt == POLL_ATTEMPTS => {
                // 最后一次尝试仍失败 → 判 failed(message 带 poll 窗口说明)
                let msg = format!("{} (cluster readback failed after {POLL_ATTEMPTS} attempts)", v.message);
                return VerifierVerdict::make(false, &v.predicate, v.actual, v.expected, msg);
            }
            ClusterCheck::Verdict(_) => {
                tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
            }
        }
    }
    unreachable!("poll loop returns within attempts")
}

/// 单次回读判定。
async fn check_once(
    action_id: &str,
    reader: &dyn ClusterReader,
    target: &ResolvedNode,
    exec_result: &Value,
    _ctx: &ExecutionContext,
) -> ClusterCheck {
    use serde_json::json;
    let rid = &target.resource_id;
    // target 类型分流:MySQL/Redis(kill_query/clear_cache 的 target)无集群判定。
    let is_k8s = rid.starts_with("deploy:") || rid.starts_with("pod:") || rid.starts_with("node:")
        || rid.starts_with("secret:") || rid.starts_with("svc:");
    if !is_k8s {
        return ClusterCheck::NotApplicable;
    }
    let fetched = match reader.get_resource_json(rid).await {
        Ok(v) => v,
        Err(e) => {
            return ClusterCheck::Verdict(VerifierVerdict::make(
                false,
                &format!("{action_id}/cluster"),
                json!({ "error": e }),
                Value::Null,
                format!("cluster readback failed: {e}"),
            ))
        }
    };

    let predicate = format!("{action_id}/cluster");
    let verdict = |passed: bool, actual: Value, expected: Value, msg: String| {
        ClusterCheck::Verdict(VerifierVerdict::make(passed, &predicate, actual, expected, msg))
    };

    match action_id {
        "scale_deployment" => {
            let expected = exec_result.get("new_replicas").and_then(Value::as_i64);
            let actual = fetched.pointer("/spec/replicas").and_then(Value::as_i64);
            match (expected, actual) {
                (Some(e), Some(a)) if a == e => verdict(true, json!(a), json!(e), String::new()),
                (Some(e), Some(a)) => verdict(
                    false,
                    json!(a),
                    json!(e),
                    format!("cluster spec.replicas={a}, expected {e}"),
                ),
                (e, a) => verdict(
                    false,
                    json!(a),
                    json!(e),
                    "cluster deployment missing spec.replicas / exec_result missing new_replicas"
                        .to_string(),
                ),
            }
        }
        "restart_pod" => {
            // pod 存在(GET 成功即存在,404 走 Err 分支)+ phase Running
            let phase = fetched
                .pointer("/status/phase")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let passed = phase == "Running";
            verdict(
                passed,
                json!(phase),
                json!("Running"),
                if passed { String::new() } else { format!("pod phase={phase}, expected Running (not yet recreated?)") },
            )
        }
        "rollback_deployment" => {
            // real handler PATCH restartedAt 注解 —— 验证注解真的在 deployment 上
            let has = fetched
                .pointer("/metadata/annotations/kubectl.kubernetes.io~1restartedAt")
                .is_some();
            verdict(
                has,
                json!(has),
                json!(true),
                if has { String::new() } else { "restartedAt annotation not present on deployment".to_string() },
            )
        }
        "refresh_secret" => {
            let has = fetched.pointer("/metadata/annotations/rotatedAt").is_some();
            verdict(
                has,
                json!(has),
                json!(true),
                if has { String::new() } else { "rotatedAt annotation not present on secret".to_string() },
            )
        }
        "drain_node" => {
            let unsched = fetched
                .pointer("/spec/unschedulable")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            verdict(
                unsched,
                json!(unsched),
                json!(true),
                if unsched { String::new() } else { "node not unschedulable (cordon not effective)".to_string() },
            )
        }
        "restart_service" => {
            // DELETE endpoints 后 controller 重建:subsets[].addresses 非空
            let addr_count = fetched
                .pointer("/subsets")
                .and_then(Value::as_array)
                .map(|subsets| {
                    subsets
                        .iter()
                        .filter_map(|s| s.pointer("/addresses").and_then(Value::as_array))
                        .map(|a| a.len())
                        .sum::<usize>()
                })
                .unwrap_or(0);
            let passed = addr_count > 0;
            verdict(
                passed,
                json!(addr_count),
                json!(">0"),
                if passed { String::new() } else { format!("endpoints addresses={addr_count} (not yet regenerated?)") },
            )
        }
        // kill_query / clear_cache(mock-only action)及未来 action:回落 twin
        _ => ClusterCheck::NotApplicable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    fn node(rid: &str) -> ResolvedNode {
        ResolvedNode {
            resource_id: rid.into(),
            resource_type: "X".into(),
            label: rid.into(),
            attributes_json: "{}".into(),
        }
    }

    fn ctx() -> ExecutionContext {
        ExecutionContext { execution_id: "e".into(), initiated_by: "t".into(), auto_rollback: false }
    }

    /// 可编程 fake reader:按 resource_id 返回预置 JSON(或 Err)。
    struct FakeReader {
        map: Mutex<std::collections::HashMap<String, Result<Value, String>>>,
        // 读计数(验证 poll 通过即返)
        calls: Mutex<std::collections::HashMap<String, u32>>,
    }

    impl FakeReader {
        fn with(rid: &str, v: Result<Value, String>) -> Self {
            let mut map = std::collections::HashMap::new();
            map.insert(rid.to_string(), v);
            Self { map: Mutex::new(map), calls: Mutex::new(std::collections::HashMap::new()) }
        }
    }

    #[async_trait::async_trait]
    impl ClusterReader for FakeReader {
        async fn get_resource_json(&self, resource_id: &str) -> Result<Value, String> {
            *self.calls.lock().unwrap().entry(resource_id.to_string()).or_insert(0) += 1;
            self.map
                .lock()
                .unwrap()
                .get(resource_id)
                .cloned()
                .unwrap_or(Err(format!("no fixture for {resource_id}")))
        }
    }

    #[tokio::test]
    async fn scale_cluster_pass_when_replicas_match() {
        let r = FakeReader::with(
            "deploy:c:ns:api",
            Ok(json!({ "spec": { "replicas": 4 } })),
        );
        let v = verify_against_cluster(
            "scale_deployment",
            &r,
            &node("deploy:c:ns:api"),
            &json!({ "new_replicas": 4 }),
            &ctx(),
        )
        .await;
        assert!(v.passed, "msg: {}", v.message);
        assert_eq!(v.predicate, "scale_deployment/cluster");
        // 通过即返:只回读了 1 次
        assert_eq!(*r.calls.lock().unwrap().get("deploy:c:ns:api").unwrap(), 1);
    }

    #[tokio::test]
    async fn scale_cluster_fails_after_poll_when_never_matches() {
        let r = FakeReader::with(
            "deploy:c:ns:api",
            Ok(json!({ "spec": { "replicas": 1 } })),
        );
        let v = verify_against_cluster(
            "scale_deployment",
            &r,
            &node("deploy:c:ns:api"),
            &json!({ "new_replicas": 4 }),
            &ctx(),
        )
        .await;
        assert!(!v.passed);
        assert!(v.message.contains("after 4 attempts"), "msg: {}", v.message);
        // poll 用满 4 次
        assert_eq!(*r.calls.lock().unwrap().get("deploy:c:ns:api").unwrap(), 4);
    }

    #[tokio::test]
    async fn drain_checks_unschedulable() {
        let r = FakeReader::with("node:c:n1", Ok(json!({ "spec": { "unschedulable": true } })));
        let v = verify_against_cluster("drain_node", &r, &node("node:c:n1"), &json!({}), &ctx()).await;
        assert!(v.passed, "msg: {}", v.message);

        let r2 = FakeReader::with("node:c:n1", Ok(json!({ "spec": {} })));
        let v2 = verify_against_cluster("drain_node", &r2, &node("node:c:n1"), &json!({}), &ctx()).await;
        assert!(!v2.passed);
    }

    #[tokio::test]
    async fn rollback_checks_restarted_at_annotation() {
        // JSON pointer 里 / 要转义为 ~1(注解键含 /)
        let r = FakeReader::with(
            "deploy:c:ns:api",
            Ok(json!({ "metadata": { "annotations": { "kubectl.kubernetes.io/restartedAt": "now" } } })),
        );
        let v = verify_against_cluster(
            "rollback_deployment",
            &r,
            &node("deploy:c:ns:api"),
            &json!({}),
            &ctx(),
        )
        .await;
        assert!(v.passed, "msg: {}", v.message);
    }

    #[tokio::test]
    async fn restart_service_checks_endpoint_addresses() {
        let r = FakeReader::with(
            "svc:c:ns:web",
            Ok(json!({ "subsets": [ { "addresses": [ {"ip": "10.1.2.3"}, {"ip": "10.1.2.4"} ] } ] })),
        );
        let v = verify_against_cluster("restart_service", &r, &node("svc:c:ns:web"), &json!({}), &ctx()).await;
        assert!(v.passed, "msg: {}", v.message);
        assert_eq!(v.actual, json!(2));
    }

    #[tokio::test]
    async fn reader_error_yields_failed_verdict_not_silent_pass() {
        let r = FakeReader::with("pod:c:ns:p1", Err("cluster unreachable".into()));
        let v = verify_against_cluster("restart_pod", &r, &node("pod:c:ns:p1"), &json!({}), &ctx()).await;
        assert!(!v.passed, "readback error must not pass");
        assert!(v.message.contains("cluster unreachable"));
    }

    #[tokio::test]
    async fn non_k8s_target_returns_not_supported_pass() {
        // MySQL(kill_query target)无集群判定 → not_supported(passed,调用方回落 twin)
        let r = FakeReader::with("mysql:c:ns:db", Ok(json!({})));
        let v = verify_against_cluster("kill_query", &r, &node("mysql:c:ns:db"), &json!({}), &ctx()).await;
        assert!(v.passed);
        assert_eq!(v.predicate, "not_supported");
    }
}
