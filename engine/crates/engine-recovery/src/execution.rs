//! RecoveryExecution 生命周期编排(复刻 `reference/app/recovery/execution.py`)。
//!
//! 3.2 范围:execute 管线 + 单机确认门(confirm/cancel)+ rollback。
//! 3.3 加:verifier + auto-rollback(verify_failed 触发,防递归)+ reverify。
//!
//! ## 与 reference 的差异
//!
//! - **I/O-free + 显式 registry**:reference 读全局 DSS;本模块 [`ExecutionRegistry`] 入参。
//! - **mutable topology twin**:execute/confirm/rollback 接 `&mut Topology` -- mock handler
//!   经 `&mut ResolvedNode` 把动作生效写回 twin 的 `attributes_json`,verifier 读 mutated
//!   attrs。orchestration(3.6)应传 materialized topology 的 **clone**(避免污染真相源)。
//! - **单机确认门**([[phase3-approval-decision]]):无 ApprovalRequest 实体/TTL/approver_team。
//! - **verifier set 可注入**:[`verify_and_maybe_rollback`] 接 `verifiers` 参数,测试注入
//!   fake failing verifier 触发 auto-rollback(对齐 reference monkeypatch VERIFIERS)。
//! - rollback skip re-approval;auto-rollback marker 使反向 exec skip verify(防递归)。

#![allow(missing_docs)]

use std::collections::HashMap;

use engine_identity::{ResolvedNode, Topology};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::action_defs::{get_action, RiskLevel};
use crate::cascade::dry_run;
use crate::handlers::{get_handler, is_executable};
use crate::models::{ExecutionContext, ExecutionError, RecoveryExecution, RecoveryStatus, VerifyStatus};
use crate::verifiers::{run_verifier, VerifierFn, VerifierVerdict, VERIFIERS};

/// in-memory execution 注册表(对齐 reference DSS `store.executions`,但显式非全局)。
#[derive(Debug, Clone, Default)]
pub struct ExecutionRegistry {
    executions: HashMap<String, RecoveryExecution>,
}

impl ExecutionRegistry {
    pub fn new() -> Self {
        Self { executions: HashMap::new() }
    }

    /// 从已加载的 execution 列表构造(orchestration 从 storage 恢复用)。
    pub fn from_executions(es: Vec<RecoveryExecution>) -> Self {
        Self { executions: es.into_iter().map(|e| (e.execution_id.clone(), e)).collect() }
    }

    pub fn get(&self, id: &str) -> Option<&RecoveryExecution> {
        self.executions.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut RecoveryExecution> {
        self.executions.get_mut(id)
    }

    pub fn insert(&mut self, e: RecoveryExecution) {
        self.executions.insert(e.execution_id.clone(), e);
    }

    pub fn list(&self) -> Vec<&RecoveryExecution> {
        self.executions.values().collect()
    }

    /// 过滤列表(新到旧,按 initiated_at 降序)。None = 不筛。
    pub fn list_filtered(
        &self,
        status: Option<RecoveryStatus>,
        action_id: Option<&str>,
        target_resource_id: Option<&str>,
        limit: usize,
    ) -> Vec<&RecoveryExecution> {
        let mut es: Vec<&RecoveryExecution> = self
            .executions
            .values()
            .filter(|e| status.is_none_or(|s| e.status == s))
            .filter(|e| action_id.is_none_or(|a| e.action_id == a))
            .filter(|e| target_resource_id.is_none_or(|t| e.target_resource_id == t))
            .collect();
        es.sort_by(|a, b| b.initiated_at.cmp(&a.initiated_at));
        es.truncate(limit);
        es
    }

    pub fn len(&self) -> usize {
        self.executions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.executions.is_empty()
    }
}

/// 执行恢复动作。
///
/// - low 风险 + 不需审批 -> 同步跑 handler(+verify),返 `succeeded`/`failed`。
/// - medium/high 或 requires_approval -> 返 `awaiting_approval`。
///
/// `topology` 是 mutable twin:mock handler 把生效写回其节点 `attributes_json`。调用方应传
/// clone(若需保留原拓扑)。抛 [`ExecutionError`] = 前置校验失败(404/501/400)。
pub fn execute(
    registry: &mut ExecutionRegistry,
    action_id: &str,
    target_resource_id: &str,
    input_params: &Value,
    topology: &mut Topology,
    initiated_by: &str,
    request_reason: &str,
) -> Result<RecoveryExecution, ExecutionError> {
    let action = get_action(action_id)
        .ok_or_else(|| ExecutionError::with_code(format!("unknown action_id: {action_id}"), 404))?;

    if !is_executable(action_id) {
        return Err(ExecutionError::with_code(
            format!("action '{action_id}' has no execute handler"),
            501,
        ));
    }

    let dry_result = dry_run(action_id, target_resource_id, input_params, topology);
    if !dry_result.target_valid {
        return Err(ExecutionError::with_code(
            format!(
                "dry-run validation failed: {}",
                dry_result.validation_error.as_deref().unwrap_or("")
            ),
            400,
        ));
    }

    let target_node = topology
        .nodes
        .iter()
        .find(|n| n.resource_id == target_resource_id);
    let needs_approval = action.risk_level != RiskLevel::Low || action.requires_approval;
    let now = now_iso();
    let target_type = target_node
        .map(|n| n.resource_type.clone())
        .unwrap_or_else(|| action.target_type.to_string());
    let cluster_id = derive_cluster_id(target_resource_id, target_node);
    // target_node 借用到此结束(上面已 clone 出 target_type / cluster_id)

    let execution = RecoveryExecution {
        execution_id: Uuid::new_v4().to_string(),
        action_id: action_id.to_string(),
        target_resource_id: target_resource_id.to_string(),
        target_resource_type: target_type,
        input_params: input_params.clone(),
        dry_run_result: dry_result,
        status: if needs_approval {
            RecoveryStatus::AwaitingApproval
        } else {
            RecoveryStatus::Executing
        },
        initiated_by: initiated_by.to_string(),
        request_reason: request_reason.to_string(),
        initiated_at: now,
        cluster_id,
        ..Default::default()
    };

    let execution_id = execution.execution_id.clone();
    registry.insert(execution);

    if needs_approval {
        return Ok(registry.get(&execution_id).cloned().expect("just inserted"));
    }

    // low 风险 -> 同步跑 handler(+ verify + 可能 auto-rollback)
    run_handler(registry, &execution_id, topology, true, true);
    Ok(registry.get(&execution_id).cloned().expect("just inserted"))
}

/// 确认执行(单机确认门 = 操作者点确认)。
///
/// `awaiting_approval` -> 跑 handler(+ verify) -> `succeeded`/`failed`。非 awaiting -> 409。
pub fn confirm_execution(
    registry: &mut ExecutionRegistry,
    execution_id: &str,
    topology: &mut Topology,
    approval_comment: &str,
) -> Result<RecoveryExecution, ExecutionError> {
    {
        let exec = registry
            .get(execution_id)
            .ok_or_else(|| ExecutionError::with_code(format!("execution not found: {execution_id}"), 404))?;
        if exec.status != RecoveryStatus::AwaitingApproval {
            return Err(ExecutionError::with_code(
                format!("execution status is {:?}, expected awaiting_approval", exec.status),
                409,
            ));
        }
    }
    {
        let exec = registry.get_mut(execution_id).expect("checked above");
        exec.approved_at = now_iso();
        exec.approval_comment = approval_comment.to_string();
    }
    run_handler(registry, execution_id, topology, true, true);
    Ok(registry.get(execution_id).cloned().expect("checked above"))
}

/// 取消执行(单机确认门 = 操作者拒绝)。`awaiting_approval` -> `rejected`。非 awaiting -> 409。
pub fn cancel_execution(
    registry: &mut ExecutionRegistry,
    execution_id: &str,
) -> Result<RecoveryExecution, ExecutionError> {
    let exec = registry
        .get_mut(execution_id)
        .ok_or_else(|| ExecutionError::with_code(format!("execution not found: {execution_id}"), 404))?;
    if exec.status != RecoveryStatus::AwaitingApproval {
        return Err(ExecutionError::with_code(
            format!("execution status is {:?}, expected awaiting_approval", exec.status),
            409,
        ));
    }
    exec.status = RecoveryStatus::Rejected;
    exec.completed_at = now_iso();
    Ok(registry.get(execution_id).cloned().expect("checked above"))
}

/// 回滚一个 `succeeded` execution。
///
/// 创建反向 execution(`reverses_execution_id` 指向原),直接同步执行(skip re-approval)。
/// 反向 handler 读 twin 的 **post-action** 状态(原 execution 已 mutate),故正确反转。
/// 仅 `succeeded` 可回滚;`rolled_back` 后不可再回滚;无 `rollback_action_id` -> 400。
pub fn rollback(
    registry: &mut ExecutionRegistry,
    execution_id: &str,
    topology: &mut Topology,
    initiated_by: &str,
    reason: &str,
) -> Result<RecoveryExecution, ExecutionError> {
    let (status, rollback_execution_id, action_id) = {
        let o = registry
            .get(execution_id)
            .ok_or_else(|| ExecutionError::with_code(format!("execution not found: {execution_id}"), 404))?;
        (o.status, o.rollback_execution_id.clone(), o.action_id.clone())
    };
    if status != RecoveryStatus::Succeeded {
        return Err(ExecutionError::with_code(
            format!("only succeeded executions can be rolled back (current: {status:?})"),
            409,
        ));
    }
    if let Some(rb_id) = rollback_execution_id {
        return Err(ExecutionError::with_code(
            format!("execution already rolled back by {rb_id}"),
            409,
        ));
    }
    let rollback_action_id = get_action(&action_id)
        .and_then(|a| a.rollback_action_id)
        .ok_or_else(|| {
            ExecutionError::with_code(format!("action '{action_id}' has no rollback_action_id"), 400)
        })?;
    do_rollback(
        registry,
        execution_id,
        rollback_action_id,
        topology,
        initiated_by,
        reason,
        false, // 手动 rollback:auto_rollback_marker=false -> 反向 exec 仍 verify
    )
}

/// 主动重新验证(不触发 auto-rollback)。仅 `succeeded`/`rolled_back` 可 reverify;其他 409。
pub fn reverify(
    registry: &mut ExecutionRegistry,
    execution_id: &str,
    topology: &mut Topology,
) -> Result<RecoveryExecution, ExecutionError> {
    let status = registry
        .get(execution_id)
        .ok_or_else(|| ExecutionError::with_code(format!("execution not found: {execution_id}"), 404))?
        .status;
    if status != RecoveryStatus::Succeeded && status != RecoveryStatus::RolledBack {
        return Err(ExecutionError::with_code(
            format!("reverify only allowed on succeeded/rolled_back (current: {status:?})"),
            409,
        ));
    }
    verify_and_maybe_rollback(registry, execution_id, topology, false, VERIFIERS);
    Ok(registry.get(execution_id).cloned().expect("checked above"))
}

/// 实际创建并执行 rollback execution。
///
/// `auto_rollback_marker=true`(verify_failed 触发的自动回滚):反向 exec **skip verify**
/// (防 verify_failed -> rollback -> verify_failed 死循环)。
pub(crate) fn do_rollback(
    registry: &mut ExecutionRegistry,
    original_id: &str,
    rollback_action_id: &str,
    topology: &mut Topology,
    initiated_by: &str,
    reason: &str,
    auto_rollback_marker: bool,
) -> Result<RecoveryExecution, ExecutionError> {
    let (target_resource_id, target_resource_type, finding_id, input_params, cluster_id) = {
        let o = registry
            .get(original_id)
            .ok_or_else(|| ExecutionError::with_code(format!("execution not found: {original_id}"), 404))?;
        (
            o.target_resource_id.clone(),
            o.target_resource_type.clone(),
            o.finding_id.clone(),
            o.input_params.clone(),
            o.cluster_id.clone(),
        )
    };
    let original_action_id = registry.get(original_id).expect("checked").action_id.clone();
    let rollback_params = derive_rollback_params(&original_action_id, &input_params);
    // dry-run 允许失败(回滚是兜底),不阻塞
    let dry_result = dry_run(rollback_action_id, &target_resource_id, &rollback_params, topology);
    let now = now_iso();

    let rb = RecoveryExecution {
        execution_id: Uuid::new_v4().to_string(),
        action_id: rollback_action_id.to_string(),
        target_resource_id,
        target_resource_type,
        finding_id,
        input_params: rollback_params,
        dry_run_result: dry_result,
        status: RecoveryStatus::Executing,
        initiated_by: initiated_by.to_string(),
        request_reason: if reason.is_empty() {
            format!("rollback of {original_id}")
        } else {
            reason.to_string()
        },
        initiated_at: now.clone(),
        executed_at: now,
        reverses_execution_id: Some(original_id.to_string()),
        cluster_id,
        ..Default::default()
    };
    let rb_id = rb.execution_id.clone();
    registry.insert(rb);

    // 反向 handler:auto_rollback=false(rollback 自身不再 auto-rollback);
    // verify = !auto_rollback_marker(自动回滚的反向 exec skip verify,防递归)
    run_handler(registry, &rb_id, topology, false, !auto_rollback_marker);

    let rb_succeeded = registry
        .get(&rb_id)
        .map(|e| e.status == RecoveryStatus::Succeeded)
        .unwrap_or(false);
    if rb_succeeded {
        let now2 = now_iso();
        let orig = registry.get_mut(original_id).expect("checked");
        orig.rollback_execution_id = Some(rb_id.clone());
        orig.status = RecoveryStatus::RolledBack;
        orig.completed_at = now2;
    }
    if auto_rollback_marker {
        if let Some(rb_exec) = registry.get_mut(&rb_id) {
            push_to_result(&mut rb_exec.result, "auto_rollback_origin", json!(original_id));
        }
    }
    Ok(registry.get(&rb_id).cloned().expect("just inserted"))
}

/// 跑 handler + (若 succeeded + verify)跑 verifier + (verify_failed + auto_rollback)触发自动回滚。
///
/// `auto_rollback=true`:succeeded 后若 verify_status=failed -> 自动 do_rollback(marker=true)。
/// `verify=false`:跳过 verifier(rollback 的反向 exec / 测试场景)。
pub(crate) fn run_handler(
    registry: &mut ExecutionRegistry,
    execution_id: &str,
    topology: &mut Topology,
    auto_rollback: bool,
    verify: bool,
) {
    let (action_id, target_id, input_params, initiated_by) = match registry.get(execution_id) {
        Some(e) => (
            e.action_id.clone(),
            e.target_resource_id.clone(),
            e.input_params.clone(),
            e.initiated_by.clone(),
        ),
        None => return,
    };
    let handler = get_handler(&action_id);
    // target &mut -> handler mutate twin
    let result = match handler {
        Some(h) => match topology.nodes.iter_mut().find(|n| n.resource_id == target_id) {
            Some(t) => {
                let ctx = ExecutionContext {
                    execution_id: execution_id.to_string(),
                    initiated_by,
                    auto_rollback,
                };
                h(t, &input_params, &ctx)
            }
            None => json!({ "success": false, "error": "target not found in current topology" }),
        },
        None => json!({ "success": false, "error": format!("no handler for action {action_id}") }),
    };
    let succeeded = result.get("success").and_then(Value::as_bool).unwrap_or(false);

    let now = now_iso();
    if let Some(exec) = registry.get_mut(execution_id) {
        exec.status = RecoveryStatus::Executing;
        exec.executed_at = now.clone();
        exec.completed_at = now;
        exec.result = result;
        exec.status = if succeeded {
            RecoveryStatus::Succeeded
        } else {
            RecoveryStatus::Failed
        };
    }

    if succeeded && verify {
        verify_and_maybe_rollback(registry, execution_id, topology, auto_rollback, VERIFIERS);
    }
}

/// 跑 verifier + 设置 verify_status;verify_failed + auto_rollback + 有 rollback_action_id -> 自动回滚。
///
/// `verifiers` 可注入(测试用 fake failing verifier 触发 auto-rollback)。
fn verify_and_maybe_rollback(
    registry: &mut ExecutionRegistry,
    execution_id: &str,
    topology: &mut Topology,
    auto_rollback: bool,
    verifiers: &[(&str, VerifierFn)],
) {
    let (action_id, target_id, input_params, result_clone) = match registry.get(execution_id) {
        Some(e) => (
            e.action_id.clone(),
            e.target_resource_id.clone(),
            e.input_params.clone(),
            e.result.clone(),
        ),
        None => return,
    };
    let target: Option<&ResolvedNode> = topology.nodes.iter().find(|n| n.resource_id == target_id);
    let ctx = ExecutionContext {
        execution_id: execution_id.to_string(),
        initiated_by: String::new(),
        auto_rollback,
    };
    let verdict = match target {
        Some(t) => run_verifier(&action_id, t, &input_params, &result_clone, &ctx, verifiers),
        None => VerifierVerdict::make(false, "error", json!(null), json!(null), "target not found"),
    };
    let verify_status = map_verdict(&verdict);
    let verdict_json = serde_json::to_value(&verdict).unwrap_or(json!(null));
    let now = now_iso();
    if let Some(exec) = registry.get_mut(execution_id) {
        exec.verify_result = verdict_json;
        exec.verified_at = now;
        exec.verify_status = verify_status;
    }

    if verify_status == VerifyStatus::Failed && auto_rollback {
        let rollback_action_id = get_action(&action_id).and_then(|a| a.rollback_action_id);
        match rollback_action_id {
            Some(rb_aid) => {
                let reason = format!(
                    "auto rollback: verify_failed ({})",
                    if verdict.message.is_empty() { "no message" } else { verdict.message.as_str() }
                );
                let rb = do_rollback(registry, execution_id, rb_aid, topology, "auto-verifier", &reason, true);
                if let Ok(rb) = rb {
                    if let Some(exec) = registry.get_mut(execution_id) {
                        push_to_result(
                            &mut exec.result,
                            "auto_rollback",
                            json!({
                                "triggered": true,
                                "rollback_execution_id": rb.execution_id,
                                "rollback_status": format!("{:?}", rb.status),
                            }),
                        );
                    }
                }
            }
            None => {
                if let Some(exec) = registry.get_mut(execution_id) {
                    push_warning(&mut exec.result, "verify_failed but action has no rollback_action_id, manual intervention needed");
                }
            }
        }
    }
}

/// verdict -> VerifyStatus(对齐 reference `_verify_and_maybe_rollback` 映射)。
fn map_verdict(v: &VerifierVerdict) -> VerifyStatus {
    match v.predicate.as_str() {
        "skipped" => VerifyStatus::Skipped,
        "not_supported" => VerifyStatus::NotSupported,
        "error" => VerifyStatus::Error,
        _ => {
            if v.passed {
                VerifyStatus::Passed
            } else {
                VerifyStatus::Failed
            }
        }
    }
}

/// 派生反向参数(对齐 reference `_derive_rollback_params`)。scale_deployment -> 反向 delta;其它复用。
fn derive_rollback_params(action_id: &str, original_params: &Value) -> Value {
    if action_id == "scale_deployment" {
        let delta = original_params
            .get("replicas_delta")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        json!({ "replicas_delta": -delta })
    } else {
        original_params.clone()
    }
}

/// 从 target 派生 cluster_id(对齐 reference)。
pub(crate) fn derive_cluster_id(target_id: &str, target_node: Option<&ResolvedNode>) -> String {
    if let Some(n) = target_node {
        if let Ok(Value::Object(m)) = serde_json::from_str::<Value>(&n.attributes_json) {
            if let Some(cid) = m.get("cluster_id").and_then(Value::as_str) {
                if !cid.is_empty() {
                    return cid.to_string();
                }
            }
        }
    }
    let parts: Vec<&str> = target_id.split(':').collect();
    if parts.len() >= 2 && !parts[1].is_empty() {
        parts[1].to_string()
    } else {
        String::new()
    }
}

/// 往 result(Value object)设一个 key(若 result 是 object)。
fn push_to_result(result: &mut Value, key: &str, val: Value) {
    if let Value::Object(m) = result {
        m.insert(key.to_string(), val);
    }
}

/// 往 result.warnings 追加一条(无则建数组)。
fn push_warning(result: &mut Value, msg: &str) {
    if !result.is_object() {
        return;
    }
    match result.get_mut("warnings") {
        Some(Value::Array(a)) => a.push(json!(msg)),
        _ => {
            result["warnings"] = json!([msg]);
        }
    }
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn topo() -> Topology {
        crate::cascade::tests::fixture_topology()
    }

    #[test]
    fn low_risk_executes_synchronously_and_verifies() {
        let mut reg = ExecutionRegistry::new();
        let mut t = topo();
        let e = execute(
            &mut reg, "scale_deployment", "deploy:order-api",
            &json!({ "replicas_delta": 2 }), &mut t, "tester", "",
        )
        .expect("execute low risk");
        assert_eq!(e.status, RecoveryStatus::Succeeded);
        assert_eq!(e.result["new_replicas"], 5);
        // verify 跑了:passed(desired=available=5)
        assert_eq!(e.verify_status, VerifyStatus::Passed);
    }

    #[test]
    fn medium_risk_awaits_approval_no_verify() {
        let mut reg = ExecutionRegistry::new();
        let mut t = topo();
        let e = execute(
            &mut reg, "restart_pod", "pod:order-api-1", &json!({}), &mut t, "tester", "",
        )
        .expect("execute medium risk");
        assert_eq!(e.status, RecoveryStatus::AwaitingApproval);
        assert_eq!(e.verify_status, VerifyStatus::NotRun); // 未跑 handler/verifier
    }

    #[test]
    fn high_risk_awaits_approval() {
        let mut reg = ExecutionRegistry::new();
        let mut t = topo();
        let e = execute(
            &mut reg, "rollback_deployment", "deploy:order-api", &json!({}), &mut t, "tester", "",
        )
        .expect("execute high risk");
        assert_eq!(e.status, RecoveryStatus::AwaitingApproval);
    }

    #[test]
    fn unknown_action_404() {
        let mut reg = ExecutionRegistry::new();
        let mut t = topo();
        let err = execute(&mut reg, "nonexistent", "deploy:order-api", &json!({}), &mut t, "tester", "").unwrap_err();
        assert_eq!(err.code, 404);
    }

    #[test]
    fn dry_run_fail_400() {
        let mut reg = ExecutionRegistry::new();
        let mut t = topo();
        let err = execute(&mut reg, "restart_pod", "deploy:order-api", &json!({}), &mut t, "tester", "").unwrap_err();
        assert_eq!(err.code, 400);
    }

    #[test]
    fn confirm_runs_handler_and_verifies() {
        let mut reg = ExecutionRegistry::new();
        let mut t = topo();
        let e = execute(&mut reg, "restart_pod", "pod:order-api-1", &json!({}), &mut t, "tester", "").expect("execute");
        let confirmed = confirm_execution(&mut reg, &e.execution_id, &mut t, "ok").expect("confirm");
        assert_eq!(confirmed.status, RecoveryStatus::Succeeded);
        assert_eq!(confirmed.result["new_restart_count"], 1);
        assert_eq!(confirmed.verify_status, VerifyStatus::Passed);
    }

    #[test]
    fn confirm_non_awaiting_409() {
        let mut reg = ExecutionRegistry::new();
        let mut t = topo();
        let e = execute(&mut reg, "scale_deployment", "deploy:order-api", &json!({"replicas_delta":1}), &mut t, "tester", "").expect("execute");
        let err = confirm_execution(&mut reg, &e.execution_id, &mut t, "").unwrap_err();
        assert_eq!(err.code, 409);
    }

    #[test]
    fn cancel_marks_rejected() {
        let mut reg = ExecutionRegistry::new();
        let mut t = topo();
        let e = execute(&mut reg, "restart_pod", "pod:order-api-1", &json!({}), &mut t, "tester", "").expect("execute");
        let canceled = cancel_execution(&mut reg, &e.execution_id).expect("cancel");
        assert_eq!(canceled.status, RecoveryStatus::Rejected);
    }

    #[test]
    fn scale_rollback_reverses_delta_reads_mutated_state() {
        // scale +2 (3->5) -> rollback scale -2(读 mutated desired=5 -> 3,正确反转)
        let mut reg = ExecutionRegistry::new();
        let mut t = topo();
        let e = execute(&mut reg, "scale_deployment", "deploy:order-api", &json!({"replicas_delta":2}), &mut t, "tester", "").expect("execute");
        assert_eq!(e.result["new_replicas"], 5);
        let rb = rollback(&mut reg, &e.execution_id, &mut t, "tester", "").expect("rollback");
        assert_eq!(rb.status, RecoveryStatus::Succeeded);
        assert_eq!(rb.input_params["replicas_delta"], -2);
        assert_eq!(rb.result["new_replicas"], 3); // 读 mutated 5 -> 5-2=3(正确反转,非重应用到原 3)
        let orig = reg.get(&e.execution_id).unwrap();
        assert_eq!(orig.status, RecoveryStatus::RolledBack);
    }

    #[test]
    fn rollback_only_succeeded_409() {
        let mut reg = ExecutionRegistry::new();
        let mut t = topo();
        let e = execute(&mut reg, "restart_pod", "pod:order-api-1", &json!({}), &mut t, "tester", "").expect("execute");
        let err = rollback(&mut reg, &e.execution_id, &mut t, "tester", "").unwrap_err();
        assert_eq!(err.code, 409);
    }

    #[test]
    fn rollback_idempotent_409() {
        let mut reg = ExecutionRegistry::new();
        let mut t = topo();
        let e = execute(&mut reg, "scale_deployment", "deploy:order-api", &json!({"replicas_delta":1}), &mut t, "tester", "").expect("execute");
        let _rb1 = rollback(&mut reg, &e.execution_id, &mut t, "tester", "").expect("rollback 1");
        let err = rollback(&mut reg, &e.execution_id, &mut t, "tester", "").unwrap_err();
        assert_eq!(err.code, 409);
        assert!(err.message.contains("only succeeded"));
    }

    #[test]
    fn rollback_no_rollback_action_id_400() {
        let mut reg = ExecutionRegistry::new();
        let mut t = topo();
        let e = execute(&mut reg, "restart_pod", "pod:order-api-1", &json!({}), &mut t, "tester", "").expect("execute");
        confirm_execution(&mut reg, &e.execution_id, &mut t, "").expect("confirm");
        let err = rollback(&mut reg, &e.execution_id, &mut t, "tester", "").unwrap_err();
        assert_eq!(err.code, 400);
        assert!(err.message.contains("no rollback_action_id"));
    }

    #[test]
    fn verify_failed_triggers_auto_rollback() {
        // 注入 fake failing verifier -> verify_failed -> auto-rollback(scale 有 rollback_action_id)
        let mut reg = ExecutionRegistry::new();
        let mut t = topo();
        let e = execute(&mut reg, "scale_deployment", "deploy:order-api", &json!({"replicas_delta":2}), &mut t, "tester", "").expect("execute");
        // 先把 verify_status 重置为 NotRun(实际 execute 已 verify passed;这里直接调
        // verify_and_maybe_rollback 用 failing verifier 模拟 verify_failed)
        // 用 failing verifier 集合
        fn fail(_t: &ResolvedNode, _p: &Value, _r: &Value, _c: &ExecutionContext) -> VerifierVerdict {
            VerifierVerdict::make(false, "fake", json!(null), json!(null), "injected failure")
        }
        let failing: &[(&str, VerifierFn)] = &[("scale_deployment", fail)];
        // e 已 succeeded + verify passed;重置 verify_status 后再跑 failing verifier
        {
            let ex = reg.get_mut(&e.execution_id).unwrap();
            ex.verify_status = VerifyStatus::NotRun;
        }
        verify_and_maybe_rollback(&mut reg, &e.execution_id, &mut t, true, failing);
        let ex = reg.get(&e.execution_id).unwrap();
        assert_eq!(ex.verify_status, VerifyStatus::Failed);
        // auto_rollback 触发:原 execution 翻 rolled_back
        assert_eq!(ex.status, RecoveryStatus::RolledBack);
        assert!(ex.result.get("auto_rollback").is_some());
        assert_eq!(ex.result["auto_rollback"]["triggered"], true);
        // 反向 exec 标 auto_rollback_origin
        let rb_id = ex.result["auto_rollback"]["rollback_execution_id"].as_str().unwrap().to_string();
        let rb = reg.get(&rb_id).unwrap();
        assert_eq!(rb.result["auto_rollback_origin"], e.execution_id);
    }

    #[test]
    fn verify_failed_without_rollback_action_warns() {
        // restart_pod 无 rollback_action_id -> verify_failed 不回滚,加 warning
        let mut reg = ExecutionRegistry::new();
        let mut t = topo();
        let e = execute(&mut reg, "restart_pod", "pod:order-api-1", &json!({}), &mut t, "tester", "").expect("execute");
        confirm_execution(&mut reg, &e.execution_id, &mut t, "").expect("confirm"); // succeeded
        fn fail(_t: &ResolvedNode, _p: &Value, _r: &Value, _c: &ExecutionContext) -> VerifierVerdict {
            VerifierVerdict::make(false, "fake", json!(null), json!(null), "injected")
        }
        let failing: &[(&str, VerifierFn)] = &[("restart_pod", fail)];
        verify_and_maybe_rollback(&mut reg, &e.execution_id, &mut t, true, failing);
        let ex = reg.get(&e.execution_id).unwrap();
        assert_eq!(ex.verify_status, VerifyStatus::Failed);
        assert_eq!(ex.status, RecoveryStatus::Succeeded); // 未回滚
        assert!(ex.result.get("warnings").is_some());
    }

    #[test]
    fn reverify_no_auto_rollback() {
        let mut reg = ExecutionRegistry::new();
        let mut t = topo();
        let e = execute(&mut reg, "scale_deployment", "deploy:order-api", &json!({"replicas_delta":1}), &mut t, "tester", "").expect("execute");
        // reverify 用默认 verifier(passed)-> 不触发 auto-rollback
        let r = reverify(&mut reg, &e.execution_id, &mut t).expect("reverify");
        assert_eq!(r.verify_status, VerifyStatus::Passed);
        assert_eq!(r.status, RecoveryStatus::Succeeded); // 未回滚
    }

    #[test]
    fn reverify_only_on_succeeded_or_rolled_back_409() {
        let mut reg = ExecutionRegistry::new();
        let mut t = topo();
        let e = execute(&mut reg, "restart_pod", "pod:order-api-1", &json!({}), &mut t, "tester", "").expect("execute");
        // awaiting -> 409
        let err = reverify(&mut reg, &e.execution_id, &mut t).unwrap_err();
        assert_eq!(err.code, 409);
    }

    #[test]
    fn list_filtered_sorts_newest_first() {
        let mut reg = ExecutionRegistry::new();
        let mut t = topo();
        let e1 = execute(&mut reg, "scale_deployment", "deploy:order-api", &json!({"replicas_delta":1}), &mut t, "tester", "").expect("execute");
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let e2 = execute(&mut reg, "restart_pod", "pod:order-api-1", &json!({}), &mut t, "tester", "").expect("execute");
        let listed = reg.list_filtered(None, None, None, 10);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].execution_id, e2.execution_id);
        assert_eq!(listed[1].execution_id, e1.execution_id);
        let succeeded = reg.list_filtered(Some(RecoveryStatus::Succeeded), None, None, 10);
        assert_eq!(succeeded.len(), 1);
        assert_eq!(succeeded[0].execution_id, e1.execution_id);
    }
}
