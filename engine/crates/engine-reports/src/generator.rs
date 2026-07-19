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
use crate::cluster_gatherers;
use crate::incident_gatherers;
use crate::models::{ReportTask};
use crate::ReportTemplate;

const APP_HEALTH_TEMPLATE: &str = include_str!("templates/application_health.md");
const CLUSTER_OVERVIEW_TEMPLATE: &str = include_str!("templates/cluster_overview.md");
const INCIDENT_REPORT_TEMPLATE: &str = include_str!("templates/incident_report.md");

#[derive(Debug, Error)]
pub enum ReportError {
    #[error("unsupported template: {0:?}")]
    UnsupportedTemplate(ReportTemplate),
    #[error("incident anchor not found: {0}")]
    AnchorNotFound(String),
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
        ReportTemplate::ClusterOverview => {
            generate_cluster_overview(task, topology, changes, executions, generated_at)
        }
        ReportTemplate::IncidentReport => {
            generate_incident_report(task, topology, changes, executions, generated_at)
        }
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

fn generate_cluster_overview(
    task: &ReportTask,
    topology: &Topology,
    changes: &ChangeRegistry,
    executions: &ExecutionRegistry,
    generated_at: &str,
) -> Result<String, ReportError> {
    let cluster_id = task.scope.cluster_id.as_deref();

    // modules: task.modules 指定启用的;空 = 全模块(对齐 reference 默认)
    let all_modules = ReportTask::valid_modules(ReportTemplate::ClusterOverview);
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

    if modules.get("cluster_health").copied().unwrap_or(false) {
        let ch = cluster_gatherers::gather_cluster_health(topology, cluster_id);
        ctx.insert("cluster_health", &ch);
    }
    if modules.get("cluster_risk_top_n").copied().unwrap_or(false) {
        // top_n 默认 5(对齐 reference cluster_risk_top_n 默认;cluster_changes top_targets 同样截 5)
        let rt = cluster_gatherers::gather_cluster_risk_top_n(topology, changes, cluster_id, 5);
        ctx.insert("cluster_risk_top_n", &rt);
    }
    if modules.get("cluster_changes").copied().unwrap_or(false) {
        let cc = cluster_gatherers::gather_cluster_changes(changes, cluster_id);
        ctx.insert("cluster_changes", &cc);
    }
    if modules.get("cluster_recoveries").copied().unwrap_or(false) {
        let cr = cluster_gatherers::gather_cluster_recoveries(executions, cluster_id);
        ctx.insert("cluster_recoveries", &cr);
    }

    let mut tera = Tera::default();
    tera.add_raw_template("cluster_overview.md", CLUSTER_OVERVIEW_TEMPLATE)
        .map_err(|e| ReportError::Setup(e.to_string()))?;
    tera.render("cluster_overview.md", &ctx)
        .map_err(|e| ReportError::Render(e.to_string()))
}

fn generate_incident_report(
    task: &ReportTask,
    topology: &Topology,
    changes: &ChangeRegistry,
    executions: &ExecutionRegistry,
    generated_at: &str,
) -> Result<String, ReportError> {
    // 锚点解析:change_event_id(Rust 仅此;fault_id -> AnchorNotFound)
    let anchor = incident_gatherers::resolve_anchor(changes, &task.scope)
        .map_err(ReportError::AnchorNotFound)?;

    let all_modules = ReportTask::valid_modules(ReportTemplate::IncidentReport);
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
    // 头部锚点显示(Rust 仅 change 锚点)
    ctx.insert("anchor_id", &anchor.anchor_id);
    ctx.insert(
        "anchor_kind_label",
        &incident_gatherers::anchor_kind_label(&anchor.kind),
    );

    if modules.get("incident_summary").copied().unwrap_or(false) {
        let s = incident_gatherers::gather_incident_summary(&anchor, topology);
        ctx.insert("incident_summary", &s);
    }
    if modules.get("incident_timeline").copied().unwrap_or(false) {
        // window 默认 3600 秒(对齐 reference gather_incident_timeline 默认)
        let t = incident_gatherers::gather_incident_timeline(
            &anchor,
            topology,
            changes,
            executions,
            3600,
        );
        ctx.insert("incident_timeline", &t);
    }
    if modules.get("incident_recoveries").copied().unwrap_or(false) {
        let r = incident_gatherers::gather_incident_recoveries(&anchor, topology, executions);
        ctx.insert("incident_recoveries", &r);
    }

    let mut tera = Tera::default();
    tera.add_raw_template("incident_report.md", INCIDENT_REPORT_TEMPLATE)
        .map_err(|e| ReportError::Setup(e.to_string()))?;
    tera.render("incident_report.md", &ctx)
        .map_err(|e| ReportError::Render(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ReportScope, ReportStatus};
    use engine_changes::{ChangeRegistry, ChangeRequest, record_change};
    use engine_identity::{ResolvedEdge, ResolvedNode, Topology};
    use engine_recovery::{ExecutionRegistry, RecoveryExecution, RecoveryStatus};

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
    fn incident_report_anchor_not_found_without_change_event_id() {
        let task = ReportTask {
            report_id: "rpt-inc".into(),
            template_id: ReportTemplate::IncidentReport,
            scope: ReportScope::default(), // 无 change_event_id / fault_id
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
        assert!(matches!(err, ReportError::AnchorNotFound(_)));
    }

    #[test]
    fn generates_incident_report_markdown() {
        // 拓扑:app -CONTAINS-> comp -DEPLOYED_AS-> deploy
        // change 锚点 target=deploy:order-api -> 反向 BFS 受影响 [comp, app]
        let topology = Topology {
            nodes: vec![
                node("app:order", "Application", "normal"),
                node("comp:order-api", "ApplicationComponent", "normal"),
                node("deploy:order-api", "Deployment", "warning"),
            ],
            edges: vec![
                ResolvedEdge { id: "e1".into(), source: "app:order".into(), target: "comp:order-api".into(), edge_type: "CONTAINS".into() },
                ResolvedEdge { id: "e2".into(), source: "comp:order-api".into(), target: "deploy:order-api".into(), edge_type: "DEPLOYED_AS".into() },
            ],
        };

        let mut cr = ChangeRegistry::new();
        let req = ChangeRequest {
            change_type: "configmap_updated".into(),
            target_resource_id: "deploy:order-api".into(),
            changed_at: Some("2026-07-20T10:00:00Z".into()),
            ..Default::default()
        };
        let event = record_change(&mut cr, &topology, &req).unwrap();
        let ce_id = event.change_event_id.clone();

        // 一条窗口内恢复执行(target=deploy:order-api 在 propagated ∪ {target} 集合)
        let mut er = ExecutionRegistry::new();
        er.insert(RecoveryExecution {
            execution_id: "ex-aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into(),
            action_id: "restart_service".into(),
            target_resource_id: "deploy:order-api".into(),
            status: RecoveryStatus::Succeeded,
            initiated_at: "2026-07-20T10:05:00Z".into(),
            completed_at: "2026-07-20T10:05:30Z".into(),
            initiated_by: "ops".into(),
            ..Default::default()
        });

        let task = ReportTask {
            report_id: "rpt-inc".into(),
            template_id: ReportTemplate::IncidentReport,
            scope: ReportScope {
                change_event_id: Some(ce_id),
                ..Default::default()
            },
            modules: vec![],
            format: "markdown".into(),
            status: ReportStatus::Generating,
            progress: 0,
            current_step: "".into(),
            error_message: None,
            markdown: None,
            created_at: "2026-07-20T10:10:00Z".into(),
            completed_at: None,
        };

        let md = generate_report(&task, &topology, &cr, &er, "2026-07-20T10:10:00Z").expect("render");
        assert!(md.contains("# 事件报告"));
        assert!(md.contains("变更事件")); // anchor_kind_label
        assert!(md.contains("## 1. 事件摘要"));
        assert!(md.contains("deploy:order-api")); // 锚点目标
        assert!(md.contains("受影响节点总数**:**2**")); // comp + app(模板加粗致冒号两侧各 ** )
        assert!(md.contains("ApplicationComponent"));
        assert!(md.contains("## 2. 事件时间线"));
        assert!(md.contains("configmap_updated")); // change 事件入时间线
        assert!(md.contains("restart_service")); // recovery 事件入时间线 + 已执行
        assert!(md.contains("## 3. 已执行恢复 & 推荐后续"));
        assert!(md.contains("rollback_deployment")); // configmap_updated -> 推荐
    }

    #[test]
    fn generates_cluster_overview_markdown() {
        // cluster_id="otel-demo" 过滤:order+payment 命中,cart(prod)被滤掉
        let task = ReportTask {
            report_id: "rpt-cluster".into(),
            template_id: ReportTemplate::ClusterOverview,
            scope: ReportScope {
                cluster_id: Some("otel-demo".into()),
                ..Default::default()
            },
            modules: vec![],
            format: "markdown".into(),
            status: ReportStatus::Generating,
            progress: 0,
            current_step: "".into(),
            error_message: None,
            markdown: None,
            created_at: "2026-07-20T00:00:00Z".into(),
            completed_at: None,
        };
        let topology = Topology {
            nodes: vec![
                node("otel-demo:app:order", "Application", "normal"),
                node("otel-demo:app:payment", "Application", "normal"),
                node("prod:app:cart", "Application", "normal"),
            ],
            edges: vec![],
        };
        let cr = ChangeRegistry::new();
        let er = ExecutionRegistry::new();
        let md = generate_report(&task, &topology, &cr, &er, "2026-07-20T00:00:00Z").expect("render");
        assert!(md.contains("# 集群健康总览"));
        assert!(md.contains("应用总数:**2**")); // 2 apps 命中 otel-demo
        assert!(md.contains("otel-demo:app:order"));
        assert!(md.contains("otel-demo:app:payment"));
        assert!(!md.contains("prod:app:cart")); // 被集群过滤
        assert!(md.contains("## 2. 风险 Top-N"));
        assert!(md.contains("## 3. 跨应用变更汇总"));
        assert!(md.contains("## 4. 跨应用恢复执行"));
    }
}
