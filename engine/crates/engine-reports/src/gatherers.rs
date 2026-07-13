//! 报告模块数据采集器(PRD-003 application_health 5 模块,对齐 reference `modules.py`)。
//!
//! 全 I/O-free 吃 `&Topology` / `&ChangeRegistry` / `&ExecutionRegistry`(reference 读全局 DSS store)。
//! fault 部分 空(Rust 无 fault injection,Phase 1 丢)。

#![allow(missing_docs)]

use std::collections::{HashMap, HashSet};

use engine_changes::{find_descendants, ChangeFilter, ChangeRegistry, ChangeType, Severity};
use engine_identity::{ResolvedNode, Topology};
use engine_recovery::{ExecutionRegistry, RecoveryStatus, suggest_for_change};
use serde::Serialize;

use crate::health_score::{compute_health_score, HealthScore};

/// 全变更 filter(无过滤,对齐 reference `store.list_change_events()`)。
fn all_changes_filter() -> ChangeFilter {
    ChangeFilter {
        change_type: None,
        target_resource_id: None,
        source: None,
        since: None,
        until: None,
    }
}

/// 子树(application + `find_descendants` 正向 BFS,3.4)。
fn subtree(application_id: &str, topology: &Topology) -> Vec<String> {
    let mut v = vec![application_id.to_string()];
    v.extend(find_descendants(application_id, topology, 6, None));
    v
}

/// 读 `attributes_json.health_status`,归一到 normal/warning/critical。
fn node_health(node: &ResolvedNode) -> &'static str {
    let attrs: serde_json::Value = serde_json::from_str(&node.attributes_json).unwrap_or(serde_json::Value::Null);
    match attrs.get("health_status").and_then(serde_json::Value::as_str).unwrap_or("normal") {
        "critical" | "red" => "critical",
        "warning" | "yellow" => "warning",
        _ => "normal",
    }
}

/// ChangeType enum -> snake_case 字符串(serde 序列化后 trim 引号)。
fn change_type_str(ct: &ChangeType) -> String {
    serde_json::to_string(ct).unwrap_or_default().trim_matches('"').to_string()
}

// ===== 模块 1: health_score =====

pub fn gather_health_score(application_id: &str, topology: &Topology) -> HealthScore {
    compute_health_score(application_id, topology)
}

// ===== 模块 2: seven_views =====

#[derive(Debug, Clone, Serialize)]
pub struct SevenViews {
    pub application_id: String,
    pub topology: SevenViewsTopology,
    pub health: SevenViewsHealth,
    /// 活跃故障列表(Rust 无 fault injection,恒空)。
    pub active_faults: Vec<serde_json::Value>,
    pub changes: SevenViewsChanges,
    pub recoveries: SevenViewsRecoveries,
}

#[derive(Debug, Clone, Serialize)]
pub struct SevenViewsTopology {
    pub components: u32,
    pub deployments: u32,
    pub pods: u32,
    pub services: u32,
    pub total_nodes: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SevenViewsHealth {
    pub normal: u32,
    pub warning: u32,
    pub critical: u32,
    pub not_ready_pods: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SevenViewsChanges {
    pub total: u32,
    pub by_type: HashMap<String, u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SevenViewsRecoveries {
    pub total: u32,
    pub succeeded: u32,
    pub failed: u32,
    pub rolled_back: u32,
}

pub fn gather_seven_views(
    application_id: &str,
    topology: &Topology,
    changes: &ChangeRegistry,
    executions: &ExecutionRegistry,
) -> SevenViews {
    let sub = subtree(application_id, topology);
    let sub_set: HashSet<&str> = sub.iter().map(|s| s.as_str()).collect();

    let mut components = 0u32;
    let mut deployments = 0u32;
    let mut pods = 0u32;
    let mut services = 0u32;
    let mut normal = 0u32;
    let mut warning = 0u32;
    let mut critical = 0u32;
    let mut not_ready_pods = 0u32;

    for rid in &sub {
        if let Some(n) = topology.nodes.iter().find(|n| &n.resource_id == rid) {
            match n.resource_type.as_str() {
                "ApplicationComponent" => components += 1,
                "Deployment" => deployments += 1,
                "Pod" => {
                    pods += 1;
                    let attrs: serde_json::Value =
                        serde_json::from_str(&n.attributes_json).unwrap_or(serde_json::Value::Null);
                    let phase = attrs.get("phase").and_then(serde_json::Value::as_str).unwrap_or("");
                    if phase != "Running" && !phase.is_empty() {
                        not_ready_pods += 1;
                    }
                }
                "Service" => services += 1,
                _ => {}
            }
            match node_health(n) {
                "normal" => normal += 1,
                "warning" => warning += 1,
                "critical" => critical += 1,
                _ => {}
            }
        }
    }

    let mut by_type: HashMap<String, u32> = HashMap::new();
    let mut changes_total = 0u32;
    for c in changes.list(&all_changes_filter()) {
        if !sub_set.contains(c.target_resource_id.as_str()) {
            continue;
        }
        changes_total += 1;
        *by_type.entry(change_type_str(&c.change_type)).or_insert(0) += 1;
    }

    let mut succeeded = 0u32;
    let mut failed = 0u32;
    let mut rolled_back = 0u32;
    let mut rec_total = 0u32;
    for e in executions.list() {
        if !sub_set.contains(e.target_resource_id.as_str()) {
            continue;
        }
        rec_total += 1;
        match e.status {
            RecoveryStatus::Succeeded => succeeded += 1,
            RecoveryStatus::Failed => failed += 1,
            RecoveryStatus::RolledBack => rolled_back += 1,
            _ => {}
        }
    }

    SevenViews {
        application_id: application_id.to_string(),
        topology: SevenViewsTopology {
            components,
            deployments,
            pods,
            services,
            total_nodes: sub.len() as u32,
        },
        health: SevenViewsHealth { normal, warning, critical, not_ready_pods },
        active_faults: vec![],
        changes: SevenViewsChanges { total: changes_total, by_type },
        recoveries: SevenViewsRecoveries { total: rec_total, succeeded, failed, rolled_back },
    }
}

// ===== 模块 3: risk_list =====

#[derive(Debug, Clone, Serialize)]
pub struct RiskList {
    pub critical: Vec<RiskEntry>,
    pub warning: Vec<RiskEntry>,
    pub change: Vec<RiskEntry>,
    pub counts: RiskCounts,
}

#[derive(Debug, Clone, Serialize)]
pub struct RiskEntry {
    pub resource_id: String,
    pub resource_type: String,
    pub name: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RiskCounts {
    pub critical: u32,
    pub warning: u32,
    pub change: u32,
}

pub fn gather_risk_list(application_id: &str, topology: &Topology, changes: &ChangeRegistry) -> RiskList {
    let sub = subtree(application_id, topology);
    let sub_set: HashSet<&str> = sub.iter().map(|s| s.as_str()).collect();

    let mut critical = Vec::new();
    let mut warning = Vec::new();
    for rid in &sub {
        if let Some(n) = topology.nodes.iter().find(|n| &n.resource_id == rid) {
            let h = node_health(n);
            let entry = RiskEntry {
                resource_id: n.resource_id.clone(),
                resource_type: n.resource_type.clone(),
                name: n.label.clone(),
                reason: format!("健康状态 {h}"),
                changed_at: None,
            };
            match h {
                "critical" => critical.push(entry),
                "warning" => warning.push(entry),
                _ => {}
            }
        }
    }

    let mut change_risks = Vec::new();
    for c in changes.list(&all_changes_filter()) {
        if !sub_set.contains(c.target_resource_id.as_str()) {
            continue;
        }
        if c.severity_estimate != Severity::High {
            continue;
        }
        let ct = change_type_str(&c.change_type);
        change_risks.push(RiskEntry {
            resource_id: c.target_resource_id.clone(),
            resource_type: c.target_resource_type.clone(),
            name: if c.description.is_empty() { ct.clone() } else { c.description.clone() },
            reason: format!("高危变更 {ct} by {}", c.changed_by),
            changed_at: Some(c.changed_at.clone()),
        });
    }

    let counts = RiskCounts {
        critical: critical.len() as u32,
        warning: warning.len() as u32,
        change: change_risks.len() as u32,
    };
    RiskList { critical, warning, change: change_risks, counts }
}

// ===== 模块 4: recommended_actions =====

#[derive(Debug, Clone, Serialize)]
pub struct RecommendedActions {
    pub actions: Vec<ActionRecommendation>,
    pub total: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActionRecommendation {
    pub action_id: String,
    pub target_resource_id: String,
    pub rationale: String,
    pub source: String,
}

pub fn gather_recommended_actions(
    application_id: &str,
    topology: &Topology,
    changes: &ChangeRegistry,
) -> RecommendedActions {
    let sub = subtree(application_id, topology);
    let sub_set: HashSet<&str> = sub.iter().map(|s| s.as_str()).collect();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut actions = Vec::new();
    // fault -> 动作:空(Rust 无 fault injection)
    // 高危变更 -> suggest_for_change(对齐 reference 模块4)
    for c in changes.list(&all_changes_filter()) {
        if !sub_set.contains(c.target_resource_id.as_str()) {
            continue;
        }
        if c.severity_estimate != Severity::High {
            continue;
        }
        let ct = change_type_str(&c.change_type);
        for sugg in suggest_for_change(&ct) {
            let key = (sugg.action.action_id.to_string(), c.target_resource_id.clone());
            if !seen.insert(key) {
                continue;
            }
            actions.push(ActionRecommendation {
                action_id: sugg.action.action_id.to_string(),
                target_resource_id: c.target_resource_id.clone(),
                rationale: sugg.rationale.to_string(),
                source: "change".to_string(),
            });
        }
    }
    let total = actions.len() as u32;
    RecommendedActions { actions, total }
}

// ===== 模块 5: historical_trends =====

#[derive(Debug, Clone, Serialize)]
pub struct HistoricalTrends {
    pub application_id: String,
    pub days: u32,
    pub rows: Vec<TrendRow>,
    pub total_changes: u32,
    pub total_recoveries: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrendRow {
    pub date: String,
    pub changes: u32,
    pub recoveries: u32,
}

pub fn gather_historical_trends(
    application_id: &str,
    topology: &Topology,
    changes: &ChangeRegistry,
    executions: &ExecutionRegistry,
    days: u32,
) -> HistoricalTrends {
    let sub = subtree(application_id, topology);
    let sub_set: HashSet<&str> = sub.iter().map(|s| s.as_str()).collect();

    let mut change_by_day: HashMap<String, u32> = HashMap::new();
    for c in changes.list(&all_changes_filter()) {
        if !sub_set.contains(c.target_resource_id.as_str()) {
            continue;
        }
        if c.changed_at.len() >= 10 {
            *change_by_day.entry(c.changed_at[..10].to_string()).or_insert(0) += 1;
        }
    }
    let mut recovery_by_day: HashMap<String, u32> = HashMap::new();
    for e in executions.list() {
        if !sub_set.contains(e.target_resource_id.as_str()) {
            continue;
        }
        if e.initiated_at.len() >= 10 {
            *recovery_by_day.entry(e.initiated_at[..10].to_string()).or_insert(0) += 1;
        }
    }

    let mut days_set: Vec<String> = change_by_day.keys().chain(recovery_by_day.keys()).cloned().collect();
    days_set.sort();
    days_set.dedup();
    let rows: Vec<TrendRow> = days_set
        .iter()
        .map(|d| TrendRow {
            date: d.clone(),
            changes: *change_by_day.get(d).unwrap_or(&0),
            recoveries: *recovery_by_day.get(d).unwrap_or(&0),
        })
        .collect();
    let total_changes: u32 = change_by_day.values().sum();
    let total_recoveries: u32 = recovery_by_day.values().sum();

    HistoricalTrends {
        application_id: application_id.to_string(),
        days,
        rows,
        total_changes,
        total_recoveries,
    }
}
