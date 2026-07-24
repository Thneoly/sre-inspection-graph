//! Canonical resolved topology —— Identity Resolver v0 的产出形态。
//!
//! ## 与 engine-core 的分工
//!
//! - `engine-core::facts_to_graph` 是**唯一**的 facts→canonical-graph 派生入口
//!   (去重 newest / `parent_resource_id` 派生 `CONTAINS` 边 / 悬空过滤)。
//! - 本模块的 [`resolve`] 复用它,只把 presentation 形态(`GraphResponse`)平移成
//!   **持久化友好形态** [`Topology`](节点 `attributes_json` 存字符串,而非解开的
//!   `properties` map),供 engine-storage 落 `topology_nodes` / `topology_edges` 表。
//! - [`topology_to_graph`] 是反向:从 materialized [`Topology`] 重建 `GraphResponse`
//!   给前端。summary 复用 `engine-core::summarize`,与 `facts_to_graph` 不漂移。
//!
//! v0 的「identity」= `resource_id` 直接当 canonical key(不做 correlation-key
//! 合并 / 冲突仲裁 —— 那是 PRD-005 完整版,见 doc/11 §5)。

use engine_core::{summarize, Fact, GraphEdge, GraphNode, GraphResponse};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// 已解析的拓扑节点 —— 落 `topology_nodes` 表的一行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedNode {
    /// Canonical 身份键。v0 == `Fact.resource_id`。
    pub resource_id: String,
    /// 资源类型(L1 14 类型 PascalCase)。
    pub resource_type: String,
    /// 展示标签(已在 `facts_to_graph` 里按 label>>name>>id 末段定好)。
    pub label: String,
    /// 属性段 —— canonical JSON 字符串(serde_json `Map` 是 `BTreeMap`,key 有序,
    /// 故同输入产同字符串,[`crate::diff`] 可按字符串相等判断属性是否变化)。
    pub attributes_json: String,
}

/// 已解析的拓扑边 —— 落 `topology_edges` 表的一行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedEdge {
    /// 边 ID —— `"{source}->{target}"`(与 `facts_to_graph` 派生边 ID 一致)。
    pub id: String,
    /// 起点节点 `resource_id`(父)。
    pub source: String,
    /// 终点节点 `resource_id`(子)。
    pub target: String,
    /// 关系类型。`CONTAINS`(`parent_resource_id` 派生)+ 富化 edge fact
    /// (`USES` / `ROUTES_TO` / `SCHEDULED_ON` 等,k8s connector 产)。
    pub edge_type: String,
}

/// 解析后的完整拓扑 —— materialized 状态的内存镜像。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Topology {
    /// 节点(按 `resource_id` 升序,继承 `facts_to_graph` 的稳定序)。
    pub nodes: Vec<ResolvedNode>,
    /// 边。
    pub edges: Vec<ResolvedEdge>,
}

impl Topology {
    /// 节点 + 边都为空。
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.edges.is_empty()
    }
}

/// 把一批 topology `Fact` 解析成 canonical [`Topology`]。
///
/// 复用 `engine_core::facts_to_graph`(单一派生入口),再把 presentation
/// `GraphResponse` 平移成持久化形态。节点 `properties` map 重新序列化为
/// canonical 字符串(key 有序)落 `attributes_json`。
///
/// **Phase 8.2(C1)**:委托前先跑 [`crate::correlation::rewrite_by_correlation_key`]
/// 合并共享 correlation key 的节点(code-repo `BUILDS` repo→image-ref 经 `image-ref:<ref>`
/// key 合并到 k8s `image:{c}:{ns}:{ref}` 节点 → repo→image→runtime 联通)。engine-core
/// `facts_to_graph` 共享契约零改(无生产调用方绕过 resolve)。
pub fn resolve(facts: &[Fact]) -> Topology {
    let rewritten = crate::correlation::rewrite_by_correlation_key(facts);
    let graph = facts_to_graph_canonical(&rewritten);
    Topology {
        nodes: graph
            .nodes
            .iter()
            .map(|n| ResolvedNode {
                resource_id: n.id.clone(),
                resource_type: n.type_.clone(),
                label: n.label.clone(),
                attributes_json: Value::Object(n.properties.clone()).to_string(),
            })
            .collect(),
        edges: graph
            .edges
            .iter()
            .map(|e| ResolvedEdge {
                id: e.id.clone(),
                source: e.source.clone(),
                target: e.target.clone(),
                edge_type: e.type_.clone(),
            })
            .collect(),
    }
}

/// 从 materialized [`Topology`] 重建前端用 [`GraphResponse`]。
///
/// 节点/边已是 canonical(去重 / 连边在 `resolve` 时完成),这里只做平移 +
/// `engine_core::summarize` 算 risk/health 统计。派生边重新带上 `{derived:true}`。
pub fn topology_to_graph(topology: &Topology) -> GraphResponse {
    let nodes: Vec<GraphNode> = topology
        .nodes
        .iter()
        .map(|n| GraphNode {
            id: n.resource_id.clone(),
            label: n.label.clone(),
            type_: n.resource_type.clone(),
            properties: parse_props(&n.attributes_json),
        })
        .collect();
    let edges: Vec<GraphEdge> = topology
        .edges
        .iter()
        .map(|e| GraphEdge {
            id: e.id.clone(),
            source: e.source.clone(),
            target: e.target.clone(),
            type_: e.edge_type.clone(),
            properties: derived_edge_props(),
        })
        .collect();
    let summary = summarize(&nodes, &edges);
    GraphResponse {
        nodes,
        edges,
        summary,
    }
}

/// 薄包装 `engine_core::facts_to_graph` —— 命名上点明「这是 canonical 派生」。
fn facts_to_graph_canonical(facts: &[Fact]) -> GraphResponse {
    engine_core::facts_to_graph(facts)
}

/// 解 canonical attributes_json 成 JSON object;非法 / 非 object 返空 map。
fn parse_props(attributes_json: &str) -> Map<String, Value> {
    match serde_json::from_str::<Value>(attributes_json) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

/// 派生边固定属性 `{derived:true}` —— 与 `facts_to_graph` 派生边一致。
fn derived_edge_props() -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("derived".to_string(), Value::Bool(true));
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node_fact(resource_id: &str, resource_type: &str, ts: u64, attrs: Value) -> Fact {
        Fact::new(
            format!("id-{resource_id}-{ts}"),
            "topology-node",
            "k8s-mini",
            resource_id,
            resource_type,
            ts,
            attrs.to_string(),
        )
    }

    #[test]
    fn resolve_dedups_and_derives_contains_edges() {
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
        let t = resolve(&facts);
        assert_eq!(
            t.nodes.iter().map(|n| n.resource_id.as_str()).collect::<Vec<_>>(),
            vec!["cluster:demo", "ns:demo:default", "pod:demo:default:web-0"]
        );
        assert_eq!(
            t.edges.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec![
                "cluster:demo->ns:demo:default",
                "ns:demo:default->pod:demo:default:web-0"
            ]
        );
        assert!(t.edges.iter().all(|e| e.edge_type == "CONTAINS"));
    }

    #[test]
    fn resolve_canonicalizes_attributes_json_with_sorted_keys() {
        // 输入 key 乱序,canonical 输出按 BTreeMap key 升序
        let f = node_fact("a", "Pod", 1, serde_json::json!({ "z": 1, "a": 2 }));
        let t = resolve(&[f]);
        assert_eq!(t.nodes[0].attributes_json, r#"{"a":2,"z":1}"#);
    }

    #[test]
    fn topology_to_graph_round_trips_resolve() {
        let facts = vec![
            node_fact(
                "ns:demo:default",
                "Namespace",
                2,
                serde_json::json!({ "parent_resource_id": "cluster:demo", "risk_level": "high" }),
            ),
            node_fact(
                "cluster:demo",
                "Cluster",
                1,
                serde_json::json!({ "health_status": "normal" }),
            ),
        ];
        // facts → graph(直接)应与 facts → resolve → topology_to_graph 等价
        let direct = engine_core::facts_to_graph(&facts);
        let via_topology = topology_to_graph(&resolve(&facts));
        assert_eq!(via_topology, direct);
    }

    #[test]
    fn topology_to_graph_recomputes_summary_buckets() {
        let t = Topology {
            nodes: vec![
                ResolvedNode {
                    resource_id: "a".into(),
                    resource_type: "Pod".into(),
                    label: "a".into(),
                    attributes_json: r#"{"risk_level":"high","health_status":"critical"}"#.into(),
                },
                ResolvedNode {
                    resource_id: "b".into(),
                    resource_type: "Pod".into(),
                    label: "b".into(),
                    attributes_json: "{}".into(),
                },
            ],
            edges: vec![],
        };
        let g = topology_to_graph(&t);
        assert_eq!(g.summary.total_nodes, 2);
        assert_eq!(g.summary.risk_counts["high"], 1);
        assert_eq!(g.summary.risk_counts["unknown"], 1);
        assert_eq!(g.summary.health_counts["critical"], 1);
        assert_eq!(g.summary.health_counts["unknown"], 1);
    }

    fn edge_fact(resource_id: &str, source: &str, target: &str, edge_type: &str, ts: u64) -> Fact {
        Fact::new(
            format!("eid-{resource_id}-{ts}"),
            "topology-edge",
            "k8s",
            resource_id,
            "Edge",
            ts,
            serde_json::json!({ "source": source, "target": target, "edge_type": edge_type })
                .to_string(),
        )
    }

    #[test]
    fn resolve_carries_uses_and_routes_to_edges() {
        // resolve 透传 facts_to_graph 的显式 edge fact 到 Topology.edges(多类型,非仅 CONTAINS)
        let facts = vec![
            node_fact("svc:s", "Service", 1, serde_json::json!({})),
            node_fact("pod:a", "Pod", 1, serde_json::json!({})),
            node_fact("cm:c", "ConfigMap", 1, serde_json::json!({})),
            edge_fact("edge:ROUTES_TO:svc:s->pod:a", "svc:s", "pod:a", "ROUTES_TO", 1),
            edge_fact("edge:USES:pod:a->cm:c", "pod:a", "cm:c", "USES", 1),
        ];
        let t = resolve(&facts);
        assert_eq!(t.nodes.len(), 3);
        assert_eq!(t.edges.len(), 2);
        let types: Vec<&str> = t.edges.iter().map(|e| e.edge_type.as_str()).collect();
        assert!(types.contains(&"ROUTES_TO"));
        assert!(types.contains(&"USES"));
    }
}
