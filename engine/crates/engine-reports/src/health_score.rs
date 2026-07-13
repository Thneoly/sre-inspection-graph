//! 健康度评分(PRD-003 模块 1,对齐 reference `health_score.py`)。
//!
//! 公式:`max(0, 100 - critical×10 - warning×3 - fault_pod×2)`
//! rating:>=80 健康 / 60-79 健康警告 / 40-59 风险中 / 0-39 风险高
//!
//! **偏差**:Rust 版无 fault injection(Phase 1 丢):critical = red-health 节点,
//! fault_pod = 0。reference 还把活跃 fault 目标计 critical + Pod 目计 fault_pod。

#![allow(missing_docs)]

use engine_changes::find_descendants;
use engine_identity::{ResolvedNode, Topology};
use serde::Serialize;

/// 健康度评分结果(对齐 reference `compute_health_score` 返回)。
#[derive(Debug, Clone, Serialize)]
pub struct HealthScore {
    pub application_id: String,
    pub score: u32,
    pub rating: String,
    pub breakdown: HealthBreakdown,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthBreakdown {
    pub critical: u32,
    pub warning: u32,
    pub fault_pod: u32,
    pub total_nodes: u32,
}

/// 计算应用健康度评分。
///
/// 子树:application + `find_descendants`(正向 BFS,3.4,沿白名单边 CONTAINS/DEPLOYED_AS/...)。
/// critical = red-health 节点;warning = yellow-health;fault_pod = 0(无 fault injection)。
/// application 不在拓扑 -> 子树空,score=100(无数据按满分,对齐 reference 子树空时)。
pub fn compute_health_score(application_id: &str, topology: &Topology) -> HealthScore {
    let mut subtree: Vec<String> = vec![application_id.to_string()];
    subtree.extend(find_descendants(application_id, topology, 6, None));

    let mut critical = 0u32;
    let mut warning = 0u32;
    for rid in &subtree {
        if let Some(n) = topology.nodes.iter().find(|n| &n.resource_id == rid) {
            match node_health(n) {
                "critical" => critical += 1,
                "warning" => warning += 1,
                _ => {}
            }
        }
    }

    let fault_pod = 0u32; // Rust 无 fault injection
    let score = 100u32.saturating_sub(critical * 10 + warning * 3 + fault_pod * 2);
    let rating = rating(score);
    let total_nodes = subtree.len() as u32;

    HealthScore {
        application_id: application_id.to_string(),
        score,
        rating,
        breakdown: HealthBreakdown {
            critical,
            warning,
            fault_pod,
            total_nodes,
        },
    }
}

/// 读 `attributes_json.health_status`,归一到 normal/warning/critical(对齐 reference `_node_health`)。
fn node_health(node: &ResolvedNode) -> &'static str {
    let attrs: serde_json::Value = serde_json::from_str(&node.attributes_json).unwrap_or(serde_json::Value::Null);
    match attrs.get("health_status").and_then(serde_json::Value::as_str).unwrap_or("normal") {
        "critical" | "red" => "critical",
        "warning" | "yellow" => "warning",
        _ => "normal",
    }
}

fn rating(score: u32) -> String {
    match score {
        s if s >= 80 => "健康".to_string(),
        s if s >= 60 => "健康警告".to_string(),
        s if s >= 40 => "风险中".to_string(),
        _ => "风险高".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_identity::{ResolvedEdge, ResolvedNode, Topology};

    fn node(rid: &str, rtype: &str, health: &str) -> ResolvedNode {
        ResolvedNode {
            resource_id: rid.into(),
            resource_type: rtype.into(),
            label: rid.into(),
            attributes_json: format!(r#"{{"health_status":"{health}"}}"#),
        }
    }

    fn topo() -> Topology {
        // app -CONTAINS-> comp -DEPLOYED_AS-> deploy -CONTAINS-> pod
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
    fn score_subtree_critical_warning() {
        // 子树 5 节点:1 critical(pod1)+ 1 warning(deploy)-> 100 - 10 - 3 = 87(健康)
        let h = compute_health_score("app:order", &topo());
        assert_eq!(h.breakdown.critical, 1);
        assert_eq!(h.breakdown.warning, 1);
        assert_eq!(h.breakdown.fault_pod, 0);
        assert_eq!(h.breakdown.total_nodes, 5);
        assert_eq!(h.score, 87);
        assert_eq!(h.rating, "健康");
    }

    #[test]
    fn unknown_app_empty_subtree_full_score() {
        let h = compute_health_score("app:missing", &topo());
        assert_eq!(h.breakdown.total_nodes, 1); // 仅 application 自身(不在拓扑,但 subtree 含它)
        assert_eq!(h.score, 100); // 无 critical/warning
    }

    #[test]
    fn rating_boundaries() {
        assert_eq!(rating(80), "健康");
        assert_eq!(rating(79), "健康警告");
        assert_eq!(rating(60), "健康警告");
        assert_eq!(rating(59), "风险中");
        assert_eq!(rating(40), "风险中");
        assert_eq!(rating(39), "风险高");
        assert_eq!(rating(0), "风险高");
    }
}
