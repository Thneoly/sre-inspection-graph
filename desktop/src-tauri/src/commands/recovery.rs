//! recovery commands - 把 `engine_recovery`(PRD-001)暴露给前端(Phase 3.6)。
//!
//! 命令面镜像 reference `app/routers/recovery.py`(HTTP -> Tauri invoke)。审批语义
//! 折叠成单机确认门:[[phase3-approval-decision]] / doc/14 §9 -- reference 的
//! `POST /approvals/{id}/approve|reject` -> [`confirm_recovery_execution`] /
//! [`cancel_recovery_execution`];无 `ApprovalRequest` 实体 / TTL / approver_team。
//!
//! ## mutable twin + 持久化
//!
//! `execute`/`confirm`/`rollback`/`reverify`/`execute_chain`/`confirm_chain` 接
//! `&mut Topology`(handler 把动作生效写回 twin `attributes_json`)。命令从 storage
//! 读 materialized topology 的 **owned clone** 作 twin(handler mutation 是 mock
//! 模拟,**不写回** materialized 表;真实集群态只由 sync 更新),调用后丢弃。
//!
//! 每次 mutation 后把 execution(s)/chain upsert 回 SQLite(重启恢复)。std
//! `Mutex`Guard 不跨 await:engine 调用(同步)在锁内完成,storage upsert(异步)
//! 在锁外。
//!
//! ## DTO 策略
//!
//! `RecoveryExecution`/`RecoveryChain`/`DryRunResult`/`ActionDef`/`ActionSuggestion`
//! 均 `derive(Serialize)` -> 命令直接返 engine 类型(区别于 `wasm.rs` 的 Fact-DTO
//! 解耦;这些类型已 serde 化,手写 DTO 是冗余)。`ChainTemplate`/`ChainStep` 未
//! Serialize(`&'static` 构造)-> 用 [`ChainTemplateDto`]。

use serde::Serialize;
use serde_json::Value;
use tauri::State;

use crate::AppState;

// ===== DTO =====

/// `ChainTemplate` 的 serde 镜像(`ChainTemplate` 未 Serialize)。
#[derive(Debug, Clone, Serialize)]
pub struct ChainStepDto {
    pub action_id: String,
    pub params: Value,
    pub verify_required: bool,
}

/// `ChainTemplate` 的 serde 镜像。
#[derive(Debug, Clone, Serialize)]
pub struct ChainTemplateDto {
    pub template_id: String,
    pub name: String,
    pub description: String,
    pub target_type: String,
    pub on_failure: engine_recovery::OnFailureStrategy,
    pub steps: Vec<ChainStepDto>,
}

impl From<engine_recovery::ChainTemplate> for ChainTemplateDto {
    fn from(t: engine_recovery::ChainTemplate) -> Self {
        Self {
            template_id: t.template_id.to_string(),
            name: t.name.to_string(),
            description: t.description.to_string(),
            target_type: t.target_type.to_string(),
            on_failure: t.on_failure,
            steps: t
                .steps
                .into_iter()
                .map(|s| ChainStepDto {
                    action_id: s.action_id.to_string(),
                    params: s.params,
                    verify_required: s.verify_required,
                })
                .collect(),
        }
    }
}

// ===== 解析 helper =====

fn parse_risk(s: &Option<String>) -> Option<engine_recovery::RiskLevel> {
    s.as_deref().and_then(|v| match v {
        "low" => Some(engine_recovery::RiskLevel::Low),
        "medium" => Some(engine_recovery::RiskLevel::Medium),
        "high" => Some(engine_recovery::RiskLevel::High),
        _ => None,
    })
}

fn parse_recovery_status(s: &str) -> Option<engine_recovery::RecoveryStatus> {
    serde_json::from_str(&format!("\"{s}\"")).ok()
}

fn parse_chain_status(s: &str) -> Option<engine_recovery::ChainStatus> {
    serde_json::from_str(&format!("\"{s}\"")).ok()
}

fn parse_on_failure(s: &str) -> Option<engine_recovery::OnFailureStrategy> {
    serde_json::from_str(&format!("\"{s}\"")).ok()
}

/// 持久化一个 execution + (若 auto-rollback 触发)其 rollback execution。
async fn persist_exec(state: &State<'_, AppState>, exec: &engine_recovery::RecoveryExecution) -> Result<(), String> {
    state
        .storage
        .upsert_recovery_execution(exec)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(rb_id) = &exec.rollback_execution_id {
        let rb = {
            let reg = state.recovery_executions.lock().await;
            reg.get(rb_id).cloned()
        };
        if let Some(rb) = rb {
            state
                .storage
                .upsert_recovery_execution(&rb)
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// 持久化 chain + 其全部 step execution。
async fn persist_chain(state: &State<'_, AppState>, chain: &engine_recovery::RecoveryChain) -> Result<(), String> {
    state
        .storage
        .upsert_recovery_chain(chain)
        .await
        .map_err(|e| e.to_string())?;
    let step_ids = chain.step_executions.clone();
    let execs: Vec<engine_recovery::RecoveryExecution> = {
        let reg = state.recovery_executions.lock().await;
        step_ids.iter().filter_map(|id| reg.get(id).cloned()).collect()
    };
    for e in &execs {
        state
            .storage
            .upsert_recovery_execution(e)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ===== actions / dry-run =====

/// 列动作模板(可按 target_type / category / risk_level 过滤)。
#[tauri::command]
pub async fn list_recovery_actions(
    target_type: Option<String>,
    category: Option<String>,
    risk_level: Option<String>,
) -> Vec<engine_recovery::ActionDef> {
    engine_recovery::list_actions_filtered(target_type.as_deref(), category.as_deref(), parse_risk(&risk_level))
        .into_iter()
        .copied()
        .collect()
}

/// 取单个动作模板。
#[tauri::command]
pub async fn get_recovery_action(action_id: String) -> Result<engine_recovery::ActionDef, String> {
    engine_recovery::get_action(&action_id)
        .copied()
        .ok_or_else(|| format!("[404] unknown action_id: {action_id}"))
}

/// 预演(dry-run)动作影响范围(只读,不 mutate topology / registry)。
#[tauri::command]
pub async fn dry_run_recovery(
    state: State<'_, AppState>,
    action_id: String,
    target_resource_id: String,
    input_params: Value,
) -> Result<engine_recovery::DryRunResult, String> {
    let topo = state.storage.materialized_topology().await.map_err(|e| e.to_string())?;
    Ok(engine_recovery::dry_run(&action_id, &target_resource_id, &input_params, &topo))
}

/// rule -> action 推荐(对齐 reference `GET /suggestions?rule_id=`)。
#[tauri::command]
pub async fn recovery_suggestions_for_rule(rule_id: String) -> Vec<engine_recovery::ActionSuggestion> {
    engine_recovery::suggest_for_rule(&rule_id).to_vec()
}

// ===== execution 生命周期 =====

/// 执行恢复动作。low 风险同步跑(+ verify,可能 auto-rollback);medium/high -> awaiting_approval。
#[tauri::command]
pub async fn execute_recovery(
    state: State<'_, AppState>,
    action_id: String,
    target_resource_id: String,
    input_params: Value,
    initiated_by: Option<String>,
    request_reason: Option<String>,
    finding_id: Option<String>,
) -> Result<engine_recovery::RecoveryExecution, String> {
    let mut topo = state.storage.materialized_topology().await.map_err(|e| e.to_string())?;
    let exec = {
        let mut reg = state.recovery_executions.lock().await;
        let mut exec = engine_recovery::execute(
            &mut reg,
            &action_id,
            &target_resource_id,
            &input_params,
            &mut topo,
            &initiated_by.unwrap_or_default(),
            &request_reason.unwrap_or_default(),
            &engine_recovery::MockHandlerExecutor,
        )
        .await
        .map_err(|e| e.to_string())?;
        if let Some(fid) = &finding_id {
            if let Some(stored) = reg.get_mut(&exec.execution_id) {
                stored.finding_id = Some(fid.clone());
            }
            exec.finding_id = Some(fid.clone());
        }
        exec
    };
    persist_exec(&state, &exec).await?;
    Ok(exec)
}

/// 列 execution(新到旧,可按 status / action_id / target 过滤)。读内存 registry
/// (与 storage 经 upsert-after-mutation + 启动载入保持一致)。
#[tauri::command]
pub async fn list_recovery_executions(
    state: State<'_, AppState>,
    status: Option<String>,
    action_id: Option<String>,
    target_resource_id: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<engine_recovery::RecoveryExecution>, String> {
    let reg = state.recovery_executions.lock().await;
    let status_filter = status.as_deref().and_then(parse_recovery_status);
    let listed = reg.list_filtered(
        status_filter,
        action_id.as_deref(),
        target_resource_id.as_deref(),
        limit.unwrap_or(100),
    );
    Ok(listed.into_iter().cloned().collect())
}

/// 取单个 execution。
#[tauri::command]
pub async fn get_recovery_execution(
    state: State<'_, AppState>,
    execution_id: String,
) -> Result<engine_recovery::RecoveryExecution, String> {
    let reg = state.recovery_executions.lock().await;
    reg.get(&execution_id)
        .cloned()
        .ok_or_else(|| format!("[404] execution not found: {execution_id}"))
}

/// 确认执行(单机确认门 = approve)。awaiting_approval -> 跑 handler(+ verify)。
#[tauri::command]
pub async fn confirm_recovery_execution(
    state: State<'_, AppState>,
    execution_id: String,
    approval_comment: Option<String>,
) -> Result<engine_recovery::RecoveryExecution, String> {
    let mut topo = state.storage.materialized_topology().await.map_err(|e| e.to_string())?;
    let exec = {
        let mut reg = state.recovery_executions.lock().await;
        engine_recovery::confirm_execution(&mut reg, &execution_id, &mut topo, &approval_comment.unwrap_or_default(), &engine_recovery::MockHandlerExecutor).await
            .map_err(|e| e.to_string())?
    };
    persist_exec(&state, &exec).await?;
    Ok(exec)
}

/// 取消执行(单机确认门 = reject)。awaiting_approval -> rejected。
#[tauri::command]
pub async fn cancel_recovery_execution(
    state: State<'_, AppState>,
    execution_id: String,
) -> Result<engine_recovery::RecoveryExecution, String> {
    let exec = {
        let mut reg = state.recovery_executions.lock().await;
        engine_recovery::cancel_execution(&mut reg, &execution_id).map_err(|e| e.to_string())?
    };
    state.storage.upsert_recovery_execution(&exec).await.map_err(|e| e.to_string())?;
    Ok(exec)
}

/// 回滚一个 succeeded execution(skip re-approval;反向 exec 读 post-action twin 状态反转)。
#[tauri::command]
pub async fn rollback_recovery_execution(
    state: State<'_, AppState>,
    execution_id: String,
    initiated_by: Option<String>,
    reason: Option<String>,
) -> Result<engine_recovery::RecoveryExecution, String> {
    let mut topo = state.storage.materialized_topology().await.map_err(|e| e.to_string())?;
    let (rb, orig) = {
        let mut reg = state.recovery_executions.lock().await;
        let rb = engine_recovery::rollback(
            &mut reg,
            &execution_id,
            &mut topo,
            &initiated_by.unwrap_or_default(),
            &reason.unwrap_or_default(),
            &engine_recovery::MockHandlerExecutor,
        )
        .await
        .map_err(|e| e.to_string())?;
        let orig = reg.get(&execution_id).cloned();
        (rb, orig)
    };
    // rb(反向 execution)+ 原 execution(status -> rolled_back)都持久化
    state.storage.upsert_recovery_execution(&rb).await.map_err(|e| e.to_string())?;
    if let Some(orig) = orig {
        state.storage.upsert_recovery_execution(&orig).await.map_err(|e| e.to_string())?;
    }
    Ok(rb)
}

/// 主动重新验证(不触发 auto-rollback)。仅 succeeded/rolled_back 可 reverify。
#[tauri::command]
pub async fn reverify_recovery_execution(
    state: State<'_, AppState>,
    execution_id: String,
) -> Result<engine_recovery::RecoveryExecution, String> {
    let mut topo = state.storage.materialized_topology().await.map_err(|e| e.to_string())?;
    let exec = {
        let mut reg = state.recovery_executions.lock().await;
        engine_recovery::reverify(&mut reg, &execution_id, &mut topo, &engine_recovery::MockHandlerExecutor).await.map_err(|e| e.to_string())?
    };
    state.storage.upsert_recovery_execution(&exec).await.map_err(|e| e.to_string())?;
    Ok(exec)
}

// ===== chains =====

/// 列全部 chain 模板(前端选择用)。
#[tauri::command]
pub async fn list_chain_templates() -> Vec<ChainTemplateDto> {
    engine_recovery::list_chain_template_ids()
        .into_iter()
        .filter_map(engine_recovery::get_chain_template)
        .map(ChainTemplateDto::from)
        .collect()
}

/// 取单个 chain 模板。
#[tauri::command]
pub async fn get_chain_template(template_id: String) -> Result<ChainTemplateDto, String> {
    engine_recovery::get_chain_template(&template_id)
        .map(ChainTemplateDto::from)
        .ok_or_else(|| format!("[404] unknown chain template: {template_id}"))
}

/// 发起 chain。全 low -> 直接跑;任一步 medium/high -> awaiting_approval。
#[tauri::command]
pub async fn execute_chain(
    state: State<'_, AppState>,
    template_id: String,
    target_resource_id: String,
    initiated_by: Option<String>,
    request_reason: Option<String>,
    on_failure_override: Option<String>,
) -> Result<engine_recovery::RecoveryChain, String> {
    let mut topo = state.storage.materialized_topology().await.map_err(|e| e.to_string())?;
    let on_failure = on_failure_override.as_deref().and_then(parse_on_failure);
    let chain = {
        let mut chain_reg = state.recovery_chains.lock().await;
        let mut exec_reg = state.recovery_executions.lock().await;
        engine_recovery::execute_chain(
            &mut chain_reg,
            &mut exec_reg,
            &mut topo,
            &template_id,
            &target_resource_id,
            &initiated_by.unwrap_or_default(),
            on_failure,
            &request_reason.unwrap_or_default(),
            &engine_recovery::MockHandlerExecutor,
        )
        .await
        .map_err(|e| e.to_string())?
    };
    persist_chain(&state, &chain).await?;
    Ok(chain)
}

/// 链级审批通过(单机确认门)。awaiting_approval -> 跑完整链。
#[tauri::command]
pub async fn confirm_chain(
    state: State<'_, AppState>,
    chain_id: String,
    approval_comment: Option<String>,
) -> Result<engine_recovery::RecoveryChain, String> {
    let mut topo = state.storage.materialized_topology().await.map_err(|e| e.to_string())?;
    let chain = {
        let mut chain_reg = state.recovery_chains.lock().await;
        let mut exec_reg = state.recovery_executions.lock().await;
        engine_recovery::confirm_chain(&mut chain_reg, &mut exec_reg, &mut topo, &chain_id, &approval_comment.unwrap_or_default(), &engine_recovery::MockHandlerExecutor).await
            .map_err(|e| e.to_string())?
    };
    persist_chain(&state, &chain).await?;
    Ok(chain)
}

/// 取消链(单机确认门 = reject)。awaiting_approval -> failed。
#[tauri::command]
pub async fn cancel_chain(state: State<'_, AppState>, chain_id: String) -> Result<engine_recovery::RecoveryChain, String> {
    let chain = {
        let mut chain_reg = state.recovery_chains.lock().await;
        engine_recovery::cancel_chain(&mut chain_reg, &chain_id).map_err(|e| e.to_string())?
    };
    state.storage.upsert_recovery_chain(&chain).await.map_err(|e| e.to_string())?;
    Ok(chain)
}

/// 中止运行中的 chain(标 aborted,不做反向 rollback)。
#[tauri::command]
pub async fn abort_chain(
    state: State<'_, AppState>,
    chain_id: String,
    reason: Option<String>,
) -> Result<engine_recovery::RecoveryChain, String> {
    let chain = {
        let mut chain_reg = state.recovery_chains.lock().await;
        engine_recovery::abort_chain(&mut chain_reg, &chain_id, &reason.unwrap_or_default()).map_err(|e| e.to_string())?
    };
    state.storage.upsert_recovery_chain(&chain).await.map_err(|e| e.to_string())?;
    Ok(chain)
}

/// 列 chain(新到旧,可按 status 过滤)。
#[tauri::command]
pub async fn list_recovery_chains(
    state: State<'_, AppState>,
    status: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<engine_recovery::RecoveryChain>, String> {
    let chain_reg = state.recovery_chains.lock().await;
    let status_filter = status.as_deref().and_then(parse_chain_status);
    let limit = limit.unwrap_or(100);
    let mut chains: Vec<engine_recovery::RecoveryChain> = chain_reg
        .list()
        .into_iter()
        .filter(|c| status_filter.is_none_or(|s| c.status == s))
        .cloned()
        .collect();
    chains.sort_by(|a, b| b.initiated_at.cmp(&a.initiated_at));
    chains.truncate(limit);
    Ok(chains)
}

/// 取单个 chain。
#[tauri::command]
pub async fn get_recovery_chain(
    state: State<'_, AppState>,
    chain_id: String,
) -> Result<engine_recovery::RecoveryChain, String> {
    let chain_reg = state.recovery_chains.lock().await;
    chain_reg
        .get(&chain_id)
        .cloned()
        .ok_or_else(|| format!("[404] chain not found: {chain_id}"))
}
