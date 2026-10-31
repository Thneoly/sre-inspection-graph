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

/// 集群资源回读通道(desktop 实装:按 K8s API 路径 GET,经 kubectl proxy)。
///
/// 按**API 路径**(而非 resource_id)设计:部分判定要 GET 非单一资源的端点
/// (如 drain 的「node 上 pods 列表」走 fieldSelector 查询),resource_id 表达不了;
/// 路径知识由本模块的 [`resource_id_to_api_path`] 等统一提供,desktop 只做 HTTP。
#[async_trait::async_trait]
#[allow(clippy::double_must_use)] // async_trait 展开的 Box<Future> 已是 must_use,新版 clippy 叠加判定
pub trait ClusterReader: Send + Sync {
    /// GET `{api_base}{path}`(path 形如 `/api/v1/...` / `/apis/apps/v1/...`,
    /// 可带 ?labelSelector= / ?fieldSelector= query)。
    /// Err = 回读失败(网络 / 404);调用方决定回落或判 error。
    async fn get_by_api_path(&self, path: &str) -> Result<Value, String>;
}

/// canonical resource_id → K8s API 路径(判定与 desktop 共用一份映射)。
///
/// - `deploy:{c}:{ns}:{n}` → `/apis/apps/v1/namespaces/{ns}/deployments/{n}`
/// - `pod:{c}:{ns}:{n}`    → `/api/v1/namespaces/{ns}/pods/{n}`
/// - `secret:{c}:{ns}:{n}` → `/api/v1/namespaces/{ns}/secrets/{n}`
/// - `svc:{c}:{ns}:{n}`    → `/api/v1/namespaces/{ns}/endpoints/{n}`
///   (restart_service 的验证对象是 Endpoints;Service 自身不受该动作影响)
/// - `node:{c}:{n}`        → `/api/v1/nodes/{n}`(node 无 namespace)
///
/// 其它前缀(mysql:/redis:/image: 等)→ Err(verifier 侧回落 twin)。
pub fn resource_id_to_api_path(resource_id: &str) -> Result<String, String> {
    let parts: Vec<&str> = resource_id.split(':').collect();
    let path = match parts.as_slice() {
        [deploy, _c, ns, n] if *deploy == "deploy" => {
            format!("/apis/apps/v1/namespaces/{ns}/deployments/{n}")
        }
        [pod, _c, ns, n] if *pod == "pod" => format!("/api/v1/namespaces/{ns}/pods/{n}"),
        [secret, _c, ns, n] if *secret == "secret" => {
            format!("/api/v1/namespaces/{ns}/secrets/{n}")
        }
        [svc, _c, ns, n] if *svc == "svc" => format!("/api/v1/namespaces/{ns}/endpoints/{n}"),
        [node, _c, n] if *node == "node" => format!("/api/v1/nodes/{n}"),
        _ => return Err(format!("resource_id '{resource_id}' maps to no K8s API path")),
    };
    Ok(path)
}

/// 某 node 上的 pods 列表路径(drain 判定:evictable 是否清零)。
/// node 名是 DNS 标签(字母数字与 `-.`),query 值里无需转义;`=` 编码为 %3D。
fn node_pods_path(node: &str) -> String {
    format!("/api/v1/pods?fieldSelector=spec.nodeName%3D{node}")
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
    params: &Value,
    exec_result: &Value,
    ctx: &ExecutionContext,
) -> VerifierVerdict {
    for attempt in 1..=POLL_ATTEMPTS {
        match check_once(action_id, reader, target, params, exec_result, ctx).await {
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
    params: &Value,
    exec_result: &Value,
    _ctx: &ExecutionContext,
) -> ClusterCheck {
    use serde_json::json;
    let rid = &target.resource_id;
    // target 类型分流:MySQL/Redis(kill_query/clear_cache 的 target)无集群判定。
    let path = match resource_id_to_api_path(rid) {
        Ok(p) => p,
        Err(_) => return ClusterCheck::NotApplicable,
    };
    let fetched = match reader.get_by_api_path(&path).await {
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
            // 完整化 real handler = 真 rollout undo:revision-1 模板被 PATCH 回
            // deployment 后,controller 把 revision 注解推进为新值(不是 target
            // 值)。故判定「集群 revision != 执行前 revision」—— guest 的
            // exec_result 带 old_revision;缺失(mock result)回落 twin。
            let Some(old_rev) = exec_result.get("old_revision").and_then(Value::as_i64) else {
                return ClusterCheck::NotApplicable;
            };
            // K8s 注解值是字符串("4"),按 str 取再 parse
            let now_rev = fetched
                .pointer("/metadata/annotations/deployment.kubernetes.io~1revision")
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<i64>().ok());
            match now_rev {
                Some(r) if r != old_rev => {
                    verdict(true, json!(r), json!(format!("!={old_rev}")), String::new())
                }
                Some(r) => verdict(
                    false,
                    json!(r),
                    json!(format!("!={old_rev}")),
                    format!("revision still {r} (== pre-action {old_rev}), rollout undo not effective"),
                ),
                None => verdict(
                    false,
                    json!(null),
                    json!(format!("!={old_rev}")),
                    "deployment missing revision annotation".to_string(),
                ),
            }
        }
        "refresh_secret" => {
            // 完整化 real handler:rotatedAt 注解 +(若带 key/new_value_b64)
            // data[key] 已是新值。引用 Pod 的重建属 restart_pod 语义,poll 窗口
            // 内逐一判定不现实,不在此处检查。
            let rotated = fetched.pointer("/metadata/annotations/rotatedAt").is_some();
            let expected_val = params.get("new_value_b64").and_then(Value::as_str);
            let key = params.get("key").and_then(Value::as_str);
            let data_ok = match (key, expected_val) {
                (Some(k), Some(v)) => {
                    fetched.pointer(&format!("/data/{k}")).and_then(Value::as_str) == Some(v)
                }
                _ => true,
            };
            let passed = rotated && data_ok;
            verdict(
                passed,
                json!({ "rotated": rotated, "data_updated": data_ok }),
                json!({ "rotated": true, "data_updated": true }),
                if passed {
                    String::new()
                } else {
                    format!("secret rotation incomplete (rotated={rotated}, data_updated={data_ok})")
                },
            )
        }
        "drain_node" => {
            // 完整化 real handler = cordon + evict:unschedulable 且 node 上
            // 非 DaemonSet/mirror pod 清零(eviction 异步,poll 窗口内递减)。
            let node_name = rid
                .split(':')
                .nth(2)
                .unwrap_or_default()
                .to_string();
            let unsched = fetched
                .pointer("/spec/unschedulable")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let remaining = match reader.get_by_api_path(&node_pods_path(&node_name)).await {
                Ok(pods) => count_evictable(&pods),
                Err(e) => {
                    return verdict(
                        false,
                        json!({ "error": e }),
                        json!(null),
                        format!("node pods readback failed: {e}"),
                    )
                }
            };
            let passed = unsched && remaining == 0;
            verdict(
                passed,
                json!({ "unschedulable": unsched, "evictable_remaining": remaining }),
                json!({ "unschedulable": true, "evictable_remaining": 0 }),
                if passed {
                    String::new()
                } else {
                    format!("drain incomplete (cordon={unsched}, {remaining} evictable pod(s) left)")
                },
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

/// node pods 列表里非 DaemonSet/mirror pod 数(与 handler 的 evict 过滤同一规则)。
fn count_evictable(pods_list: &Value) -> usize {
    pods_list
        .pointer("/items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|pod| {
                    pod.pointer("/metadata/annotations/kubernetes.io~1config.mirror").is_none()
                        && !pod
                            .pointer("/metadata/ownerReferences")
                            .and_then(Value::as_array)
                            .map(|owners| {
                                owners.iter().any(|o| {
                                    o.pointer("/kind").and_then(Value::as_str) == Some("DaemonSet")
                                })
                            })
                            .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0)
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

    /// 可编程 fake reader:按 API path 返回预置 JSON(或 Err)。
    struct FakeReader {
        map: Mutex<std::collections::HashMap<String, Result<Value, String>>>,
        calls: Mutex<std::collections::HashMap<String, u32>>,
    }

    impl FakeReader {
        fn with(entries: &[(&str, Result<Value, String>)]) -> Self {
            let map = entries.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
            Self { map: Mutex::new(map), calls: Mutex::new(std::collections::HashMap::new()) }
        }
    }

    #[async_trait::async_trait]
    impl ClusterReader for FakeReader {
        async fn get_by_api_path(&self, path: &str) -> Result<Value, String> {
            *self.calls.lock().unwrap().entry(path.to_string()).or_insert(0) += 1;
            self.map
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .unwrap_or_else(|| Err(format!("no fixture for {path}")))
        }
    }

    #[test]
    fn maps_resource_ids_to_api_paths() {
        assert_eq!(
            resource_id_to_api_path("deploy:vm:otel-demo:frontend").unwrap(),
            "/apis/apps/v1/namespaces/otel-demo/deployments/frontend"
        );
        assert_eq!(
            resource_id_to_api_path("pod:vm:otel-demo:web-1").unwrap(),
            "/api/v1/namespaces/otel-demo/pods/web-1"
        );
        assert_eq!(
            resource_id_to_api_path("secret:vm:otel-demo:db-pass").unwrap(),
            "/api/v1/namespaces/otel-demo/secrets/db-pass"
        );
        assert_eq!(
            resource_id_to_api_path("svc:vm:otel-demo:web").unwrap(),
            "/api/v1/namespaces/otel-demo/endpoints/web"
        );
        assert_eq!(
            resource_id_to_api_path("node:vm:worker-1").unwrap(),
            "/api/v1/nodes/worker-1"
        );
        assert!(resource_id_to_api_path("mysql:vm:ns:db").is_err());
    }

    #[tokio::test]
    async fn scale_cluster_pass_when_replicas_match() {
        let path = "/apis/apps/v1/namespaces/ns/deployments/api";
        let r = FakeReader::with(&[(path, Ok(json!({ "spec": { "replicas": 4 } })))]);
        let v = verify_against_cluster(
            "scale_deployment",
            &r,
            &node("deploy:c:ns:api"),
            &json!({}),
            &json!({ "new_replicas": 4 }),
            &ctx(),
        )
        .await;
        assert!(v.passed, "msg: {}", v.message);
        assert_eq!(v.predicate, "scale_deployment/cluster");
        // 通过即返:只回读了 1 次
        assert_eq!(*r.calls.lock().unwrap().get(path).unwrap(), 1);
    }

    #[tokio::test]
    async fn scale_cluster_fails_after_poll_when_never_matches() {
        let path = "/apis/apps/v1/namespaces/ns/deployments/api";
        let r = FakeReader::with(&[(path, Ok(json!({ "spec": { "replicas": 1 } })))]);
        let v = verify_against_cluster(
            "scale_deployment",
            &r,
            &node("deploy:c:ns:api"),
            &json!({}),
            &json!({ "new_replicas": 4 }),
            &ctx(),
        )
        .await;
        assert!(!v.passed);
        assert!(v.message.contains("after 4 attempts"), "msg: {}", v.message);
        assert_eq!(*r.calls.lock().unwrap().get(path).unwrap(), 4);
    }

    #[tokio::test]
    async fn drain_passes_when_cordoned_and_no_evictable_pods() {
        // cordon + evict 完成:unschedulable 且 node pods 只剩 DaemonSet/mirror
        let node_path = "/api/v1/nodes/n1";
        let pods_path = "/api/v1/pods?fieldSelector=spec.nodeName%3Dn1";
        let r = FakeReader::with(&[
            (node_path, Ok(json!({ "spec": { "unschedulable": true } }))),
            (
                pods_path,
                Ok(json!({ "items": [
                    { "metadata": { "name": "ds", "namespace": "kube-system",
                                     "ownerReferences": [ { "kind": "DaemonSet" } ] } },
                    { "metadata": { "name": "mirror", "namespace": "kube-system",
                                     "annotations": { "kubernetes.io/config.mirror": "x" } } }
                ] })),
            ),
        ]);
        let v = verify_against_cluster(
            "drain_node",
            &r,
            &node("node:c:n1"),
            &json!({}),
            &json!({}),
            &ctx(),
        )
        .await;
        assert!(v.passed, "msg: {}", v.message);

        // 还有 1 个普通 pod 未 evict → 不通过
        let r2 = FakeReader::with(&[
            (node_path, Ok(json!({ "spec": { "unschedulable": true } }))),
            (
                pods_path,
                Ok(json!({ "items": [ { "metadata": { "name": "app", "namespace": "ns" } } ] })),
            ),
        ]);
        let v2 = verify_against_cluster(
            "drain_node",
            &r2,
            &node("node:c:n1"),
            &json!({}),
            &json!({}),
            &ctx(),
        )
        .await;
        assert!(!v2.passed);
        assert!(v2.message.contains("evictable"), "msg: {}", v2.message);
    }

    #[tokio::test]
    async fn rollback_passes_when_revision_moved_off_old() {
        // 真 rollout undo:集群 revision 离开执行前的值(controller 推进为新号)
        let path = "/apis/apps/v1/namespaces/ns/deployments/api";
        let r = FakeReader::with(&[(
            path,
            Ok(json!({ "metadata": { "annotations": { "deployment.kubernetes.io/revision": "4" } } })),
        )]);
        let v = verify_against_cluster(
            "rollback_deployment",
            &r,
            &node("deploy:c:ns:api"),
            &json!({}),
            &json!({ "old_revision": 3, "current_revision": 2 }),
            &ctx(),
        )
        .await;
        assert!(v.passed, "msg: {}", v.message);

        // revision 未动 → 不通过
        let r2 = FakeReader::with(&[(
            path,
            Ok(json!({ "metadata": { "annotations": { "deployment.kubernetes.io/revision": "3" } } })),
        )]);
        let v2 = verify_against_cluster(
            "rollback_deployment",
            &r2,
            &node("deploy:c:ns:api"),
            &json!({}),
            &json!({ "old_revision": 3 }),
            &ctx(),
        )
        .await;
        assert!(!v2.passed);
    }

    #[tokio::test]
    async fn rollback_without_old_revision_falls_back() {
        // exec_result 无 old_revision(mock result / 旧 guest)→ NotApplicable,
        // dual_source 回落 twin 判定(fetch 本身要成功,否则先走 readback-error)
        let path = "/apis/apps/v1/namespaces/ns/deployments/api";
        let r = FakeReader::with(&[(path, Ok(json!({})))]);
        let v = verify_against_cluster(
            "rollback_deployment",
            &r,
            &node("deploy:c:ns:api"),
            &json!({}),
            &json!({ "current_revision": 1 }),
            &ctx(),
        )
        .await;
        assert!(v.passed);
        assert_eq!(v.predicate, "not_supported");
    }

    #[tokio::test]
    async fn refresh_secret_checks_rotation_and_data() {
        let path = "/api/v1/namespaces/ns/secrets/db-pass";
        // 带 key/new_value_b64:rotatedAt + data 都要对
        let r = FakeReader::with(&[(
            path,
            Ok(json!({ "metadata": { "annotations": { "rotatedAt": "now" } },
                       "data": { "password": "bmV3dmFs" } })),
        )]);
        let v = verify_against_cluster(
            "refresh_secret",
            &r,
            &node("secret:c:ns:db-pass"),
            &json!({ "key": "password", "new_value_b64": "bmV3dmFs" }),
            &json!({}),
            &ctx(),
        )
        .await;
        assert!(v.passed, "msg: {}", v.message);

        // data 不匹配 → 失败
        let r2 = FakeReader::with(&[(
            path,
            Ok(json!({ "metadata": { "annotations": { "rotatedAt": "now" } },
                        "data": { "password": "b2xk" } })),
        )]);
        let v2 = verify_against_cluster(
            "refresh_secret",
            &r2,
            &node("secret:c:ns:db-pass"),
            &json!({ "key": "password", "new_value_b64": "bmV3dmFs" }),
            &json!({}),
            &ctx(),
        )
        .await;
        assert!(!v2.passed);
    }

    #[tokio::test]
    async fn restart_service_checks_endpoint_addresses() {
        let path = "/api/v1/namespaces/ns/endpoints/web";
        let r = FakeReader::with(&[(
            path,
            Ok(json!({ "subsets": [ { "addresses": [ { "ip": "10.1.2.3" }, { "ip": "10.1.2.4" } ] } ] })),
        )]);
        let v = verify_against_cluster(
            "restart_service",
            &r,
            &node("svc:c:ns:web"),
            &json!({}),
            &json!({}),
            &ctx(),
        )
        .await;
        assert!(v.passed, "msg: {}", v.message);
        assert_eq!(v.actual, json!(2));
    }

    #[tokio::test]
    async fn reader_error_yields_failed_verdict_not_silent_pass() {
        let path = "/api/v1/namespaces/ns/pods/p1";
        let r = FakeReader::with(&[(path, Err("cluster unreachable".into()))]);
        let v = verify_against_cluster(
            "restart_pod",
            &r,
            &node("pod:c:ns:p1"),
            &json!({}),
            &json!({}),
            &ctx(),
        )
        .await;
        assert!(!v.passed, "readback error must not pass");
        assert!(v.message.contains("cluster unreachable"));
    }

    #[tokio::test]
    async fn non_k8s_target_returns_not_supported_pass() {
        // MySQL(kill_query target)无集群判定 → not_supported(passed,调用方回落 twin)
        let r = FakeReader::with(&[]);
        let v = verify_against_cluster(
            "kill_query",
            &r,
            &node("mysql:c:ns:db"),
            &json!({}),
            &json!({}),
            &ctx(),
        )
        .await;
        assert!(v.passed);
        assert_eq!(v.predicate, "not_supported");
    }
}

