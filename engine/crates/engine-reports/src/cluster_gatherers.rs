//! cluster_overview 报告采集器(PRD-003 Phase 4.2a,对齐 reference cluster_modules.py)。
//!
//! 全 I/O-free 吃 &Topology/&ChangeRegistry/&ExecutionRegistry。cluster_id prefix 匹配
//! resource_id。fault 部分 空(无 fault injection)。

#![allow(missing_docs)]

use std::collections::HashMap;

use engine_changes::{ChangeFilter, ChangeRegistry, ChangeType, Severity};
use engine_identity::Topology;
use engine_recovery::ExecutionRegistry;
use serde::Serialize;

use crate::health_score::compute_health_score;

fn all_changes_filter() -> ChangeFilter {
    ChangeFilter {
        change_type: None,
        target_resource_id: None,
        source: None,
        since: None,
        until: None,
    }
}

fn matches_cluster(resource_id: &str, cluster_id: Option<&str>) -> bool {
    match cluster_id {
        None | Some("") => true,
        Some(cid) => {
            let needle = cid.rsplit(':').next().unwrap_or(cid);
            resource_id.contains(&format!(":{needle}:")) || resource_id.starts_with(&format!("{needle}:"))
        }
    }
}

fn list_applications<'a>(topology: &'a Topology, cluster_id: Option<&str>) -> Vec<&'a engine_identity::ResolvedNode> {
    topology
        .nodes
        .iter()
        .filter(|n| n.resource_type == "Application" && matches_cluster(&n.resource_id, cluster_id))
        .collect()
}

fn change_type_str(ct: &ChangeType) -> String {
    serde_json::to_string(ct).unwrap_or_default().trim_matches('"').to_string()
}

// ===== 模块 1: cluster_health =====

/// 4 档评级计数(对齐 reference rating_counts dict;Rust 用 struct 避 Tera 中文 key 索引 bug)。
#[derive(Debug, Clone, Serialize, Default)]
pub struct ClusterRatingCounts {
    /// 健康(>=80)
    pub healthy: u32,
    /// 健康警告(60-79)
    pub health_warning: u32,
    /// 风险中(40-59)
    pub risk_medium: u32,
    /// 风险高(0-39)
    pub risk_high: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClusterHealth {
    pub cluster_id: String,
    pub total_apps: u32,
    pub rating_counts: ClusterRatingCounts,
    pub apps: Vec<ClusterAppScore>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClusterAppScore {
    pub application_id: String,
    pub name: String,
    pub score: u32,
    pub rating: String,
}

pub fn gather_cluster_health(topology: &Topology, cluster_id: Option<&str>) -> ClusterHealth {
    let apps = list_applications(topology, cluster_id);
    let mut counts = ClusterRatingCounts::default();
    let mut scores: Vec<ClusterAppScore> = Vec::new();

    for app in &apps {
        let hs = compute_health_score(&app.resource_id, topology);
        match hs.rating.as_str() {
            "健康" => counts.healthy += 1,
            "健康警告" => counts.health_warning += 1,
            "风险中" => counts.risk_medium += 1,
            "风险高" => counts.risk_high += 1,
            _ => {}
        }
        scores.push(ClusterAppScore {
            application_id: app.resource_id.clone(),
            name: app.label.clone(),
            score: hs.score,
            rating: hs.rating,
        });
    }
    scores.sort_by_key(|s| s.score); // score 低(风险高)在前

    ClusterHealth {
        cluster_id: cluster_id.unwrap_or("all").to_string(),
        total_apps: apps.len() as u32,
        rating_counts: counts,
        apps: scores,
    }
}

// ===== 模块 2: cluster_risk_top_n =====

#[derive(Debug, Clone, Serialize)]
pub struct ClusterRiskTopN {
    pub cluster_id: String,
    pub top_n: u32,
    pub top_apps: Vec<ClusterAppScore>,
    pub active_faults_total: u32,
    pub high_severity_changes_total: u32,
}

pub fn gather_cluster_risk_top_n(
    topology: &Topology,
    changes: &ChangeRegistry,
    cluster_id: Option<&str>,
    top_n: usize,
) -> ClusterRiskTopN {
    let health = gather_cluster_health(topology, cluster_id);
    let top_apps = health.apps.iter().take(top_n).cloned().collect();
    let high_changes = changes
        .list(&all_changes_filter())
        .iter()
        .filter(|c| {
            c.severity_estimate == Severity::High && matches_cluster(&c.target_resource_id, cluster_id)
        })
        .count() as u32;

    ClusterRiskTopN {
        cluster_id: cluster_id.unwrap_or("all").to_string(),
        top_n: top_n as u32,
        top_apps,
        active_faults_total: 0, // 无 fault injection
        high_severity_changes_total: high_changes,
    }
}

// ===== 模块 3: cluster_changes =====

#[derive(Debug, Clone, Serialize)]
pub struct ClusterChanges {
    pub cluster_id: String,
    pub total: u32,
    pub by_type: HashMap<String, u32>,
    pub top_targets: Vec<ClusterChangeTarget>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClusterChangeTarget {
    pub resource_id: String,
    pub changes: u32,
}

pub fn gather_cluster_changes(changes: &ChangeRegistry, cluster_id: Option<&str>) -> ClusterChanges {
    let mut by_type: HashMap<String, u32> = HashMap::new();
    let mut by_target: HashMap<String, u32> = HashMap::new();

    for c in changes.list(&all_changes_filter()) {
        if !matches_cluster(&c.target_resource_id, cluster_id) {
            continue;
        }
        *by_type.entry(change_type_str(&c.change_type)).or_insert(0) += 1;
        *by_target.entry(c.target_resource_id.clone()).or_insert(0) += 1;
    }

    let mut top_targets: Vec<ClusterChangeTarget> = by_target
        .iter()
        .map(|(rid, cnt)| ClusterChangeTarget {
            resource_id: rid.clone(),
            changes: *cnt,
        })
        .collect();
    top_targets.sort_by_key(|b| std::cmp::Reverse(b.changes));
    top_targets.truncate(5);

    ClusterChanges {
        cluster_id: cluster_id.unwrap_or("all").to_string(),
        total: by_target.values().sum(),
        by_type,
        top_targets,
    }
}

// ===== 模块 4: cluster_recoveries =====

#[derive(Debug, Clone, Serialize)]
pub struct ClusterRecoveries {
    pub cluster_id: String,
    pub total: u32,
    pub status_counts: HashMap<String, u32>,
    pub success_rate: f64,
}

pub fn gather_cluster_recoveries(executions: &ExecutionRegistry, cluster_id: Option<&str>) -> ClusterRecoveries {
    let mut status_counts: HashMap<String, u32> = HashMap::new();
    let mut total: u32 = 0;

    for e in executions.list() {
        if !matches_cluster(&e.target_resource_id, cluster_id) {
            continue;
        }
        total += 1;
        let status_str = format!("{:?}", e.status).to_lowercase();
        *status_counts.entry(status_str).or_insert(0) += 1;
    }

    let succeeded = *status_counts.get("succeeded").unwrap_or(&0);
    let failed = *status_counts.get("failed").unwrap_or(&0);
    let rolled_back = *status_counts.get("rolled_back").unwrap_or(&0);
    let terminal = succeeded + failed + rolled_back;
    let success_rate = if terminal > 0 {
        (succeeded as f64) / (terminal as f64)
    } else {
        0.0
    };

    ClusterRecoveries {
        cluster_id: cluster_id.unwrap_or("all").to_string(),
        total,
        status_counts,
        success_rate: (success_rate * 1000.0).round() / 1000.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_changes::{ChangeRequest, record_change};
    use engine_identity::{ResolvedEdge, ResolvedNode};
    use engine_recovery::{RecoveryExecution, RecoveryStatus};

    fn node(rid: &str, rtype: &str, health: &str) -> ResolvedNode {
        ResolvedNode {
            resource_id: rid.into(),
            resource_type: rtype.into(),
            label: rid.into(),
            attributes_json: format!(r#"{{"health_status":"{health}"}}"#),
        }
    }

    fn edge(src: &str, tgt: &str, etype: &str) -> ResolvedEdge {
        ResolvedEdge {
            id: format!("{src}->{tgt}"),
            source: src.into(),
            target: tgt.into(),
            edge_type: etype.into(),
        }
    }

    // app:a 无子树 -> 100 健康;app:b -CONTAINS-> 3 critical pod -> 70 健康警告
    fn topo() -> Topology {
        Topology {
            nodes: vec![
                node("otel-demo:app:a", "Application", "normal"),
                node("otel-demo:app:b", "Application", "normal"),
                node("otel-demo:pod:b1", "Pod", "critical"),
                node("otel-demo:pod:b2", "Pod", "critical"),
                node("otel-demo:pod:b3", "Pod", "critical"),
                node("prod:app:cart", "Application", "normal"),
            ],
            edges: vec![
                edge("otel-demo:app:b", "otel-demo:pod:b1", "CONTAINS"),
                edge("otel-demo:app:b", "otel-demo:pod:b2", "CONTAINS"),
                edge("otel-demo:app:b", "otel-demo:pod:b3", "CONTAINS"),
            ],
        }
    }

    #[test]
    fn cluster_health_counts_ratings_and_sorts() {
        let h = gather_cluster_health(&topo(), Some("otel-demo"));
        assert_eq!(h.total_apps, 2); // app:a + app:b;prod:app:cart 被滤
        assert_eq!(h.rating_counts.healthy, 1); // app:a 100
        assert_eq!(h.rating_counts.health_warning, 1); // app:b 70
        assert_eq!(h.rating_counts.risk_medium, 0);
        assert_eq!(h.rating_counts.risk_high, 0);
        // score 升序:b(70) 在 a(100) 前
        assert_eq!(h.apps[0].application_id, "otel-demo:app:b");
        assert_eq!(h.apps[0].score, 70);
        assert_eq!(h.apps[1].application_id, "otel-demo:app:a");
        assert_eq!(h.apps[1].score, 100);
    }

    #[test]
    fn cluster_risk_top_n_truncates_and_counts_high_changes() {
        let top = gather_cluster_risk_top_n(&topo(), &ChangeRegistry::new(), Some("otel-demo"), 1);
        assert_eq!(top.top_n, 1);
        assert_eq!(top.top_apps.len(), 1);
        assert_eq!(top.top_apps[0].application_id, "otel-demo:app:b"); // 最低分在前
        assert_eq!(top.active_faults_total, 0); // 无 fault injection
        assert_eq!(top.high_severity_changes_total, 0);
    }

    #[test]
    fn cluster_changes_by_type_and_cluster_filter() {
        let t = topo();
        let mut cr = ChangeRegistry::new();
        let req = |ctype: &str, target: &str| ChangeRequest {
            change_type: ctype.into(),
            target_resource_id: target.into(),
            ..Default::default()
        };
        record_change(&mut cr, &t, &req("configmap_updated", "otel-demo:deploy:order")).unwrap();
        record_change(&mut cr, &t, &req("secret_rotated", "otel-demo:deploy:payment")).unwrap();
        record_change(&mut cr, &t, &req("configmap_updated", "prod:deploy:cart")).unwrap();

        let cc = gather_cluster_changes(&cr, Some("otel-demo"));
        assert_eq!(cc.total, 2); // prod 变更被滤
        assert_eq!(*cc.by_type.get("configmap_updated").unwrap_or(&0), 1);
        assert_eq!(*cc.by_type.get("secret_rotated").unwrap_or(&0), 1);
        assert!(cc.top_targets.iter().any(|t| t.resource_id == "otel-demo:deploy:order"));
        assert!(!cc.top_targets.iter().any(|t| t.resource_id == "prod:deploy:cart"));
    }

    #[test]
    fn cluster_recoveries_status_rate_and_cluster_filter() {
        let mut er = ExecutionRegistry::new();
        let mk = |id: &str, target: &str, status: RecoveryStatus| RecoveryExecution {
            execution_id: id.into(),
            target_resource_id: target.into(),
            status,
            ..Default::default()
        };
        er.insert(mk("ex1", "otel-demo:deploy:order", RecoveryStatus::Succeeded));
        er.insert(mk("ex2", "otel-demo:deploy:payment", RecoveryStatus::Succeeded));
        er.insert(mk("ex3", "otel-demo:deploy:order", RecoveryStatus::Failed));
        er.insert(mk("ex4", "prod:deploy:cart", RecoveryStatus::Succeeded)); // 被滤

        let cr = gather_cluster_recoveries(&er, Some("otel-demo"));
        assert_eq!(cr.total, 3);
        assert_eq!(*cr.status_counts.get("succeeded").unwrap_or(&0), 2);
        assert_eq!(*cr.status_counts.get("failed").unwrap_or(&0), 1);
        // terminal = 2 succeeded + 1 failed = 3;rate = 2/3 ≈ 0.667(3 位小数)
        assert_eq!(cr.success_rate, 0.667);
    }
}
