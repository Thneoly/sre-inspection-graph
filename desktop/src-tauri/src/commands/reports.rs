//! PRD-003 报告命令(Phase 4.1)。生成 application_health Markdown 报告。

use tauri::State;

use engine_reports::{ReportScope, ReportStatus, ReportTask, ReportTemplate};

use crate::AppState;

pub(crate) fn parse_template(s: &str) -> Result<ReportTemplate, String> {
    match s {
        "application_health" => Ok(ReportTemplate::ApplicationHealth),
        "cluster_overview" => Ok(ReportTemplate::ClusterOverview),
        "incident_report" => Ok(ReportTemplate::IncidentReport),
        _ => Err(format!("unknown template_id: {s}")),
    }
}

/// 生成报告(采集 + Tera 渲染)。当前只 application_health 实现;cluster_overview/incident_report 留 Phase 4.2。
#[tauri::command]
pub async fn generate_report_cmd(
    state: State<'_, AppState>,
    template_id: String,
    application_id: Option<String>,
    cluster_id: Option<String>,
    change_event_id: Option<String>,
    fault_id: Option<String>,
    modules: Option<Vec<String>>,
) -> Result<ReportTask, String> {
    let template = parse_template(&template_id)?;
    let scope = ReportScope {
        application_id,
        cluster_id,
        change_event_id,
        fault_id,
        ..Default::default()
    };
    let report_id = format!("rpt-{}", uuid::Uuid::new_v4().simple());
    let now = chrono::Utc::now().to_rfc3339();
    let mut task = ReportTask {
        report_id,
        template_id: template,
        scope,
        modules: modules.unwrap_or_default(),
        format: "markdown".into(),
        status: ReportStatus::Generating,
        progress: 0,
        current_step: "采集 + 渲染".into(),
        error_message: None,
        markdown: None,
        created_at: now.clone(),
        completed_at: None,
    };

    // 采集:materialized topology + change_events + recovery_executions(锁内同步调用,不跨 await)
    let topo = state.storage.materialized_topology().await.map_err(|e| e.to_string())?;
    let changes = state.change_events.lock().await;
    let execs = state.recovery_executions.lock().await;
    let md = engine_reports::generate_report(&task, &topo, &changes, &execs, &now)
        .map_err(|e| e.to_string())?;
    drop(changes);
    drop(execs);

    task.markdown = Some(md);
    task.status = ReportStatus::Completed;
    task.progress = 100;
    task.current_step.clear();
    task.completed_at = Some(chrono::Utc::now().to_rfc3339());
    state.reports.lock().await.add(task.clone());
    Ok(task)
}

/// 列报告(新到旧,可按 template_id / application_id 过滤)。
#[tauri::command]
pub async fn list_reports(
    state: State<'_, AppState>,
    template_id: Option<String>,
    application_id: Option<String>,
) -> Result<Vec<ReportTask>, String> {
    let tid = match template_id.as_deref() {
        Some(t) => Some(parse_template(t)?),
        None => None,
    };
    let reg = state.reports.lock().await;
    Ok(reg.list(tid, application_id.as_deref()).into_iter().cloned().collect())
}

/// 取报告详情(含 markdown)。
#[tauri::command]
pub async fn get_report(state: State<'_, AppState>, report_id: String) -> Result<ReportTask, String> {
    state
        .reports
        .lock().await
        .get(&report_id)
        .cloned()
        .ok_or_else(|| format!("report not found: {report_id}"))
}
