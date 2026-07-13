//! 报告生成器(PRD-003,对齐 reference `generator.py`)。
//!
//! 按 `task.modules` 顺序采集 + Tera 渲染 -> Markdown。Tera 替 Jinja2(语法兼容 ~90%)。

#![allow(missing_docs)]

use std::collections::HashMap;

use engine_changes::ChangeRegistry;
use engine_identity::Topology;
use engine_recovery::ExecutionRegistry;
use tera::{Context, Tera};
use thiserror::Error;

use crate::gatherers;
use crate::models::{ReportTask};
use crate::ReportTemplate;

const APP_HEALTH_TEMPLATE: &str = include_str!("templates/application_health.md");

#[derive(Debug, Error)]
pub enum ReportError {
    #[error("unsupported template: {0:?}")]
    UnsupportedTemplate(ReportTemplate),
    #[error("tera setup failed: {0}")]
    Setup(String),
    #[error("tera render failed: {0}")]
    Render(String),
}

/// 生成报告 Markdown(按 modules 采集 + Tera 渲染)。
///
/// `generated_at` 由调用方传(ISO8601),避免引擎依赖时钟。
pub fn generate_report(
    task: &ReportTask,
    topology: &Topology,
    changes: &ChangeRegistry,
    executions: &ExecutionRegistry,
    generated_at: &str,
) -> Result<String, ReportError> {
    match task.template_id {
        ReportTemplate::ApplicationHealth => {
            generate_app_health(task, topology, changes, executions, generated_at)
        }
        _ => Err(ReportError::UnsupportedTemplate(task.template_id)),
    }
}

fn generate_app_health(
    task: &ReportTask,
    topology: &Topology,
    changes: &ChangeRegistry,
    executions: &ExecutionRegistry,
    generated_at: &str,
) -> Result<String, ReportError> {
    let app_id = task.scope.application_id.as_deref().unwrap_or("");

    // modules: task.modules 指定启用的;空 = 全模块(对齐 reference 默认)
    let all_modules = ReportTask::valid_modules(ReportTemplate::ApplicationHealth);
    let enabled: std::collections::HashSet<&str> = if task.modules.is_empty() {
        all_modules.iter().copied().collect()
    } else {
        task.modules.iter().map(|s| s.as_str()).collect()
    };
    let modules: HashMap<String, bool> = all_modules
        .iter()
        .map(|m| (m.to_string(), enabled.contains(m)))
        .collect();

    let mut ctx = Context::new();
    ctx.insert("scope", &task.scope);
    ctx.insert("generated_at", generated_at);
    ctx.insert("report_id", &task.report_id);
    ctx.insert("modules", &modules);

    if modules.get("health_score").copied().unwrap_or(false) {
        let hs = gatherers::gather_health_score(app_id, topology);
        ctx.insert("health_score", &hs);
    }
    if modules.get("seven_views").copied().unwrap_or(false) {
        let sv = gatherers::gather_seven_views(app_id, topology, changes, executions);
        ctx.insert("seven_views", &sv);
    }
    if modules.get("risk_list").copied().unwrap_or(false) {
        let rl = gatherers::gather_risk_list(app_id, topology, changes);
        ctx.insert("risk_list", &rl);
    }
    if modules.get("recommended_actions").copied().unwrap_or(false) {
        let ra = gatherers::gather_recommended_actions(app_id, topology, changes);
        ctx.insert("recommended_actions", &ra);
    }
    if modules.get("historical_trends").copied().unwrap_or(false) {
        let ht = gatherers::gather_historical_trends(app_id, topology, changes, executions, 7);
        ctx.insert("historical_trends", &ht);
    }

    let mut tera = Tera::default();
    tera.add_raw_template("application_health.md", APP_HEALTH_TEMPLATE)
        .map_err(|e| ReportError::Setup(e.to_string()))?;
    tera.render("application_health.md", &ctx)
        .map_err(|e| ReportError::Render(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ReportScope, ReportStatus};
    use engine_changes::ChangeRegistry;
    use engine_identity::{ResolvedEdge, ResolvedNode, Topology};
    use engine_recovery::ExecutionRegistry;

    fn node(rid: &str, rtype: &str, health: &str) -> ResolvedNode {
        ResolvedNode {
            resource_id: rid.into(),
            resource_type: rtype.into(),
            label: rid.into(),
            attributes_json: format!(r#"{{"health_status":"{health}"}}"#),
        }
    }

    fn topo() -> Topology {
        Topology {
            nodes: vec![
                node("app:order", "Application", "normal"),
                node("comp:order-api", "ApplicationComponent", "normal"),
                node("deploy:order-api", "Deployment", "warning"),
                node("pod:order-api-1", "Pod", "critical"),
                node("pod:order-api-2", "Pod", "normal"),
            ],
            edges: vec![
                ResolvedEdge { id: "e1".into(), source: "app:order".into(), target: "comp:order-api".into(), edge_type: "CONTAINS".into() },
                ResolvedEdge { id: "e2".into(), source: "comp:order-api".into(), target: "deploy:order-api".into(), edge_type: "DEPLOYED_AS".into() },
                ResolvedEdge { id: "e3".into(), source: "deploy:order-api".into(), target: "pod:order-api-1".into(), edge_type: "CONTAINS".into() },
                ResolvedEdge { id: "e4".into(), source: "deploy:order-api".into(), target: "pod:order-api-2".into(), edge_type: "CONTAINS".into() },
            ],
        }
    }

    #[test]
    fn generates_app_health_markdown() {
        let task = ReportTask {
            report_id: "rpt-test".into(),
            template_id: ReportTemplate::ApplicationHealth,
            scope: ReportScope {
                application_id: Some("app:order".into()),
                ..Default::default()
            },
            modules: vec![],
            format: "markdown".into(),
            status: ReportStatus::Generating,
            progress: 0,
            current_step: "".into(),
            error_message: None,
            markdown: None,
            created_at: "2026-07-13T00:00:00Z".into(),
            completed_at: None,
        };
        let t = topo();
        let cr = ChangeRegistry::new();
        let er = ExecutionRegistry::new();
        let md = generate_report(&task, &t, &cr, &er, "2026-07-13T00:00:00Z").expect("render");
        assert!(md.contains("# 应用健康报告 - app:order"));
        assert!(md.contains("## 1. 健康度评分"));
        assert!(md.contains("评分")); // health_score 段
        assert!(md.contains("87")); // score 100 - 10(critical) - 3(warning)
        assert!(md.contains("## 2. 视图结论汇总"));
        assert!(md.contains("## 5. 历史趋势"));
    }

    #[test]
    fn unsupported_template_errors() {
        let task = ReportTask {
            report_id: "rpt-x".into(),
            template_id: ReportTemplate::ClusterOverview,
            scope: ReportScope::default(),
            modules: vec![],
            format: "markdown".into(),
            status: ReportStatus::Pending,
            progress: 0,
            current_step: "".into(),
            error_message: None,
            markdown: None,
            created_at: "".into(),
            completed_at: None,
        };
        let t = Topology::default();
        let cr = ChangeRegistry::new();
        let er = ExecutionRegistry::new();
        let err = generate_report(&task, &t, &cr, &er, "").unwrap_err();
        assert!(matches!(err, ReportError::UnsupportedTemplate(_)));
    }
}
