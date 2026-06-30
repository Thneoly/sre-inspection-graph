//! GraphResponse 契约 —— `Fact` → 前端可渲染的 `{nodes, edges, summary}`。
//!
//! 这是 **三层契约 B 层(Tauri commands)对外暴露的图响应规范型**,刻意与
//! `reference/app/models/graph.py::GraphResponse` 字段对齐(`nodes` / `edges` /
//! `summary`,node 带 `type` + `properties`,summary 带 `risk_counts` /
//! `health_counts`),为 Phase 2.6+ 把 reference 6 个巡检视图迁过来时不返工。
//!
//! ## 为什么落在 engine-core(而非 Tauri command 里)
//!
//! CLAUDE.md 反模式:**desktop/ 不写业务逻辑**。Fact→Graph 的去重、连边、
//! 统计是领域逻辑,必须可单测、可被 engine-cli / 未来 query 层复用,所以放
//! engine-core;Tauri `get_graph` 只是 `storage.latest_topology_facts()` +
//! 本函数的薄包装。
//!
//! ## 与前端旧 `factsToElements` 的关系
//!
//! Phase 1 的去重 / parent_resource_id 连边 / 悬空边过滤原本在 TypeScript
//! (`TopologyView.factsToElements`)做。2.4 把这套逻辑回收到 Rust:前端只
//! 负责把 `GraphNode`/`GraphEdge` 映射成 Cytoscape element,不再解 JSON。
//!
//! ## summary 语义(忠实移植 reference `graph_service.format_graph_response`)
//!
//! - `risk_counts` 固定上报 `high/medium/low/unknown` 四桶;
//!   `health_counts` 固定上报 `normal/warning/critical/unknown` 四桶。
//! - 节点 `properties` 缺 `risk_level` / `health_status` → 计入 `unknown`。
//! - **意外值**(如 risk_level="weird")不进任何上报桶 —— 与 reference
//!   `Counter.get("high", 0)` 只读固定 key 的行为一致(意外值被丢弃,不算 unknown)。

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::Fact;

/// 只有 `kind == "topology-node"` 的 Fact 进图。
const TOPOLOGY_NODE_KIND: &str = "topology-node";
/// attributes_json 里指向父资源的字段名 —— 据此派生 `CONTAINS` 父子边。
const PARENT_KEY: &str = "parent_resource_id";
/// 派生父子边的关系类型(对照 reference relationship_type 命名空间)。
const CONTAINS_EDGE_TYPE: &str = "CONTAINS";

/// 图节点 —— 与 reference `GraphNode` 字段一致。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    /// 节点 ID == `Fact.resource_id`(图内唯一)。
    pub id: String,
    /// 展示标签 —— `properties.label` ?? `properties.name` ?? resource_id 末段。
    pub label: String,
    /// 资源类型 == `Fact.resource_type`。JSON key 为 `type`(reference 对齐)。
    #[serde(rename = "type")]
    pub type_: String,
    /// 属性段 —— `Fact.attributes_json` 解出来的 JSON object(非法/非 object 时空)。
    pub properties: Map<String, Value>,
}

/// 图边 —— 与 reference `GraphEdge` 字段一致。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphEdge {
    /// 边 ID —— 派生边用 `"{source}->{target}"`。
    pub id: String,
    /// 起点节点 ID(父资源)。
    pub source: String,
    /// 终点节点 ID(子资源)。
    pub target: String,
    /// 关系类型。JSON key 为 `type`。派生父子边固定 `CONTAINS`。
    #[serde(rename = "type")]
    pub type_: String,
    /// 边属性。派生边带 `{"derived": true}` 标明非显式 edge fact。
    pub properties: Map<String, Value>,
}

/// 图统计摘要 —— 与 reference `GraphSummary` 字段一致。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphSummary {
    /// 节点总数。
    pub total_nodes: usize,
    /// 边总数。
    pub total_edges: usize,
    /// 风险等级分布(固定 high/medium/low/unknown 四桶)。
    pub risk_counts: BTreeMap<String, usize>,
    /// 健康状态分布(固定 normal/warning/critical/unknown 四桶)。
    pub health_counts: BTreeMap<String, usize>,
}

/// 图响应 —— 三层契约 B 层对前端暴露的规范型。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphResponse {
    /// 节点列表(按 resource_id 升序,稳定)。
    pub nodes: Vec<GraphNode>,
    /// 边列表(随节点遍历顺序产生)。
    pub edges: Vec<GraphEdge>,
    /// 统计摘要。
    pub summary: GraphSummary,
}

/// 把一批 topology Fact 转成 [`GraphResponse`]。
///
/// 步骤:
/// 1. 过滤出 `kind == "topology-node"`,按 `resource_id` 去重(timestamp 严格
///    更大者替换 —— 平局保留输入序中先出现的,与前端旧逻辑一致)。
/// 2. 节点按 `resource_id` 升序排列(稳定输出,便于前端 diff / 测试断言)。
/// 3. 每个节点解 `attributes_json`:取 `parent_resource_id` 派生 `CONTAINS` 边
///    (父在本批 + 非自环才连);取 `risk_level` / `health_status` 累计 summary。
/// 4. 悬空边(指向不存在节点)在连边时即过滤。
pub fn facts_to_graph(facts: &[Fact]) -> GraphResponse {
    // 1. dedup:resource_id → 最新 topology-node fact(严格更大替换,平局保留先到)
    let mut newest: HashMap<&str, &Fact> = HashMap::new();
    for f in facts {
        if f.kind != TOPOLOGY_NODE_KIND {
            continue;
        }
        match newest.get(f.resource_id.as_str()) {
            Some(prev) if prev.timestamp >= f.timestamp => {}
            _ => {
                newest.insert(f.resource_id.as_str(), f);
            }
        }
    }

    // 2. 按 resource_id 升序 —— 稳定输出
    let mut ordered: Vec<&Fact> = newest.into_values().collect();
    ordered.sort_by(|a, b| a.resource_id.cmp(&b.resource_id));

    let node_ids: HashSet<&str> = ordered.iter().map(|f| f.resource_id.as_str()).collect();

    let mut nodes: Vec<GraphNode> = Vec::with_capacity(ordered.len());
    let mut edges: Vec<GraphEdge> = Vec::new();

    for f in &ordered {
        let props = parse_props(&f.attributes_json);

        // 派生父子边
        if let Some(parent) = props.get(PARENT_KEY).and_then(Value::as_str) {
            if parent != f.resource_id && node_ids.contains(parent) {
                edges.push(GraphEdge {
                    id: format!("{parent}->{}", f.resource_id),
                    source: parent.to_string(),
                    target: f.resource_id.clone(),
                    type_: CONTAINS_EDGE_TYPE.to_string(),
                    properties: derived_edge_props(),
                });
            }
        }

        nodes.push(GraphNode {
            id: f.resource_id.clone(),
            label: label_for(&props, &f.resource_id),
            type_: f.resource_type.clone(),
            properties: props,
        });
    }

    let summary = summarize(&nodes, &edges);

    GraphResponse {
        nodes,
        edges,
        summary,
    }
}

/// 从已成型的节点 / 边算 [`GraphSummary`] —— 唯一的统计入口。
///
/// `facts_to_graph`(facts→graph)与 `engine-identity::topology_to_graph`
/// (materialized topology→graph)共用此函数,保证两条路径的 summary 语义
/// 不漂移。每个节点读 `properties.risk_level` / `properties.health_status`:
///
/// - 缺失 → 计入 `unknown`;
/// - **意外值**(非固定桶 key)被丢弃 —— 与 reference `Counter.get(key, 0)` 一致。
pub fn summarize(nodes: &[GraphNode], edges: &[GraphEdge]) -> GraphSummary {
    let mut risk_counter: HashMap<String, usize> = HashMap::new();
    let mut health_counter: HashMap<String, usize> = HashMap::new();

    for n in nodes {
        let risk = n
            .properties
            .get("risk_level")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        *risk_counter.entry(risk).or_insert(0) += 1;
        let health = n
            .properties
            .get("health_status")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        *health_counter.entry(health).or_insert(0) += 1;
    }

    GraphSummary {
        total_nodes: nodes.len(),
        total_edges: edges.len(),
        risk_counts: fixed_counts(&risk_counter, &["high", "medium", "low", "unknown"]),
        health_counts: fixed_counts(
            &health_counter,
            &["normal", "warning", "critical", "unknown"],
        ),
    }
}

/// 解 `attributes_json` 成 JSON object;非法 JSON 或非 object 一律返空 map
/// (节点仍渲染,只是没属性 —— 与前端旧逻辑「解析失败照样画节点」一致)。
fn parse_props(attributes_json: &str) -> Map<String, Value> {
    match serde_json::from_str::<Value>(attributes_json) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

/// 节点标签:`properties.label` ?? `properties.name` ?? resource_id 末段(按 `:` 切)。
fn label_for(props: &Map<String, Value>, resource_id: &str) -> String {
    if let Some(s) = props.get("label").and_then(Value::as_str) {
        return s.to_string();
    }
    if let Some(s) = props.get("name").and_then(Value::as_str) {
        return s.to_string();
    }
    resource_id
        .rsplit(':')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(resource_id)
        .to_string()
}

/// 派生边的固定属性。
fn derived_edge_props() -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("derived".to_string(), Value::Bool(true));
    m
}

/// 从 raw counter 抽固定 key 组成上报桶 —— 缺的 key 补 0,raw 里的意外 key 被丢。
fn fixed_counts(counter: &HashMap<String, usize>, keys: &[&str]) -> BTreeMap<String, usize> {
    keys.iter()
        .map(|k| (k.to_string(), counter.get(*k).copied().unwrap_or(0)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node_fact(resource_id: &str, resource_type: &str, ts: u64, attrs: Value) -> Fact {
        Fact::new(
            format!("id-{resource_id}-{ts}"),
            TOPOLOGY_NODE_KIND,
            "k8s-mini",
            resource_id,
            resource_type,
            ts,
            attrs.to_string(),
        )
    }

    #[test]
    fn builds_stable_nodes_and_parent_edges() {
        let facts = vec![
            node_fact(
                "pod:demo:default:web-0",
                "Pod",
                3,
                serde_json::json!({ "parent_resource_id": "ns:demo:default" }),
            ),
            node_fact("cluster:demo", "Cluster", 1, serde_json::json!({})),
            node_fact(
                "ns:demo:default",
                "Namespace",
                2,
                serde_json::json!({ "parent_resource_id": "cluster:demo" }),
            ),
        ];
        let g = facts_to_graph(&facts);

        // 节点按 resource_id 升序
        assert_eq!(
            g.nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
            vec!["cluster:demo", "ns:demo:default", "pod:demo:default:web-0"]
        );
        // 两条父子边
        assert_eq!(
            g.edges.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec![
                "cluster:demo->ns:demo:default",
                "ns:demo:default->pod:demo:default:web-0"
            ]
        );
        assert!(g.edges.iter().all(|e| e.type_ == "CONTAINS"));
        assert_eq!(g.edges[0].properties.get("derived"), Some(&Value::Bool(true)));
        assert_eq!(g.summary.total_nodes, 3);
        assert_eq!(g.summary.total_edges, 2);
    }

    #[test]
    fn keeps_newest_per_resource_and_drops_dangling_self_edges() {
        let facts = vec![
            node_fact(
                "pod:demo:default:web-0",
                "Pod",
                1,
                serde_json::json!({ "parent_resource_id": "missing-parent" }),
            ),
            // 同 resource_id,更新 ts → 覆盖;parent 指向自己 → 自环过滤
            node_fact(
                "pod:demo:default:web-0",
                "Pod",
                2,
                serde_json::json!({ "parent_resource_id": "pod:demo:default:web-0" }),
            ),
        ];
        let g = facts_to_graph(&facts);
        assert_eq!(g.nodes.len(), 1);
        // 取的是 ts=2 那条(parent=自己)→ 自环不连;ts=1 那条 parent 悬空也不连
        assert_eq!(g.edges.len(), 0);
    }

    #[test]
    fn malformed_attributes_json_still_yields_node() {
        let mut f = node_fact("svc:demo:default:web", "Service", 1, serde_json::json!({}));
        f.attributes_json = "not-json".to_string();
        let g = facts_to_graph(&[f]);
        assert_eq!(g.nodes.len(), 1);
        assert!(g.nodes[0].properties.is_empty());
        assert_eq!(g.edges.len(), 0);
    }

    #[test]
    fn label_prefers_label_then_name_then_resource_id_tail() {
        let with_label = node_fact(
            "pod:a",
            "Pod",
            1,
            serde_json::json!({ "label": "L", "name": "N" }),
        );
        let with_name = node_fact("pod:b", "Pod", 1, serde_json::json!({ "name": "N" }));
        let bare = node_fact("pod:demo:default:web-0", "Pod", 1, serde_json::json!({}));
        let g = facts_to_graph(&[with_label, with_name, bare]);
        let by_id: HashMap<&str, &GraphNode> = g.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        assert_eq!(by_id["pod:a"].label, "L");
        assert_eq!(by_id["pod:b"].label, "N");
        assert_eq!(by_id["pod:demo:default:web-0"].label, "web-0");
    }

    #[test]
    fn summary_counts_risk_and_health_with_fixed_buckets() {
        let facts = vec![
            node_fact(
                "a",
                "Pod",
                1,
                serde_json::json!({ "risk_level": "high", "health_status": "critical" }),
            ),
            node_fact(
                "b",
                "Pod",
                1,
                serde_json::json!({ "risk_level": "low", "health_status": "normal" }),
            ),
            // 缺 risk/health → unknown
            node_fact("c", "Pod", 1, serde_json::json!({})),
            // 意外值 → 不进任何上报桶(既不算 high 也不算 unknown)
            node_fact(
                "d",
                "Pod",
                1,
                serde_json::json!({ "risk_level": "weird", "health_status": "weird" }),
            ),
        ];
        let g = facts_to_graph(&facts);
        assert_eq!(g.summary.risk_counts["high"], 1);
        assert_eq!(g.summary.risk_counts["low"], 1);
        assert_eq!(g.summary.risk_counts["medium"], 0);
        assert_eq!(g.summary.risk_counts["unknown"], 1); // 仅 c;d 的 weird 被丢
        assert_eq!(g.summary.health_counts["critical"], 1);
        assert_eq!(g.summary.health_counts["normal"], 1);
        assert_eq!(g.summary.health_counts["warning"], 0);
        assert_eq!(g.summary.health_counts["unknown"], 1);
    }

    #[test]
    fn non_topology_facts_are_ignored() {
        let mut metric = node_fact("a", "Pod", 1, serde_json::json!({}));
        metric.kind = "metric".to_string();
        let node = node_fact("b", "Pod", 1, serde_json::json!({}));
        let g = facts_to_graph(&[metric, node]);
        assert_eq!(g.nodes.len(), 1);
        assert_eq!(g.nodes[0].id, "b");
    }

    #[test]
    fn empty_input_yields_empty_graph_with_zeroed_buckets() {
        let g = facts_to_graph(&[]);
        assert_eq!(g.nodes.len(), 0);
        assert_eq!(g.edges.len(), 0);
        assert_eq!(g.summary.total_nodes, 0);
        assert_eq!(g.summary.risk_counts["high"], 0);
        assert_eq!(g.summary.health_counts["normal"], 0);
        // 固定四桶都在
        assert_eq!(g.summary.risk_counts.len(), 4);
        assert_eq!(g.summary.health_counts.len(), 4);
    }

    #[test]
    fn serializes_with_reference_field_names() {
        let g = facts_to_graph(&[node_fact(
            "cluster:demo",
            "Cluster",
            1,
            serde_json::json!({ "name": "demo" }),
        )]);
        let v = serde_json::to_value(&g).unwrap();
        // node 用 `type` key(非 type_)
        assert_eq!(v["nodes"][0]["type"], "Cluster");
        assert_eq!(v["nodes"][0]["id"], "cluster:demo");
        assert_eq!(v["nodes"][0]["label"], "demo");
        assert!(v["nodes"][0]["properties"].is_object());
        assert_eq!(v["summary"]["total_nodes"], 1);
        assert!(v["summary"]["risk_counts"].is_object());
        assert!(v["summary"]["health_counts"].is_object());
    }
}
