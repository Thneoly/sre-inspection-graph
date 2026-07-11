//! RecoveryExecution 生命周期编排(复刻 `reference/app/recovery/execution.py`)。
//!
//! 3.2 范围:execute 管线 + 单机确认门(confirm/cancel)+ rollback(skip re-approval)
//! + 8 mock handler。**不含** verifier / auto-rollback / chain(3.3)。
//!
//! ## 与 reference 的差异
//!
//! - **I/O-free + 显式 registry**:reference 读全局 DSS `store`;本模块用 [`ExecutionRegistry`]
//!   (in-memory HashMap)作入参,orchestration 层(3.6 Tauri/CLI)负责从 storage 加载 + 持久化。
//! - **单机确认门**([[phase3_approval-decision]]):无 `ApprovalRequest` 独立实体 / 无 TTL /
//!   无 approver_team。medium/high -> `awaiting_approval`;[`confirm_execution`] = 操作者确认 ->
//!   跑 handler;[`cancel_execution`] = 拒绝 -> `rejected`。
//! - **rollback skip re-approval**:反向 execution 直接 `executing`(reference 同设计)。
//! - 不写 Neo4j(本 port 无 Neo4j;3.6 落 SQLite)。
//! - verify / auto-rollback 留 3.3;3.2 `run_handler` 不跑 verifier。

#![allow(missing_docs)]

use std::collections::HashMap;

use engine_identity::{ResolvedNode, Topology};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::action_defs::{get_action, RiskLevel};
use crate::cascade::dry_run;
use crate::handlers::{get_handler, is_executable};
use crate::models::{ExecutionContext, ExecutionError, RecoveryExecution, RecoveryStatus};

/// in-memory execution 注册表(对齐 reference DSS `store.executions`,但显式非全局)。
///
/// orchestration 层(3.6)从 storage 加载 [`RecoveryExecution`] 列表 -> [`from_executions`]
/// 构造 -> 调 pipeline 函数 -> 持久化变更。测试直接 [`new`]。
#[derive(Debug, Clone, Default)]
pub struct ExecutionRegistry {
    executions: HashMap<String, RecoveryExecution>,
}

impl ExecutionRegistry {
    /// 空注册表。
    pub fn new() -> Self {
        Self {
            executions: HashMap::new(),
        }
    }

    /// 从已加载的 execution 列表构造(orchestration 从 storage 恢复用)。
    pub fn from_executions(es: Vec<RecoveryExecution>) -> Self {
        Self {
            executions: es.into_iter().map(|e| (e.execution_id.clone(), e)).collect(),
        }
    }

    /// 取 execution。
    pub fn get(&self, id: &str) -> Option<&RecoveryExecution> {
        self.executions.get(id)
    }

    /// 取 execution(可变)。
    pub fn get_mut(&mut self, id: &str) -> Option<&mut RecoveryExecution> {
        self.executions.get_mut(id)
    }

    /// 插入 / 覆盖(按 execution_id)。
    pub fn insert(&mut self, e: RecoveryExecution) {
        self.executions.insert(e.execution_id.clone(), e);
    }

    /// 全部 execution。
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

    /// 数量。
    pub fn len(&self) -> usize {
        self.executions.len()
    }

    /// 是否空。
    pub fn is_empty(&self) -> bool {
        self.executions.is_empty()
    }
}

/// 执行恢复动作。
///
/// - low 风险 + 不需审批 -> 同步跑 handler,返 `succeeded`/`failed`。
/// - medium/high 或 requires_approval -> 返 `awaiting_approval`(调用方据此走确认门)。
///
/// 抛 [`ExecutionError`] = 前置校验失败(动作不存在 404 / 无 handler 501 / dry-run 失败 400)。
pub fn execute(
    registry: &mut ExecutionRegistry,
    action_id: &str,
    target_resource_id: &str,
    input_params: &Value,
    topology: &Topology,
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

    // dry-run(复用 cascade)
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

    let execution = RecoveryExecution {
        execution_id: Uuid::new_v4().to_string(),
        action_id: action_id.to_string(),
        target_resource_id: target_resource_id.to_string(),
        target_resource_type: target_node
            .map(|n| n.resource_type.clone())
            .unwrap_or_else(|| action.target_type.to_string()),
        input_params: input_params.clone(),
        dry_run_result: dry_result,
        status: if needs_approval {
            RecoveryStatus::AwaitingApproval
        } else {
            RecoveryStatus::Executing
        },
        initiated_by: initiated_by.to_string(),
        request_reason: request_reason.to_string(),
        initiated_at: now.clone(),
        cluster_id: derive_cluster_id(target_resource_id, target_node),
        ..Default::default()
    };

    let execution_id = execution.execution_id.clone();
    registry.insert(execution);

    if needs_approval {
        // 待确认;返回 awaiting_approval(3.6 Tauri 据此走 202)
        return Ok(registry
            .get(&execution_id)
            .cloned()
            .expect("just inserted"));
    }

    // low 风险 -> 同步跑 handler
    run_handler(registry, &execution_id, topology);
    Ok(registry.get(&execution_id).cloned().expect("just inserted"))
}

/// 确认执行(单机确认门 = 操作者点确认)。
///
/// `awaiting_approval` -> 跑 handler -> `succeeded`/`failed`。
/// 非 `awaiting_approval` -> 409。
pub fn confirm_execution(
    registry: &mut ExecutionRegistry,
    execution_id: &str,
    topology: &Topology,
    approval_comment: &str,
) -> Result<RecoveryExecution, ExecutionError> {
    {
        let exec = registry
            .get(execution_id)
            .ok_or_else(|| ExecutionError::with_code(format!("execution not found: {execution_id}"), 404))?;
        if exec.status != RecoveryStatus::AwaitingApproval {
            return Err(ExecutionError::with_code(
                format!(
                    "execution status is {:?}, expected awaiting_approval",
                    exec.status
                ),
                409,
            ));
        }
    }
    {
        let exec = registry.get_mut(execution_id).expect("checked above");
        exec.approved_at = now_iso();
        exec.approval_comment = approval_comment.to_string();
    }
    run_handler(registry, execution_id, topology);
    Ok(registry.get(execution_id).cloned().expect("checked above"))
}

/// 取消执行(单机确认门 = 操作者拒绝)。
///
/// `awaiting_approval` -> `rejected`。非 `awaiting_approval` -> 409。
pub fn cancel_execution(
    registry: &mut ExecutionRegistry,
    execution_id: &str,
) -> Result<RecoveryExecution, ExecutionError> {
    let exec = registry
        .get_mut(execution_id)
        .ok_or_else(|| ExecutionError::with_code(format!("execution not found: {execution_id}"), 404))?;
    if exec.status != RecoveryStatus::AwaitingApproval {
        return Err(ExecutionError::with_code(
            format!(
                "execution status is {:?}, expected awaiting_approval",
                exec.status
            ),
            409,
        ));
    }
    exec.status = RecoveryStatus::Rejected;
    exec.completed_at = now_iso();
    Ok(registry.get(execution_id).cloned().expect("checked above"))
}

/// 回滚一个 `succeeded` execution。
///
/// 创建反向 execution(`reverses_execution_id` 指向原 execution),直接同步执行,
/// **不再二次审批**(reference 设计)。仅 `succeeded` 可回滚;`rolled_back` 后不可再回滚;
/// 动作无 `rollback_action_id` -> 400。
pub fn rollback(
    registry: &mut ExecutionRegistry,
    execution_id: &str,
    topology: &Topology,
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
            ExecutionError::with_code(
                format!("action '{action_id}' has no rollback_action_id"),
                400,
            )
        })?;
    do_rollback(
        registry,
        execution_id,
        rollback_action_id,
        topology,
        initiated_by,
        reason,
    )
}

/// 实际创建并执行 rollback execution(供 [`rollback`] 与 3.3 auto-rollback 复用)。
fn do_rollback(
    registry: &mut ExecutionRegistry,
    original_id: &str,
    rollback_action_id: &str,
    topology: &Topology,
    initiated_by: &str,
    reason: &str,
) -> Result<RecoveryExecution, ExecutionError> {
    // clone 原 execution 需要的字段(避免持有 registry 借用跨 dry_run)
    let (
        target_resource_id,
        target_resource_type,
        finding_id,
        input_params,
        cluster_id,
    ) = {
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

    let rollback_params = derive_rollback_params(&registry.get(original_id).expect("checked").action_id, &input_params);
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

    // 跑反向 handler(skip re-approval;3.2 不 verify)
    run_handler(registry, &rb_id, topology);

    // 回滚成功 -> 原 execution 标 rolled_back
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
    Ok(registry.get(&rb_id).cloned().expect("just inserted"))
}

/// 跑 handler,更新 execution 状态(3.2 不跑 verifier)。
fn run_handler(registry: &mut ExecutionRegistry, execution_id: &str, topology: &Topology) {
    // 1. 不可变读 execution + topology,算 result(释放借用后再 get_mut)
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
    let target: Option<&ResolvedNode> = topology
        .nodes
        .iter()
        .find(|n| n.resource_id == target_id);
    let result = match (handler, target) {
        (Some(h), Some(t)) => {
            let ctx = ExecutionContext {
                execution_id: execution_id.to_string(),
                initiated_by,
                auto_rollback: false,
            };
            h(t, &input_params, &ctx)
        }
        (Some(_), None) => json!({ "success": false, "error": "target not found in current topology" }),
        (None, _) => json!({ "success": false, "error": format!("no handler for action {action_id}") }),
    };
    let succeeded = result
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    // 2. 可变写 execution
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
}

/// 派生反向参数(对齐 reference `_derive_rollback_params`)。
///
/// scale_deployment -> `replicas_delta` 取反;其它 -> 复用原参数。
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
///
/// 优先 target attrs `cluster_id`,次回 target_id 第二段(`<type>:<cluster>:...`)。
fn derive_cluster_id(target_id: &str, target_node: Option<&ResolvedNode>) -> String {
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

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn topo() -> Topology {
        // 复用 cascade::tests 的 fixture_topology(同一张 9 节点 11 边图)
        crate::cascade::tests::fixture_topology()
    }

    #[test]
    fn low_risk_executes_synchronously() {
        // scale_deployment(low,不审批)-> 同步 succeeded
        let mut reg = ExecutionRegistry::new();
        let e = execute(
            &mut reg,
            "scale_deployment",
            "deploy:order-api",
            &json!({ "replicas_delta": 2 }),
            &topo(),
            "tester",
            "",
        )
        .expect("execute low risk");
        assert_eq!(e.status, RecoveryStatus::Succeeded);
        assert_eq!(e.result["success"], true);
        assert_eq!(e.result["new_replicas"], 5); // default desired_replicas=3 + 2
    }

    #[test]
    fn medium_risk_awaits_approval() {
        // restart_pod(medium,需审批)-> awaiting_approval
        let mut reg = ExecutionRegistry::new();
        let e = execute(
            &mut reg,
            "restart_pod",
            "pod:order-api-1",
            &json!({}),
            &topo(),
            "tester",
            "",
        )
        .expect("execute medium risk");
        assert_eq!(e.status, RecoveryStatus::AwaitingApproval);
        assert!(e.result.as_object().is_none_or(|o| o.is_empty())); // 未跑 handler
    }

    #[test]
    fn high_risk_awaits_approval() {
        let mut reg = ExecutionRegistry::new();
        let e = execute(
            &mut reg,
            "rollback_deployment",
            "deploy:order-api",
            &json!({}),
            &topo(),
            "tester",
            "",
        )
        .expect("execute high risk");
        assert_eq!(e.status, RecoveryStatus::AwaitingApproval);
    }

    #[test]
    fn unknown_action_404() {
        let mut reg = ExecutionRegistry::new();
        let err = execute(
            &mut reg,
            "nonexistent",
            "deploy:order-api",
            &json!({}),
            &topo(),
            "tester",
            "",
        )
        .unwrap_err();
        assert_eq!(err.code, 404);
        assert!(err.message.contains("unknown action_id"));
    }

    #[test]
    fn dry_run_fail_400() {
        // 类型不匹配:restart_pod on Deployment -> dry-run fail -> 400
        let mut reg = ExecutionRegistry::new();
        let err = execute(
            &mut reg,
            "restart_pod",
            "deploy:order-api",
            &json!({}),
            &topo(),
            "tester",
            "",
        )
        .unwrap_err();
        assert_eq!(err.code, 400);
        assert!(err.message.contains("dry-run validation failed"));
    }

    #[test]
    fn confirm_runs_handler_to_succeeded() {
        let mut reg = ExecutionRegistry::new();
        let e = execute(
            &mut reg,
            "restart_pod",
            "pod:order-api-1",
            &json!({}),
            &topo(),
            "tester",
            "",
        )
        .expect("execute");
        assert_eq!(e.status, RecoveryStatus::AwaitingApproval);
        let confirmed = confirm_execution(&mut reg, &e.execution_id, &topo(), "ok").expect("confirm");
        assert_eq!(confirmed.status, RecoveryStatus::Succeeded);
        assert_eq!(confirmed.result["success"], true);
        assert_eq!(confirmed.result["new_restart_count"], 1); // default 0 + 1
        assert!(!confirmed.approved_at.is_empty());
    }

    #[test]
    fn confirm_non_awaiting_409() {
        let mut reg = ExecutionRegistry::new();
        let e = execute(
            &mut reg,
            "scale_deployment",
            "deploy:order-api",
            &json!({ "replicas_delta": 1 }),
            &topo(),
            "tester",
            "",
        )
        .expect("execute"); // succeeded
        let err = confirm_execution(&mut reg, &e.execution_id, &topo(), "").unwrap_err();
        assert_eq!(err.code, 409);
    }

    #[test]
    fn cancel_marks_rejected() {
        let mut reg = ExecutionRegistry::new();
        let e = execute(
            &mut reg,
            "restart_pod",
            "pod:order-api-1",
            &json!({}),
            &topo(),
            "tester",
            "",
        )
        .expect("execute");
        let canceled = cancel_execution(&mut reg, &e.execution_id).expect("cancel");
        assert_eq!(canceled.status, RecoveryStatus::Rejected);
    }

    #[test]
    fn scale_rollback_reverses_delta() {
        // scale +2 (3->5) succeeded -> rollback(scale -2, 5->3) -> 原 execution rolled_back
        let mut reg = ExecutionRegistry::new();
        let e = execute(
            &mut reg,
            "scale_deployment",
            "deploy:order-api",
            &json!({ "replicas_delta": 2 }),
            &topo(),
            "tester",
            "",
        )
        .expect("execute");
        assert_eq!(e.status, RecoveryStatus::Succeeded);
        assert_eq!(e.result["new_replicas"], 5);

        let rb = rollback(&mut reg, &e.execution_id, &topo(), "tester", "").expect("rollback");
        assert_eq!(rb.status, RecoveryStatus::Succeeded);
        assert_eq!(rb.action_id, "scale_deployment");
        assert_eq!(rb.input_params["replicas_delta"], -2); // 反向 delta
        assert_eq!(rb.reverses_execution_id.as_deref(), Some(e.execution_id.as_str()));

        // 原 execution 翻 rolled_back
        let orig = reg.get(&e.execution_id).unwrap();
        assert_eq!(orig.status, RecoveryStatus::RolledBack);
        assert_eq!(orig.rollback_execution_id.as_deref(), Some(rb.execution_id.as_str()));
    }

    #[test]
    fn rollback_only_succeeded_409() {
        let mut reg = ExecutionRegistry::new();
        // awaiting_approval -> rollback 应 409
        let e = execute(
            &mut reg,
            "restart_pod",
            "pod:order-api-1",
            &json!({}),
            &topo(),
            "tester",
            "",
        )
        .expect("execute");
        let err = rollback(&mut reg, &e.execution_id, &topo(), "tester", "").unwrap_err();
        assert_eq!(err.code, 409);
        assert!(err.message.contains("only succeeded"));
    }

    #[test]
    fn rollback_idempotent_409() {
        let mut reg = ExecutionRegistry::new();
        let e = execute(
            &mut reg,
            "scale_deployment",
            "deploy:order-api",
            &json!({ "replicas_delta": 1 }),
            &topo(),
            "tester",
            "",
        )
        .expect("execute");
        let _rb1 = rollback(&mut reg, &e.execution_id, &topo(), "tester", "").expect("rollback 1");
        // 二次 rollback -> 409(原 execution 已 RolledBack,先撞 status!=Succeeded 检查)
        let err = rollback(&mut reg, &e.execution_id, &topo(), "tester", "").unwrap_err();
        assert_eq!(err.code, 409);
        assert!(err.message.contains("only succeeded"));
    }

    #[test]
    fn rollback_no_rollback_action_id_400() {
        // restart_pod rollback_action_id=None -> 400
        let mut reg = ExecutionRegistry::new();
        let e = execute(
            &mut reg,
            "restart_pod",
            "pod:order-api-1",
            &json!({}),
            &topo(),
            "tester",
            "",
        )
        .expect("execute");
        // 先 confirm 到 succeeded
        confirm_execution(&mut reg, &e.execution_id, &topo(), "").expect("confirm");
        // 再 rollback -> 400(restart_pod 无 rollback_action_id)
        let err = rollback(&mut reg, &e.execution_id, &topo(), "tester", "").unwrap_err();
        assert_eq!(err.code, 400);
        assert!(err.message.contains("no rollback_action_id"));
    }

    #[test]
    fn list_filtered_sorts_newest_first() {
        let mut reg = ExecutionRegistry::new();
        let e1 = execute(
            &mut reg,
            "scale_deployment",
            "deploy:order-api",
            &json!({ "replicas_delta": 1 }),
            &topo(),
            "tester",
            "",
        )
        .expect("execute");
        // 令 e2 的 initiated_at 晚于 e1(chrono now 单调,但同秒可能相等;加个明显晚的)
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let e2 = execute(
            &mut reg,
            "restart_pod",
            "pod:order-api-1",
            &json!({}),
            &topo(),
            "tester",
            "",
        )
        .expect("execute");
        let listed = reg.list_filtered(None, None, None, 10);
        assert_eq!(listed.len(), 2);
        // 新到旧:e2 在前
        assert_eq!(listed[0].execution_id, e2.execution_id);
        assert_eq!(listed[1].execution_id, e1.execution_id);
        // filter by status
        let succeeded = reg.list_filtered(Some(RecoveryStatus::Succeeded), None, None, 10);
        assert_eq!(succeeded.len(), 1);
        assert_eq!(succeeded[0].execution_id, e1.execution_id);
    }
}
