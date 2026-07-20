//! 订阅执行器(PRD-003 Phase 4.3,对齐 reference `_run_subscription_safely`)。
//!
//! `run_subscription`:从订阅构造 `ReportTask` -> `generate_report` -> `email_sender.send`。
//! 调度循环 + `trigger_now` 命令共用此入口。`last_*` 回写由调用方持久化(返 `RunResult`
//! 供其更新订阅 + 存 ReportStore)。I/O 仅经 `EmailSender` trait(可注入 InMemory 测试)。

#![allow(missing_docs)]

use engine_changes::ChangeRegistry;
use engine_identity::Topology;
use engine_recovery::ExecutionRegistry;

use crate::email::{EmailError, EmailSender};
use crate::generator::{generate_report, ReportError};
use crate::models::{ReportStatus, ReportTask};
use crate::subscription::ReportSubscription;
use crate::ReportTemplate;

/// 订阅执行错误。
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// 报告生成失败。
    #[error("generate failed: {0}")]
    Generate(String),
    /// 邮件发送失败。
    #[error("email failed: {0}")]
    Email(String),
}

impl From<ReportError> for RunError {
    fn from(e: ReportError) -> Self {
        RunError::Generate(e.to_string())
    }
}

impl From<EmailError> for RunError {
    fn from(e: EmailError) -> Self {
        RunError::Email(e.to_string())
    }
}

/// 订阅执行结果(含 completed ReportTask 供调用方存 ReportStore)。
#[derive(Debug, Clone)]
pub struct RunResult {
    pub report_id: String,
    pub markdown: String,
    /// 已完成的 ReportTask(status=Completed,markdown 已填)。
    pub task: ReportTask,
}

/// 构造邮件 subject(对齐 reference:`[SRE 巡检报告] {template} - {scope_label}`)。
fn build_subject(sub: &ReportSubscription) -> String {
    let scope_label = sub
        .scope
        .application_id
        .clone()
        .or_else(|| sub.scope.cluster_id.clone())
        .or_else(|| sub.scope.change_event_id.clone())
        .or_else(|| sub.scope.fault_id.clone())
        .unwrap_or_else(|| "总览".to_string());
    let tpl = match sub.template_id {
        ReportTemplate::ApplicationHealth => "application_health",
        ReportTemplate::ClusterOverview => "cluster_overview",
        ReportTemplate::IncidentReport => "incident_report",
    };
    format!("[SRE 巡检报告] {tpl} - {scope_label}")
}

/// 立即执行一次订阅(trigger_now / 调度触发共用)。
///
/// `now` 由调用方传(ISO8601),避免引擎依赖时钟。
pub async fn run_subscription(
    sub: &ReportSubscription,
    topology: &Topology,
    changes: &ChangeRegistry,
    executions: &ExecutionRegistry,
    email_sender: &dyn EmailSender,
    now: &str,
) -> Result<RunResult, RunError> {
    let report_id = format!("rpt-{}", uuid::Uuid::new_v4().simple());
    let task = ReportTask {
        report_id: report_id.clone(),
        template_id: sub.template_id,
        scope: sub.scope.clone(),
        modules: sub.modules.clone(),
        format: "markdown".to_string(),
        status: ReportStatus::Generating,
        progress: 0,
        current_step: "采集 + 渲染".to_string(),
        error_message: None,
        markdown: None,
        created_at: now.to_string(),
        completed_at: None,
    };
    let markdown = generate_report(&task, topology, changes, executions, now)?;

    let subject = build_subject(sub);
    let attachment_filename = format!("{report_id}.md");
    // body 和附件同一份 markdown(对齐 reference:body=plain markdown,attachment=.md)
    email_sender
        .send(
            sub.recipients.clone(),
            &subject,
            &markdown,
            &attachment_filename,
            &markdown,
        )
        .await?;

    let mut completed = task;
    completed.status = ReportStatus::Completed;
    completed.progress = 100;
    completed.markdown = Some(markdown.clone());
    completed.completed_at = Some(now.to_string());

    Ok(RunResult {
        report_id,
        markdown,
        task: completed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_changes::{ChangeRegistry};
    use engine_identity::{ResolvedNode, Topology};
    use engine_recovery::ExecutionRegistry;

    use crate::email::InMemoryEmailSender;
    use crate::models::ReportScope;
    use crate::subscription::SubscriptionStatus;

    fn topo() -> Topology {
        Topology {
            nodes: vec![ResolvedNode {
                resource_id: "app:order".into(),
                resource_type: "Application".into(),
                label: "order".into(),
                attributes_json: r#"{"health_status":"normal"}"#.into(),
            }],
            edges: vec![],
        }
    }

    fn sub_app_health() -> ReportSubscription {
        ReportSubscription {
            subscription_id: "sub-test".into(),
            template_id: ReportTemplate::ApplicationHealth,
            scope: ReportScope {
                application_id: Some("app:order".into()),
                ..Default::default()
            },
            modules: vec![],
            cron: "0 9 * * 1".into(),
            recipients: vec!["ops@example.com".into(), "sre@example.com".into()],
            enabled: true,
            created_at: "2026-07-20T00:00:00Z".into(),
            last_run_at: String::new(),
            last_status: SubscriptionStatus::Never,
            last_error: String::new(),
            last_report_id: String::new(),
        }
    }

    // 移植 reference test_reports_sprint2_sub.py::test_trigger_now_generates_and_sends
    #[tokio::test]
    async fn run_subscription_generates_and_sends() {
        let sub = sub_app_health();
        let t = topo();
        let cr = ChangeRegistry::new();
        let er = ExecutionRegistry::new();
        let sender = InMemoryEmailSender::new();
        let now = "2026-07-20T09:00:30Z";

        let result = run_subscription(&sub, &t, &cr, &er, &sender, now).await.unwrap();

        // 报告生成
        assert!(result.markdown.contains("# 应用健康报告"));
        assert!(result.report_id.starts_with("rpt-"));
        // completed task
        assert_eq!(result.task.status, ReportStatus::Completed);
        assert_eq!(result.task.markdown.as_deref(), Some(result.markdown.as_str()));
        assert_eq!(result.task.completed_at.as_deref(), Some(now));
        // 邮件发送
        let sent = sender.list_sent().await;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].recipients, vec!["ops@example.com".to_string(), "sre@example.com".to_string()]);
        assert!(sent[0].subject.contains("application_health"));
        assert!(sent[0].subject.contains("app:order"));
        assert_eq!(sent[0].body, result.markdown); // body = markdown
        assert_eq!(sent[0].attachment_filename, format!("{}.md", result.report_id));
        assert_eq!(sent[0].attachment_content, result.markdown); // 附件同一份 markdown
    }

    #[tokio::test]
    async fn run_subscription_incident_anchor_not_found_errors() {
        // incident 订阅但 change_event_id 不存在 -> generate_report 返 AnchorNotFound -> RunError::Generate
        let sub = ReportSubscription {
            template_id: ReportTemplate::IncidentReport,
            scope: ReportScope {
                change_event_id: Some("ce-nonexistent".into()),
                ..Default::default()
            },
            ..sub_app_health()
        };
        let t = topo();
        let cr = ChangeRegistry::new();
        let er = ExecutionRegistry::new();
        let sender = InMemoryEmailSender::new();
        let err = run_subscription(&sub, &t, &cr, &er, &sender, "2026-07-20T09:00:30Z").await.unwrap_err();
        assert!(matches!(err, RunError::Generate(_)));
        // 生成失败不应发邮件
        assert!(sender.list_sent().await.is_empty());
    }
}
