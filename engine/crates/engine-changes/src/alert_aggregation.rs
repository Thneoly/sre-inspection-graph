//! alert-aggregation 视图(复刻 reference view6)。
//!
//! 与其余 4 视图不同:起点不是拓扑节点,而是 [`AlertRegistry`] 里的 firing 告警。
//! 对每个告警**合成**一个 `AlertEvent` 图节点 + 一条 `FIRED_ON` 边(alert -> `resource_ref`),
//! 再从 `resource_ref` 用既有 [`engine_identity::views::subgraph`] 展开邻域作上下文。
//! 查询时 join,**不改** AlertRegistry / Topology 模型 —— alert 是动态观测(L3),
//! topology 是结构(L2),刻意分离(对齐 reference 4 层图模型)。
//!
//! ## 偏差(对齐 reference view6)
//!
//! reference 从 resource forward 到 Application(1..4 跳);本 port 用 Both 方向 + 全边
//! 类型白名单展开 resource 邻域 —— 我们的边方向里从 Pod 正向到不了 app(无 pod→deploy
//! 正向边),Both 更鲁棒,跨 resource 类型都有效。reference 读 Neo4j AlertEvent;本 port
//! 读内存 [`AlertRegistry`](SQLite 持久化 3.6 接)。丢 reference 的 4 个额外端点(metrics /
//! inspection runs/findings / resource)。

#![allow(missing_docs)]

use std::collections::HashSet;

use engine_core::{
    summarize, types::edge_type, GraphEdge, GraphNode, GraphResponse,
};
use engine_identity::{subgraph, topology_to_graph, Topology, TraversalDir};
use serde_json::{json, Map};

use crate::{AlertEvent, AlertRegistry, AlertSeverity, AlertStatus};

/// 告警 resource 邻域展开白名单 —— 全部真实边类型(Both 方向,跨 resource 类型鲁棒)。
pub const ALERT_CONTEXT_EDGES: &[&str] = &[
    edge_type::CONTAINS,
    edge_type::DEPLOYED_AS,
    edge_type::BELONGS_TO,
    edge_type::SCHEDULED_ON,
    edge_type::RUNS,
    edge_type::ROUTES_TO,
    edge_type::EXPOSES,
    edge_type::USES,
];

/// 默认 resource 邻域展开 depth。
pub const DEFAULT_ALERT_AGGREGATION_DEPTH: usize = 3;

/// 构建 alert-aggregation [`GraphResponse`]。
///
/// - 过滤 `firing` 告警(+ 可选 `severity`)。
/// - 每个告警:合成 `AlertEvent` 节点 + `FIRED_ON` 边到 `resource_ref`(若该资源在拓扑)。
/// - 从 `resource_ref` Both 方向 subgraph(`depth`,白名单 [`ALERT_CONTEXT_EDGES`])展开邻域,
///   合并去重(同 resource 多告警只展开一次)。
/// - [`summarize`] 算 risk/health 桶。
///
/// I/O-free 纯领域逻辑,可单测。
pub fn alert_aggregation_graph(
    alerts: &AlertRegistry,
    topology: &Topology,
    severity: Option<AlertSeverity>,
    depth: usize,
) -> GraphResponse {
    let mut nodes: Vec<GraphNode> = Vec::new();
    let mut edges: Vec<GraphEdge> = Vec::new();
    let mut node_ids: HashSet<String> = HashSet::new();
    let mut edge_ids: HashSet<String> = HashSet::new();
    let mut expanded_resources: HashSet<String> = HashSet::new();

    for a in alerts.list_all().iter().filter(|a| a.status == AlertStatus::Firing) {
        if let Some(sev) = severity {
            if a.severity != sev {
                continue;
            }
        }
        // alert 节点
        if node_ids.insert(a.alert_event_id.clone()) {
            nodes.push(alert_node(a));
        }
        // resource_ref 在拓扑 -> FIRED_ON 边 + 邻域展开
        let in_topo = !a.resource_ref.is_empty()
            && topology.nodes.iter().any(|n| n.resource_id == a.resource_ref);
        if !in_topo {
            continue;
        }
        let fired_on_id = format!("{}->{}", a.alert_event_id, a.resource_ref);
        if edge_ids.insert(fired_on_id.clone()) {
            edges.push(GraphEdge {
                id: fired_on_id,
                source: a.alert_event_id.clone(),
                target: a.resource_ref.clone(),
                type_: "FIRED_ON".into(),
                properties: Map::new(),
            });
        }
        // 同 resource 只展开一次(多告警命中同一资源)
        if !expanded_resources.insert(a.resource_ref.clone()) {
            continue;
        }
        let sub = subgraph(
            topology,
            &a.resource_ref,
            depth,
            ALERT_CONTEXT_EDGES,
            TraversalDir::Both,
        );
        let sub_graph = topology_to_graph(&sub);
        for n in sub_graph.nodes {
            if node_ids.insert(n.id.clone()) {
                nodes.push(n);
            }
        }
        for e in sub_graph.edges {
            if edge_ids.insert(e.id.clone()) {
                edges.push(e);
            }
        }
    }

    let summary = summarize(&nodes, &edges);
    GraphResponse { nodes, edges, summary }
}

/// 合成告警图节点(type=`AlertEvent`,severity/status/metric 等入 properties)。
fn alert_node(a: &AlertEvent) -> GraphNode {
    let mut props = Map::new();
    props.insert("alert_name".into(), json!(a.alert_name));
    props.insert("severity".into(), json!(a.severity));
    props.insert("status".into(), json!(a.status));
    props.insert("fired_at".into(), json!(a.fired_at));
    props.insert("metric_name".into(), json!(a.metric_name));
    props.insert("metric_value".into(), json!(a.metric_value));
    props.insert("rule_id".into(), json!(a.rule_id));
    props.insert("summary".into(), json!(a.summary));
    GraphNode {
        id: a.alert_event_id.clone(),
        label: a.alert_name.clone(),
        type_: "AlertEvent".into(),
        properties: props,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_identity::{ResolvedEdge, ResolvedNode};

    fn node(id: &str, rtype: &str) -> ResolvedNode {
        ResolvedNode {
            resource_id: id.into(),
            resource_type: rtype.into(),
            label: id.into(),
            attributes_json: "{}".into(),
        }
    }
    fn edge(s: &str, t: &str, et: &str) -> ResolvedEdge {
        ResolvedEdge { id: format!("{s}->{t}"), source: s.into(), target: t.into(), edge_type: et.into() }
    }

    /// app <- BELONGS_TO <- comp <- BELONGS_TO <- deploy;svc ROUTES_TO pod;pod SCHEDULED_ON node
    fn topo() -> Topology {
        Topology {
            nodes: vec![
                node("app:a", "Application"),
                node("comp:c", "ApplicationComponent"),
                node("deploy:d", "Deployment"),
                node("pod:p1", "Pod"),
                node("svc:s", "Service"),
                node("node:n1", "Node"),
            ],
            edges: vec![
                edge("comp:c", "app:a", "BELONGS_TO"),
                edge("deploy:d", "comp:c", "BELONGS_TO"),
                edge("svc:s", "pod:p1", "ROUTES_TO"),
                edge("pod:p1", "node:n1", "SCHEDULED_ON"),
            ],
        }
    }

    fn firing_alert(id: &str, res: &str, sev: AlertSeverity) -> AlertEvent {
        let mut a = AlertEvent::new(id, "HighCPU");
        a.resource_ref = res.into();
        a.severity = sev;
        a
    }

    #[test]
    fn includes_alert_node_and_fired_on_edge() {
        let reg = AlertRegistry::from_alerts(vec![firing_alert("a1", "pod:p1", AlertSeverity::Critical)]);
        let g = alert_aggregation_graph(&reg, &topo(), None, DEFAULT_ALERT_AGGREGATION_DEPTH);
        // alert 节点在
        assert!(g.nodes.iter().any(|n| n.id == "a1" && n.type_ == "AlertEvent"));
        // FIRED_ON 边 a1 -> pod:p1
        assert!(g.edges.iter().any(|e| e.source == "a1" && e.target == "pod:p1" && e.type_ == "FIRED_ON"));
        // resource 邻域展开:pod:p1 + svc:s(ROUTES_TO)+ node:n1(SCHEDULED_ON)
        let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"pod:p1"));
        assert!(ids.contains(&"svc:s"));
        assert!(ids.contains(&"node:n1"));
    }

    #[test]
    fn skips_resolved_alerts() {
        let mut a = firing_alert("a1", "pod:p1", AlertSeverity::Critical);
        a.status = AlertStatus::Resolved;
        let reg = AlertRegistry::from_alerts(vec![a]);
        let g = alert_aggregation_graph(&reg, &topo(), None, DEFAULT_ALERT_AGGREGATION_DEPTH);
        assert!(g.nodes.is_empty(), "resolved alerts excluded");
    }

    #[test]
    fn severity_filter() {
        let reg = AlertRegistry::from_alerts(vec![
            firing_alert("a1", "pod:p1", AlertSeverity::Critical),
            firing_alert("a2", "svc:s", AlertSeverity::Warning),
        ]);
        let g = alert_aggregation_graph(&reg, &topo(), Some(AlertSeverity::Warning), DEFAULT_ALERT_AGGREGATION_DEPTH);
        assert!(g.nodes.iter().any(|n| n.id == "a2"));
        assert!(!g.nodes.iter().any(|n| n.id == "a1"), "critical filtered out");
    }

    #[test]
    fn alert_without_resource_in_topo_only_node() {
        // resource_ref 不在拓扑 -> 只 alert 节点,无 FIRED_ON 边,无邻域
        let reg = AlertRegistry::from_alerts(vec![firing_alert("a1", "pod:ghost", AlertSeverity::Critical)]);
        let g = alert_aggregation_graph(&reg, &topo(), None, DEFAULT_ALERT_AGGREGATION_DEPTH);
        assert_eq!(g.nodes.len(), 1);
        assert!(g.edges.is_empty());
    }

    #[test]
    fn multiple_alerts_same_resource_expand_once() {
        let reg = AlertRegistry::from_alerts(vec![
            firing_alert("a1", "pod:p1", AlertSeverity::Critical),
            firing_alert("a2", "pod:p1", AlertSeverity::Warning),
        ]);
        let g = alert_aggregation_graph(&reg, &topo(), None, DEFAULT_ALERT_AGGREGATION_DEPTH);
        // 两个 alert 节点 + 两条 FIRED_ON 边(各一条),但 pod:p1 邻域节点只算一次(去重)
        assert!(g.nodes.iter().any(|n| n.id == "a1"));
        assert!(g.nodes.iter().any(|n| n.id == "a2"));
        let fired_on: Vec<_> = g.edges.iter().filter(|e| e.type_ == "FIRED_ON").collect();
        assert_eq!(fired_on.len(), 2, "two FIRED_ON edges (one per alert)");
        // pod:p1 / svc:s / node:n1 各只出现一次(节点去重)
        assert_eq!(g.nodes.iter().filter(|n| n.id == "pod:p1").count(), 1);
    }
}
