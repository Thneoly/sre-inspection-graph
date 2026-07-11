//! change_events commands - 把 `engine_changes`(PRD-002)暴露给前端(Phase 3.6)。
//!
//! 命令面镜像 reference `app/routers/change_event.py`。`record_change_event` =
//! `POST /change-events`;`correlated_changes` = `GET /correlated`;`frequent_changes`
//! = `GET /frequent`;`change_event_impact` = `GET /{id}/impact`;
//! `change_event_recovery_suggestion` = `GET /{id}/recovery-suggestion`(桥 PRD-001);
//! `change_event_alerts` = `GET /{id}/alerts`(桥 alert 关联)。
//!
//! ## 持久化
//!
//! `record_change_event` 调 `engine_changes::record_change`(mutate registry)+ upsert
//! 回 `change_events` 表。其余查询读内存 registry(经 upsert-after-mutation + 启动
//! 载入与 storage 一致)。需要 `&Topology` 的命令(correlated/impact/suggestion/
//! alerts)从 storage 读 materialized topology。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;

use crate::AppState;

/// `record_change_event` 入参 DTO(`ChangeRequest` 未 Deserialize;字段 serde default)。
#[derive(Debug, Clone, Deserialize)]
pub struct RecordChangeRequest {
    pub change_type: String,
    pub target_resource_id: String,
    #[serde(default)]
    pub changed_by: String,
    #[serde(default = "default_source")]
    pub source: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_diff_summary")]
    pub diff_summary: Value,
    #[serde(default)]
    pub related_commit: String,
    #[serde(default)]
    pub related_pr: String,
    #[serde(default)]
    pub changed_at: Option<String>,
    #[serde(default)]
    pub commit_sha: String,
    #[serde(default)]
    pub pipeline_url: String,
    #[serde(default)]
    pub git_repo: String,
    #[serde(default)]
    pub cluster_id: String,
    #[serde(default)]
    pub yaml_diff: String,
}

fn default_source() -> String {
    engine_changes::Source::default_name().to_string()
}

fn default_diff_summary() -> Value {
    serde_json::json!({})
}

impl From<RecordChangeRequest> for engine_changes::ChangeRequest {
    fn from(r: RecordChangeRequest) -> Self {
        engine_changes::ChangeRequest {
            change_type: r.change_type,
            target_resource_id: r.target_resource_id,
            changed_by: r.changed_by,
            source: r.source,
            description: r.description,
            diff_summary: r.diff_summary,
            related_commit: r.related_commit,
            related_pr: r.related_pr,
            changed_at: r.changed_at,
            commit_sha: r.commit_sha,
            pipeline_url: r.pipeline_url,
            git_repo: r.git_repo,
            cluster_id: r.cluster_id,
            yaml_diff: r.yaml_diff,
        }
    }
}

/// `change_event_impact` 返回(blast-radius + severity)。
#[derive(Debug, Clone, Serialize)]
pub struct ChangeEventImpactResponse {
    pub change_event_id: String,
    pub target_resource_id: String,
    pub target_resource_type: String,
    pub affected: Vec<String>,
    pub affected_count: usize,
    pub severity_estimate: engine_changes::Severity,
}

/// `frequent_changes` 返回(对齐 reference `{frequent, window_seconds, threshold}`)。
#[derive(Debug, Clone, Serialize)]
pub struct FrequentChangesResponse {
    pub frequent: Vec<engine_changes::FrequentTarget>,
    pub window_seconds: i64,
    pub threshold: usize,
}

/// 记录一个变更事件。mutate registry + upsert 回 storage。返回序列化事件(含 propagated_count)。
#[tauri::command]
pub async fn record_change_event(
    state: State<'_, AppState>,
    req: RecordChangeRequest,
) -> Result<Value, String> {
    let req: engine_changes::ChangeRequest = req.into();
    let topo = state.storage.materialized_topology().await.map_err(|e| e.to_string())?;
    let event = {
        let mut reg = state.change_events.lock().map_err(|e| e.to_string())?;
        engine_changes::record_change(&mut reg, &topo, &req).map_err(|e| e.to_string())?
    };
    state.storage.upsert_change_event(&event).await.map_err(|e| e.to_string())?;
    Ok(engine_changes::serialize(&event))
}

/// 列变更事件(可按 type/target/source/since/until 过滤,changed_at 倒序)。
#[tauri::command]
pub fn list_change_events(
    state: State<'_, AppState>,
    change_type: Option<String>,
    target_resource_id: Option<String>,
    source: Option<String>,
    since: Option<String>,
    until: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<Value>, String> {
    let reg = state.change_events.lock().map_err(|e| e.to_string())?;
    let filter = engine_changes::ChangeFilter {
        change_type: change_type.as_deref().and_then(engine_changes::ChangeType::from_name),
        target_resource_id,
        source: source.as_deref().and_then(engine_changes::Source::from_name),
        since,
        until,
    };
    let mut events: Vec<Value> = reg.list(&filter).into_iter().map(engine_changes::serialize).collect();
    events.sort_by(|a, b| {
        let ta = a.get("changed_at").and_then(Value::as_str).unwrap_or("");
        let tb = b.get("changed_at").and_then(Value::as_str).unwrap_or("");
        tb.cmp(ta)
    });
    if let Some(l) = limit {
        events.truncate(l);
    }
    Ok(events)
}

/// 取单个变更事件(序列化,含 propagated_count)。
#[tauri::command]
pub fn get_change_event(state: State<'_, AppState>, change_event_id: String) -> Result<Value, String> {
    let reg = state.change_events.lock().map_err(|e| e.to_string())?;
    let event = reg
        .get(&change_event_id)
        .ok_or_else(|| format!("[404] change_event not found: {change_event_id}"))?;
    Ok(engine_changes::serialize(event))
}

/// 查询 target 在时间窗内的相关变更(direct + propagated)。
#[tauri::command]
pub async fn correlated_changes(
    state: State<'_, AppState>,
    target_resource_id: String,
    window: Option<i64>,
    since: Option<String>,
    until: Option<String>,
    include_propagated: Option<bool>,
) -> Result<engine_changes::CorrelatedResult, String> {
    let topo = state.storage.materialized_topology().await.map_err(|e| e.to_string())?;
    let result = {
        let reg = state.change_events.lock().map_err(|e| e.to_string())?;
        engine_changes::correlated_changes(
            &reg,
            &topo,
            &target_resource_id,
            window.unwrap_or(engine_changes::DEFAULT_CORRELATED_WINDOW_SECONDS),
            since.as_deref(),
            until.as_deref(),
            include_propagated.unwrap_or(true),
        )
    };
    Ok(result)
}

/// 过频变更检测(按 target 分桶,count > threshold)。
#[tauri::command]
pub fn frequent_changes(
    state: State<'_, AppState>,
    window: Option<i64>,
    threshold: Option<usize>,
) -> Result<FrequentChangesResponse, String> {
    let win = window.unwrap_or(engine_changes::DEFAULT_WINDOW_SECONDS);
    let thr = threshold.unwrap_or(engine_changes::DEFAULT_THRESHOLD);
    let reg = state.change_events.lock().map_err(|e| e.to_string())?;
    let frequent = engine_changes::detect_frequent_changes(&reg, win, thr);
    Ok(FrequentChangesResponse {
        frequent,
        window_seconds: win,
        threshold: thr,
    })
}

/// 变更影响范围(blast-radius + severity)。用当前 materialized topology 重算 `derive_propagation`。
#[tauri::command]
pub async fn change_event_impact(
    state: State<'_, AppState>,
    change_event_id: String,
) -> Result<ChangeEventImpactResponse, String> {
    let topo = state.storage.materialized_topology().await.map_err(|e| e.to_string())?;
    let event = {
        let reg = state.change_events.lock().map_err(|e| e.to_string())?;
        reg.get(&change_event_id)
            .cloned()
            .ok_or_else(|| format!("[404] change_event not found: {change_event_id}"))?
    };
    let affected = engine_changes::derive_propagation(
        &event.target_resource_id,
        &topo,
        engine_changes::DEFAULT_PROPAGATION_DEPTH,
        None,
    );
    let affected_count = affected.len();
    Ok(ChangeEventImpactResponse {
        change_event_id: event.change_event_id,
        target_resource_id: event.target_resource_id,
        target_resource_type: event.target_resource_type,
        affected,
        affected_count,
        severity_estimate: event.severity_estimate,
    })
}

/// 给定变更事件,返回 PRD-001 恢复动作推荐(桥 `suggest_for_change`)。
#[tauri::command]
pub async fn change_event_recovery_suggestion(
    state: State<'_, AppState>,
    change_event_id: String,
) -> Result<engine_changes::RecoverySuggestionResult, String> {
    let topo = state.storage.materialized_topology().await.map_err(|e| e.to_string())?;
    let result = {
        let reg = state.change_events.lock().map_err(|e| e.to_string())?;
        engine_changes::get_recovery_suggestion(&reg, &topo, &change_event_id)
            .map_err(|e| e.to_string())?
    };
    Ok(result)
}

/// 给定变更事件,返回窗口内关联的 alerts(`correlate_alerts`)。
///
/// 同时锁 change_events + alerts(顺序:change_events 先,alerts 后,与
/// `correlate_changes_for_alert` 一致,避免死锁)。
#[tauri::command]
pub async fn change_event_alerts(
    state: State<'_, AppState>,
    change_event_id: String,
    window: Option<i64>,
) -> Result<engine_changes::CorrelateAlertsResult, String> {
    let win = window.unwrap_or(engine_changes::DEFAULT_ALERT_WINDOW_SECONDS);
    let result = {
        let reg = state.change_events.lock().map_err(|e| e.to_string())?;
        let alerts = state.alerts.lock().map_err(|e| e.to_string())?;
        engine_changes::correlate_alerts(&reg, &alerts, &change_event_id, win).map_err(|e| e.to_string())?
    };
    Ok(result)
}
