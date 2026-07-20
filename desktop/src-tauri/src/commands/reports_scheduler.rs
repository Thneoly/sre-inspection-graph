//! PRD-003 报告订阅命令(Phase 4.3)。订阅 CRUD + 立即触发 + 已发邮件调试。
//!
//! 对齐 reference `/api/v1/reports/subscriptions/*` 端点。订阅持久化到 SQLite
//! `report_subscriptions` 表(orchestration 层 upsert);`trigger_subscription_now` 复用
//! `engine_reports::run_subscription`(与调度循环同一入口)。DTO 直接返 engine Serialize
//! 类型(对齐 Phase 3.6 偏差)。

use tauri::{AppHandle, Manager, State};

use engine_reports::{
    parse_cron, run_subscription, ReportScope, ReportSubscription, ReportTask, SubscriptionStatus,
    validate_subscription,
};

use crate::commands::reports::parse_template;
use crate::AppState;

/// 构造 ReportScope(透传 4 个锚点字段 + time_range)。
fn build_scope(
    application_id: Option<String>,
    cluster_id: Option<String>,
    change_event_id: Option<String>,
    fault_id: Option<String>,
) -> ReportScope {
    ReportScope {
        application_id,
        cluster_id,
        change_event_id,
        fault_id,
        ..Default::default()
    }
}

/// 创建订阅(校验 cron + recipients -> 入 registry + upsert storage)。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn create_subscription(
    state: State<'_, AppState>,
    template_id: String,
    application_id: Option<String>,
    cluster_id: Option<String>,
    change_event_id: Option<String>,
    fault_id: Option<String>,
    modules: Option<Vec<String>>,
    cron: String,
    recipients: Vec<String>,
    enabled: Option<bool>,
) -> Result<ReportSubscription, String> {
    let template = parse_template(&template_id)?;
    validate_subscription(template, &cron, &recipients)?;
    let _ = parse_cron(&cron)?; // 再解析一次拿 Schedule(校验已含,此处冗余但确保)
    let now = chrono::Utc::now().to_rfc3339();
    let sub = ReportSubscription {
        subscription_id: ReportSubscription::new_id(),
        template_id: template,
        scope: build_scope(application_id, cluster_id, change_event_id, fault_id),
        modules: modules.unwrap_or_default(),
        cron,
        recipients,
        enabled: enabled.unwrap_or(true),
        created_at: now,
        last_run_at: String::new(),
        last_status: SubscriptionStatus::Never,
        last_error: String::new(),
        last_report_id: String::new(),
    };
    state
        .storage
        .upsert_subscription(&sub)
        .await
        .map_err(|e| e.to_string())?;
    state.subscriptions.lock().await.add(sub.clone());
    Ok(sub)
}

/// 列订阅(新到旧,可按 template_id 过滤)。
#[tauri::command]
pub async fn list_subscriptions(
    state: State<'_, AppState>,
    template_id: Option<String>,
) -> Result<Vec<ReportSubscription>, String> {
    let tid = match template_id.as_deref() {
        Some(t) => Some(parse_template(t)?),
        None => None,
    };
    let reg = state.subscriptions.lock().await;
    Ok(reg.list(tid).into_iter().cloned().collect())
}

/// 取订阅详情。
#[tauri::command]
pub async fn get_subscription(
    state: State<'_, AppState>,
    subscription_id: String,
) -> Result<ReportSubscription, String> {
    state
        .subscriptions
        .lock()
        .await
        .get(&subscription_id)
        .cloned()
        .ok_or_else(|| format!("subscription not found: {subscription_id}"))
}

/// 更新订阅(cron / recipients / enabled / modules;None 字段不动)。改后 upsert。
#[tauri::command]
pub async fn update_subscription(
    state: State<'_, AppState>,
    subscription_id: String,
    cron: Option<String>,
    recipients: Option<Vec<String>>,
    enabled: Option<bool>,
    modules: Option<Vec<String>>,
) -> Result<ReportSubscription, String> {
    let snap = {
        let mut reg = state.subscriptions.lock().await;
        let s = reg
            .get_mut(&subscription_id)
            .ok_or_else(|| format!("subscription not found: {subscription_id}"))?;
        if let Some(c) = cron {
            validate_subscription(s.template_id, &c, &s.recipients)?;
            s.cron = c;
        }
        if let Some(r) = recipients {
            validate_subscription(s.template_id, &s.cron, &r)?;
            s.recipients = r;
        }
        if let Some(e) = enabled {
            s.enabled = e;
        }
        if let Some(m) = modules {
            s.modules = m;
        }
        s.clone()
    };
    state
        .storage
        .upsert_subscription(&snap)
        .await
        .map_err(|e| e.to_string())?;
    Ok(snap)
}

/// 删除订阅(registry + storage)。
#[tauri::command]
pub async fn delete_subscription(
    state: State<'_, AppState>,
    subscription_id: String,
) -> Result<bool, String> {
    let removed = state.subscriptions.lock().await.delete(&subscription_id);
    if removed {
        state
            .storage
            .delete_subscription(&subscription_id)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(removed)
}

/// 立即触发订阅(对齐 reference `trigger_now`;复用 run_subscription,与调度循环同入口)。
/// 生成报告 + 发邮件 + 回写 last_*,返回生成的 ReportTask。
#[tauri::command]
pub async fn trigger_subscription_now(
    app: AppHandle,
    subscription_id: String,
) -> Result<ReportTask, String> {
    let state = app.state::<AppState>();
    let sub = state
        .subscriptions
        .lock()
        .await
        .get(&subscription_id)
        .cloned()
        .ok_or_else(|| format!("subscription not found: {subscription_id}"))?;
    let now = chrono::Utc::now().to_rfc3339();

    let topo = state
        .storage
        .materialized_topology()
        .await
        .map_err(|e| e.to_string())?;
    let result = {
        let changes = state.change_events.lock().await;
        let execs = state.recovery_executions.lock().await;
        run_subscription(&sub, &topo, &changes, &execs, &*state.email_sender, &now)
            .await
            .map_err(|e| e.to_string())
    };
    let r = result?;

    // 存报告 + 回写 last_*
    state.reports.lock().await.add(r.task.clone());
    state
        .storage
        .upsert_report(&r.task)
        .await
        .map_err(|e| e.to_string())?;
    let snap = {
        let mut reg = state.subscriptions.lock().await;
        if let Some(s) = reg.get_mut(&subscription_id) {
            s.last_run_at = now.clone();
            s.last_status = SubscriptionStatus::Ok;
            s.last_error.clear();
            s.last_report_id = r.report_id.clone();
            Some(s.clone())
        } else {
            None
        }
    };
    if let Some(snap) = snap {
        state
            .storage
            .upsert_subscription(&snap)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(r.task)
}

/// 已发送邮件列表(仅 InMemory 模式返捕获;Smtp 模式返空)。
#[tauri::command]
pub async fn list_sent_emails(app: AppHandle) -> Result<Vec<engine_reports::SentEmail>, String> {
    let state = app.state::<AppState>();
    Ok(state.email_sender.list_sent().await)
}
