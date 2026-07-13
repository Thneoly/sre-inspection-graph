//! alerts commands - 把 `engine_changes::alerts` + alert↔change 关联暴露给前端(Phase 3.6)。
//!
//! 命令面镜像 reference `app/routers/alert.py`。`record_alert` = `POST /alerts`;
//! `list_alerts` = `GET /alerts`;`resolve_alert` = `POST /{id}/resolve`;
//! `correlate_changes_for_alert` = 反向关联(给定 alert 找窗口内 change events)。
//!
//! **无 live 源**:k8s-watch / webhook 延后(Phase 3 延后);alerts 仅经
//! `record_alert` 手动录入。仍持久化到 `alert_events` 表(重启恢复)。

use serde::Deserialize;
use tauri::State;

use crate::AppState;

/// `record_alert` 入参 DTO(`AlertEvent` 字段子集;id/fired_at 可选,缺省自动生成)。
#[derive(Debug, Clone, Deserialize)]
pub struct RecordAlertRequest {
    pub alert_name: String,
    #[serde(default)]
    pub alert_event_id: Option<String>,
    pub resource_ref: String,
    #[serde(default = "default_severity")]
    pub severity: String,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default)]
    pub fired_at: Option<String>,
    #[serde(default)]
    pub rule_id: String,
    #[serde(default)]
    pub metric_name: String,
    #[serde(default)]
    pub metric_value: f64,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub cluster_id: String,
}

fn default_severity() -> String {
    "critical".into()
}

fn default_status() -> String {
    "firing".into()
}

fn parse_severity(s: &str) -> Option<engine_changes::AlertSeverity> {
    serde_json::from_str(&format!("\"{s}\"")).ok()
}

fn parse_status(s: &str) -> Option<engine_changes::AlertStatus> {
    serde_json::from_str(&format!("\"{s}\"")).ok()
}

/// 录入一条 alert。mutate registry + upsert 回 storage。
#[tauri::command]
pub async fn record_alert(state: State<'_, AppState>, req: RecordAlertRequest) -> Result<engine_changes::AlertEvent, String> {
    let id = req
        .alert_event_id
        .clone()
        .unwrap_or_else(|| format!("alert-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]));
    let mut alert = engine_changes::AlertEvent::new(id, req.alert_name);
    alert.resource_ref = req.resource_ref;
    alert.severity = parse_severity(&req.severity).unwrap_or(engine_changes::AlertSeverity::Critical);
    alert.status = parse_status(&req.status).unwrap_or(engine_changes::AlertStatus::Firing);
    alert.fired_at = req.fired_at.unwrap_or_else(engine_changes::iso::now_iso);
    alert.rule_id = req.rule_id;
    alert.metric_name = req.metric_name;
    alert.metric_value = req.metric_value;
    alert.summary = req.summary;
    alert.description = req.description;
    alert.cluster_id = req.cluster_id;

    {
        let mut reg = state.alerts.lock().await;
        reg.add(alert.clone());
    }
    state.storage.upsert_alert_event(&alert).await.map_err(|e| e.to_string())?;
    Ok(alert)
}

/// 列 alerts(可按 resource_ref / severity / status / since / until 过滤,fired_at 倒序)。
#[tauri::command]
pub async fn list_alerts(
    state: State<'_, AppState>,
    resource_ref: Option<String>,
    severity: Option<String>,
    status: Option<String>,
    since: Option<String>,
    until: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<engine_changes::AlertEvent>, String> {
    let reg = state.alerts.lock().await;
    let sev = severity.as_deref().and_then(parse_severity);
    let st = status.as_deref().and_then(parse_status);
    let mut alerts: Vec<engine_changes::AlertEvent> = reg
        .list(since.as_deref(), until.as_deref())
        .into_iter()
        .filter(|&a| resource_ref.as_deref().is_none_or(|r| a.resource_ref == r))
        .filter(|&a| sev.is_none_or(|s| a.severity == s))
        .filter(|&a| st.is_none_or(|s| a.status == s))
        .cloned()
        .collect();
    alerts.sort_by(|a, b| b.fired_at.cmp(&a.fired_at));
    if let Some(l) = limit {
        alerts.truncate(l);
    }
    Ok(alerts)
}

/// 取单个 alert。
#[tauri::command]
pub async fn get_alert(state: State<'_, AppState>, alert_event_id: String) -> Result<engine_changes::AlertEvent, String> {
    let reg = state.alerts.lock().await;
    reg.get(&alert_event_id)
        .cloned()
        .ok_or_else(|| format!("[404] alert not found: {alert_event_id}"))
}

/// 标记 alert 已恢复(status=resolved,resolved_at=now)。
#[tauri::command]
pub async fn resolve_alert(state: State<'_, AppState>, alert_event_id: String) -> Result<engine_changes::AlertEvent, String> {
    let alert = {
        let mut reg = state.alerts.lock().await;
        let a = reg
            .get_mut(&alert_event_id)
            .ok_or_else(|| format!("[404] alert not found: {alert_event_id}"))?;
        a.status = engine_changes::AlertStatus::Resolved;
        a.resolved_at = engine_changes::iso::now_iso();
        a.clone()
    };
    state.storage.upsert_alert_event(&alert).await.map_err(|e| e.to_string())?;
    Ok(alert)
}

/// 反向关联:给定 alert,找窗口内对其 resource 关联的 change events。
///
/// 同时锁 change_events + alerts(顺序:change_events 先,alerts 后,与
/// `change_event_alerts` 一致,避免死锁)。
#[tauri::command]
pub async fn correlate_changes_for_alert(
    state: State<'_, AppState>,
    alert_event_id: String,
    window: Option<i64>,
    resource_ref: Option<String>,
) -> Result<engine_changes::CorrelateChangesForAlertResult, String> {
    let win = window.unwrap_or(engine_changes::DEFAULT_CHANGE_WINDOW_SECONDS);
    let result = {
        let reg = state.change_events.lock().await;
        let alerts = state.alerts.lock().await;
        engine_changes::correlate_changes_for_alert(
            &reg,
            &alerts,
            &alert_event_id,
            win,
            resource_ref.as_deref().unwrap_or(""),
        )
    };
    Ok(result)
}
