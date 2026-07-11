//! RecoveryExecution 模型 + 状态枚举(复刻 `reference/app/datasource/models.py`)。
//!
//! ## 与 reference 的差异
//!
//! - **审批**:桌面单机确认门(见 [[phase3_approval-decision]] / doc/14 §9)。**不设
//!   `ApprovalRequest` 独立实体**,审批态折叠进 [`RecoveryStatus`](awaiting_approval /
//!   approved / rejected);丢弃 `approver_team`、`expiry_at`、24h TTL、多人 approve-reject。
//!   3.2 `approval` 模块的 confirm/cancel 即对应 reference 的 approve/reject。
//! - `verify_status` 默认 [`VerifyStatus::NotRun`](3.3 verifiers 填),reference 用空串。
//! - `dry_run_result` 用 typed [`DryRunResult`](非 dict),`input_params`/`result`/
//!   `verify_result` 用 `serde_json::Value`(异构,storage 落 JSON 文本)。

#![allow(missing_docs)]

use serde::{Deserialize, Serialize};

use crate::cascade::DryRunResult;

/// 执行生命周期状态(对齐 reference `RecoveryExecution.status`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryStatus {
    /// 刚创建(本 port 不单独走,execute 原子到下一态)。
    Pending,
    /// dry-run 通过(本 port 不单独走)。
    DryRunOk,
    /// 待审批(单机确认门:medium/high 风险)。
    AwaitingApproval,
    /// 已确认(瞬态,confirm 后立即 executing)。
    Approved,
    /// 已取消(操作者拒绝确认)。
    Rejected,
    /// 执行中。
    Executing,
    /// 执行成功。
    Succeeded,
    /// 执行失败(handler 返 success=false 或前置错)。
    Failed,
    /// 已回滚(被 rollback execution 反转)。
    RolledBack,
}

/// 验证状态(对齐 reference `verify_status`;3.3 verifiers 填)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyStatus {
    /// 未验证(默认;reference 用空串)。
    #[default]
    NotRun,
    /// 验证通过。
    Passed,
    /// 验证失败。
    Failed,
    /// 跳过。
    Skipped,
    /// 动作不支持验证。
    NotSupported,
    /// 验证超时。
    Timeout,
    /// 验证出错。
    Error,
}

/// handler 执行上下文(对齐 reference `context` dict)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionContext {
    /// 触发本次 handler 的 execution_id。
    pub execution_id: String,
    /// 发起人。
    pub initiated_by: String,
    /// 是否允许 verify_failed -> 自动回滚(3.3 用;3.2 始终 false)。
    pub auto_rollback: bool,
}

/// 一次恢复动作的执行实例(对齐 reference `RecoveryExecution`)。
///
/// 生命周期(3.2 范围):
/// ```text
/// execute:  ┬─ low 风险 ─-> executing -> succeeded/failed
///           └─ medium/high -> awaiting_approval
///                                  ┬─ confirm -> executing -> succeeded/failed
///                                  └─ cancel  -> rejected
/// rollback: succeeded -> [反向 execution,skip re-approval] -> rolled_back
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryExecution {
    /// UUID。
    pub execution_id: String,
    /// 动作 ID(引用 [`crate::action_defs::ActionDef::action_id`])。
    pub action_id: String,
    /// 实际目标资源。
    pub target_resource_id: String,
    /// 目标资源类型。
    pub target_resource_type: String,
    /// 触发动作的 InspectionFinding(可空,3.x reports 模块用)。
    pub finding_id: Option<String>,
    /// 输入参数(JSON object)。
    pub input_params: serde_json::Value,
    /// dry-run 结果。
    pub dry_run_result: DryRunResult,
    /// 当前状态。
    pub status: RecoveryStatus,
    /// 发起人。
    pub initiated_by: String,
    /// 发起理由。
    pub request_reason: String,
    /// 发起时间(ISO8601)。
    pub initiated_at: String,
    /// 执行开始时间(ISO8601)。
    pub executed_at: String,
    /// 完成时间(ISO8601)。
    pub completed_at: String,
    /// handler 结果(flat dict:{success, old_X, new_X, ...})。
    pub result: serde_json::Value,
    /// 若本 execution 被回滚,指向 rollback execution_id。
    pub rollback_execution_id: Option<String>,
    /// 若本 execution 是回滚动作,指向被回滚的原 execution_id。
    pub reverses_execution_id: Option<String>,
    /// 目标所属集群(从 target_id 第二段解析)。
    pub cluster_id: String,
    /// 验证状态(3.3 填)。
    pub verify_status: VerifyStatus,
    /// 验证结果(3.3 填)。
    pub verify_result: serde_json::Value,
    /// 验证时间(3.3 填)。
    pub verified_at: String,
    /// 所属 chain(3.3 填)。
    pub chain_id: String,
    /// 在 chain 中的步骤索引(-1 = 非链步骤;3.3 填)。
    pub chain_step_index: i32,
    /// 确认备注(单机确认门:操作者确认时留的 note)。
    pub approval_comment: String,
    /// 确认时间(ISO8601)。
    pub approved_at: String,
}

/// 执行流程异常。`message` 直接给用户看;`code` 是 HTTP-like(404/400/409/501),
/// 3.6 Tauri command 据此映射返回。
#[derive(Debug, Clone)]
pub struct ExecutionError {
    /// 人读消息。
    pub message: String,
    /// HTTP-like code(默认 400)。
    pub code: u16,
}

impl ExecutionError {
    /// 新建(默认 code=400)。
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 400,
        }
    }

    /// 带 code 新建。
    pub fn with_code(message: impl Into<String>, code: u16) -> Self {
        Self {
            message: message.into(),
            code,
        }
    }
}

impl std::fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for ExecutionError {}

impl Default for RecoveryExecution {
    fn default() -> Self {
        Self {
            execution_id: String::new(),
            action_id: String::new(),
            target_resource_id: String::new(),
            target_resource_type: String::new(),
            finding_id: None,
            input_params: serde_json::Value::Object(serde_json::Map::new()),
            dry_run_result: DryRunResult {
                action_id: String::new(),
                action_name: None,
                target_resource_id: String::new(),
                target_resource_type: None,
                target_resource_name: None,
                target_valid: false,
                validation_error: None,
                affected_resources: vec![],
                affected_count: 0,
                estimated_duration_seconds: 0,
                estimated_sla_impact: String::new(),
                warnings: vec![],
                rollback_action_id: None,
                rollback_input_params: None,
                risk_level: None,
                requires_approval: None,
            },
            status: RecoveryStatus::Pending,
            initiated_by: String::new(),
            request_reason: String::new(),
            initiated_at: String::new(),
            executed_at: String::new(),
            completed_at: String::new(),
            result: serde_json::Value::Object(serde_json::Map::new()),
            rollback_execution_id: None,
            reverses_execution_id: None,
            cluster_id: String::new(),
            verify_status: VerifyStatus::NotRun,
            verify_result: serde_json::Value::Object(serde_json::Map::new()),
            verified_at: String::new(),
            chain_id: String::new(),
            chain_step_index: -1,
            approval_comment: String::new(),
            approved_at: String::new(),
        }
    }
}
