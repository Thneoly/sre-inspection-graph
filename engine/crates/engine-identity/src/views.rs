//! 巡检视图子图遍历(Phase 5 复刻 reference view2-6)。
//!
//! 4 个图遍历视图(access-link / node-impact / config-impact / image-risk)共用同一
//! 原语:从起点节点 BFS、`max_depth` 限深、`edge_type` 白名单过滤、有向(forward /
//! reverse / both),返回 **induced subgraph**([`Topology`])。command 层据视图配
//! `{start, depth, whitelist, dir}`,再 [`topology_to_graph`] 得前端 `GraphResponse`。
//!
//! ## 与 engine-changes::propagation 的关系
//!
//! engine-changes 也有反向 BFS([`derive_propagation`](engine_changes propagation)),
//! 但它返节点 ID(供 change 传播 / impact 计数);本原语返**子图**(节点+边,供渲染)。
//! engine-changes 已依赖本 crate(吃 `&Topology`),故子图原语放本 crate 避免依赖环。
//! 两份 BFS 各自独立,语义不同,不强行 DRY。
//!
//! ## 方向语义(Rust 边约定 source→target)
//!
//! - [`TraversalDir::Reverse`] — 沿 `edge.target == current → next = edge.source`
//!   (找依赖者,blast radius:node ← pod ← service)。
//! - [`TraversalDir::Forward`] — 沿 `edge.source == current → next = edge.target`
//!   (子树:Application → Component → Deployment)。
//! - [`TraversalDir::Both`] — 无向(任一方向可走,access-link)。

use std::collections::HashSet;

use engine_core::types::edge_type;

use crate::{ResolvedEdge, ResolvedNode, Topology};

/// 遍历方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraversalDir {
    /// 正向 source→target(子树)。
    Forward,
    /// 反向 target→source(找依赖者 / blast radius)。
    Reverse,
    /// 无向(任一方向)。
    Both,
}

/// node-impact 视图 edge 白名单(照 reference view3 Cypher)。
///
/// 起点为 `KubernetesNode`,Reverse 找其上 pod + 路由到这些 pod 的 service。
/// `CONTROLLED_BY`/`AFFECTS`/`FIRED_ON` Rust 暂不产 —— 不匹配即无害,future-proof。
pub const NODE_IMPACT_EDGES: &[&str] = &[
    edge_type::SCHEDULED_ON,
    edge_type::CONTAINS,
    edge_type::DEPLOYED_AS,
    edge_type::BELONGS_TO,
    edge_type::RUNS,
    edge_type::CONTROLLED_BY,
    edge_type::AFFECTS,
    edge_type::FIRED_ON,
];

/// config-impact 视图 edge 白名单(照 reference view4 Cypher)。
///
/// 起点为 `Secret` / `ConfigMap`,Reverse 找 USES 它的 pod → service → deployment。
/// 与 `engine_changes::PROPAGATION_EDGES` 一致。
pub const CONFIG_IMPACT_EDGES: &[&str] = &[
    edge_type::USES,
    edge_type::CONTAINS,
    edge_type::DEPLOYED_AS,
    edge_type::BELONGS_TO,
    edge_type::RUNS,
    edge_type::SCHEDULED_ON,
    edge_type::EXPOSES,
    edge_type::ROUTES_TO,
];

/// access-link 视图 edge 白名单(照 reference view2 Cypher),Both 无向。
pub const ACCESS_LINK_EDGES: &[&str] = &[
    edge_type::ROUTES_TO,
    edge_type::EXPOSES,
    edge_type::DEPLOYED_IN,
    edge_type::BELONGS_TO,
    edge_type::CONTAINS,
    edge_type::DEPLOYED_AS,
    edge_type::RUNS,
    edge_type::SCHEDULED_ON,
];

/// image-risk 视图 edge 白名单(照 reference view5 Cypher + `USES_IMAGE`),Reverse。
///
/// reference view5 用 plain `USES`;本 port 加 `USES_IMAGE`(k8s connector 产的
/// container→image 边,语义区别于 config 的 USES,对齐 reference 模型的 USES_IMAGE 概念)。
pub const IMAGE_RISK_EDGES: &[&str] = &[
    edge_type::USES,
    edge_type::USES_IMAGE,
    edge_type::CONTAINS,
    edge_type::DEPLOYED_AS,
    edge_type::BELONGS_TO,
    edge_type::RUNS,
    edge_type::SCHEDULED_ON,
    edge_type::STORED_IN,
];

/// 从 `start` 节点出发 BFS,只走 `allowed` 里的 `edge_type`,`max_depth` 限深,
/// 方向 `dir`,返回 **induced subgraph**:含 start + 所有可达节点 + (两端都在集内
/// 且 `edge_type` 在白名单)的边。`start` 不在拓扑 -> 空 [`Topology`]。
///
/// I/O-free 纯领域逻辑,吃 `&Topology`,可单测。command 层拿结果调
/// [`topology_to_graph`] 得 `GraphResponse`。
pub fn subgraph(
    topo: &Topology,
    start: &str,
    max_depth: usize,
    allowed: &[&str],
    dir: TraversalDir,
) -> Topology {
    // start 不在拓扑 -> 空(对齐 reference:start 节点缺失返空图)
    if !topo.nodes.iter().any(|n| n.resource_id == start) {
        return Topology::default();
    }

    let allowed: HashSet<&str> = allowed.iter().copied().collect();

    // BFS 收集可达节点 ID
    let mut visited: HashSet<String> = HashSet::new();
    visited.insert(start.to_string());
    let mut frontier: std::collections::VecDeque<(String, usize)> =
        std::collections::VecDeque::new();
    frontier.push_back((start.to_string(), 0));
    while let Some((node, depth)) = frontier.pop_front() {
        if depth >= max_depth {
            continue;
        }
        for e in &topo.edges {
            if !allowed.contains(e.edge_type.as_str()) {
                continue;
            }
            let next = match dir {
                TraversalDir::Forward => {
                    if e.source == node {
                        Some(e.target.as_str())
                    } else {
                        None
                    }
                }
                TraversalDir::Reverse => {
                    if e.target == node {
                        Some(e.source.as_str())
                    } else {
                        None
                    }
                }
                TraversalDir::Both => {
                    if e.source == node {
                        Some(e.target.as_str())
                    } else if e.target == node {
                        Some(e.source.as_str())
                    } else {
                        None
                    }
                }
            };
            if let Some(n) = next {
                if visited.insert(n.to_string()) {
                    frontier.push_back((n.to_string(), depth + 1));
                }
            }
        }
    }

    // induced subgraph:节点 ID 命中 + 边两端命中 且 edge_type 在白名单
    let nodes: Vec<ResolvedNode> = topo
        .nodes
        .iter()
        .filter(|n| visited.contains(&n.resource_id))
        .cloned()
        .collect();
    let edges: Vec<ResolvedEdge> = topo
        .edges
        .iter()
        .filter(|e| {
            allowed.contains(e.edge_type.as_str())
                && visited.contains(&e.source)
                && visited.contains(&e.target)
        })
        .cloned()
        .collect();
    Topology { nodes, edges }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::types::resource_type;

    fn n(id: &str, rtype: &str) -> ResolvedNode {
        ResolvedNode {
            resource_id: id.into(),
            resource_type: rtype.into(),
            label: id.into(),
            attributes_json: "{}".into(),
        }
    }

    fn e(source: &str, target: &str, edge_type: &str) -> ResolvedEdge {
        ResolvedEdge {
            id: format!("{source}->{target}"),
            source: source.into(),
            target: target.into(),
            edge_type: edge_type.into(),
        }
    }

    /// config-impact 反向链 cm ← pod ← svc(edge source→target):
    /// USES[pod→cm], ROUTES_TO[svc→pod]。均含于 CONFIG_IMPACT_EDGES。
    fn config_chain_topo() -> Topology {
        Topology {
            nodes: vec![
                n("cm:c1", "ConfigMap"),
                n("pod:p1", "Pod"),
                n("svc:s1", "Service"),
            ],
            edges: vec![
                e("pod:p1", "cm:c1", "USES"),
                e("svc:s1", "pod:p1", "ROUTES_TO"),
            ],
        }
    }

    /// node-impact 反向:起点 KubernetesNode,经 SCHEDULED_ON 反向到其上的 pod。
    /// (NODE_IMPACT_EDGES 不含 ROUTES_TO,故不再向 svc 延伸 —— 忠实 reference view3 白名单)
    fn blast_radius_topo() -> Topology {
        Topology {
            nodes: vec![
                n("node:vm1", "KubernetesNode"),
                n("pod:p1", "Pod"),
                n("pod:p2", "Pod"),
                n("svc:s1", "Service"),
            ],
            edges: vec![
                e("pod:p1", "node:vm1", "SCHEDULED_ON"),
                e("pod:p2", "node:vm1", "SCHEDULED_ON"),
                e("svc:s1", "pod:p1", "ROUTES_TO"),
            ],
        }
    }

    #[test]
    fn subgraph_reverse_collects_dependents() {
        // config-impact 多跳反向:cm <- pod <- svc(USES + ROUTES_TO 均在白名单)
        let topo = config_chain_topo();
        let sub = subgraph(
            &topo,
            "cm:c1",
            4,
            CONFIG_IMPACT_EDGES,
            TraversalDir::Reverse,
        );
        let mut ids: Vec<&str> = sub.nodes.iter().map(|n| n.resource_id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["cm:c1", "pod:p1", "svc:s1"]);
        assert_eq!(sub.edges.len(), 2); // USES + ROUTES_TO
    }

    #[test]
    fn subgraph_node_impact_stops_at_pods() {
        // node-impact 忠实白名单:只 SCHEDULED_ON 反向到 pod,不含 ROUTES_TO(到不了 svc)
        let topo = blast_radius_topo();
        let sub = subgraph(
            &topo,
            "node:vm1",
            4,
            NODE_IMPACT_EDGES,
            TraversalDir::Reverse,
        );
        let mut ids: Vec<&str> = sub.nodes.iter().map(|n| n.resource_id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["node:vm1", "pod:p1", "pod:p2"]);
        assert_eq!(sub.edges.len(), 2); // 2 SCHEDULED_ON;ROUTES_TO 不在白名单
    }

    #[test]
    fn subgraph_forward_collects_descendants() {
        // CONTAINS[app→comp→deploy]
        let topo = Topology {
            nodes: vec![
                n("app:a", "Application"),
                n("comp:c1", "ApplicationComponent"),
                n("deploy:d1", "Deployment"),
            ],
            edges: vec![
                e("app:a", "comp:c1", "CONTAINS"),
                e("comp:c1", "deploy:d1", "DEPLOYED_AS"),
            ],
        };
        let sub = subgraph(
            &topo,
            "app:a",
            4,
            CONFIG_IMPACT_EDGES,
            TraversalDir::Forward,
        );
        let mut ids: Vec<&str> = sub.nodes.iter().map(|n| n.resource_id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["app:a", "comp:c1", "deploy:d1"]);
        assert_eq!(sub.edges.len(), 2);
    }

    #[test]
    fn subgraph_both_undirected() {
        // undirected 应跨方向可达:svc --ROUTES_TO--> pod(pod 是 target),
        // 从 pod Both 走能到 svc(forward 不行,reverse 行;Both 行)
        let topo = blast_radius_topo();
        let sub = subgraph(
            &topo,
            "pod:p1",
            4,
            ACCESS_LINK_EDGES,
            TraversalDir::Both,
        );
        let ids: Vec<&str> = sub.nodes.iter().map(|n| n.resource_id.as_str()).collect();
        // pod:p1 + svc:s1(ROUTES_TO 反向)+ node:vm1(SCHEDULED_ON 反向)
        assert!(ids.contains(&"pod:p1"));
        assert!(ids.contains(&"svc:s1"));
        assert!(ids.contains(&"node:vm1"));
    }

    #[test]
    fn subgraph_respects_depth_limit() {
        // 链 a->b->c->d(CONTAINS),depth 2 from a -> {a,b,c},不含 d
        let topo = Topology {
            nodes: vec![n("a", "X"), n("b", "X"), n("c", "X"), n("d", "X")],
            edges: vec![
                e("a", "b", "CONTAINS"),
                e("b", "c", "CONTAINS"),
                e("c", "d", "CONTAINS"),
            ],
        };
        let sub = subgraph(&topo, "a", 2, CONFIG_IMPACT_EDGES, TraversalDir::Forward);
        let ids: Vec<&str> = sub.nodes.iter().map(|n| n.resource_id.as_str()).collect();
        assert!(ids.contains(&"a") && ids.contains(&"b") && ids.contains(&"c"));
        assert!(!ids.contains(&"d"));
    }

    #[test]
    fn subgraph_filters_edge_whitelist() {
        // EXPOSES 不在 NODE_IMPACT_EDGES 白名单 -> 不走,svc 不可达
        let topo = Topology {
            nodes: vec![n("node:vm1", "KubernetesNode"), n("svc:s1", "Service")],
            edges: vec![e("svc:s1", "node:vm1", "EXPOSES")],
        };
        let sub = subgraph(
            &topo,
            "node:vm1",
            4,
            NODE_IMPACT_EDGES,
            TraversalDir::Reverse,
        );
        assert_eq!(sub.nodes.len(), 1); // 仅 start
        assert!(sub.edges.is_empty());
    }

    #[test]
    fn subgraph_includes_only_edges_within_set() {
        // pod:p2 有条到拓扑外节点 cm:orphan 的 USES 边;cm:orphan 不在白名单可达集
        // (USE 在白名单但 cm 经 pod:p2 reverse 不该被纳入? Reverse from node: pod:p2 可达,
        // pod:p2 是某 edge 的 source? USES[pod:p2->cm:orphan] target=cm, source=pod:p2.
        // Reverse from pod:p2: edge.target==pod:p2? 无。故 cm 不进集,该边 induced 时排除)
        let topo = Topology {
            nodes: vec![
                n("node:vm1", "KubernetesNode"),
                n("pod:p2", "Pod"),
                n("cm:orphan", "ConfigMap"),
            ],
            edges: vec![
                e("pod:p2", "node:vm1", "SCHEDULED_ON"),
                e("pod:p2", "cm:orphan", "USES"),
            ],
        };
        let sub = subgraph(
            &topo,
            "node:vm1",
            4,
            NODE_IMPACT_EDGES,
            TraversalDir::Reverse,
        );
        // 可达集 {node, pod:p2};cm:orphan 不可达(reverse 不走 source 方向)
        let ids: Vec<&str> = sub.nodes.iter().map(|n| n.resource_id.as_str()).collect();
        assert!(!ids.contains(&"cm:orphan"));
        // USES 边一端(cm)不在集 -> induced 排除;仅 SCHEDULED_ON 进结果
        assert_eq!(sub.edges.len(), 1);
        assert_eq!(sub.edges[0].edge_type, "SCHEDULED_ON");
    }

    #[test]
    fn subgraph_missing_start_returns_empty() {
        let topo = blast_radius_topo();
        let sub = subgraph(
            &topo,
            "node:nonexistent",
            4,
            NODE_IMPACT_EDGES,
            TraversalDir::Reverse,
        );
        assert!(sub.is_empty());
    }

    /// Realistic-fixture 集成测试:用 k8s connector 真实产出的 resource_type /
    /// edge_type 词表(非合成名)跑 4 个视图,锁住类型契约 + 验算法对真实形状生效。
    ///
    /// 防 [[resource-type-vocab-drift]] 类 bug:reference Neo4j label(KubernetesNode)
    /// ≠ Rust resource_type(Node)。此 fixture 用真名,若 connector 改了 Node 类型名,
    /// 前端 `list_resources_by_types(["Node"])` 会空 —— 此处文档化正确词表。
    fn realistic_k8s_topology() -> Topology {
        // 真集群 otel-demo 切片:app -> comp -> deploy -> pod -> node/svc/cm/secret/container
        // 类型名引用 engine_core::types::resource_type(canonical 注册表)—— 此 fixture
        // 是 host 侧 canonical 拼写参考,防 [[resource-type-vocab-drift]] 类 bug。
        Topology {
            nodes: vec![
                n("app:otel-demo", resource_type::APPLICATION),
                n("comp:frontend", resource_type::APPLICATION_COMPONENT),
                n("deploy:frontend", resource_type::DEPLOYMENT),
                n("pod:frontend-1", resource_type::POD),
                n("pod:frontend-2", resource_type::POD),
                n("svc:frontend", resource_type::SERVICE),
                n("node:vm1", resource_type::NODE),
                n("cm:flagd-config", resource_type::CONFIG_MAP),
                n("secret:frontend", resource_type::SECRET),
                n("container:frontend:main", resource_type::CONTAINER),
                n("image:otel-demo:frontend:1.0", resource_type::CONTAINER_IMAGE),
            ],
            edges: vec![
                e("app:otel-demo", "comp:frontend", edge_type::CONTAINS),
                e("comp:frontend", "deploy:frontend", edge_type::DEPLOYED_AS),
                e("deploy:frontend", "comp:frontend", edge_type::BELONGS_TO),
                e("comp:frontend", "app:otel-demo", edge_type::BELONGS_TO),
                e("pod:frontend-1", "node:vm1", edge_type::SCHEDULED_ON),
                e("pod:frontend-2", "node:vm1", edge_type::SCHEDULED_ON),
                e("svc:frontend", "pod:frontend-1", edge_type::ROUTES_TO),
                e("svc:frontend", "deploy:frontend", edge_type::EXPOSES),
                e("pod:frontend-1", "cm:flagd-config", edge_type::USES),
                e("pod:frontend-1", "secret:frontend", edge_type::USES),
                e("pod:frontend-1", "container:frontend:main", edge_type::RUNS),
                e("container:frontend:main", "image:otel-demo:frontend:1.0", edge_type::USES_IMAGE),
            ],
        }
    }

    #[test]
    fn subgraph_views_against_realistic_k8s_topology() {
        let topo = realistic_k8s_topology();

        // node-impact:起点 resource_type = `Node`(NOT KubernetesNode)。
        // SCHEDULED_ON 反向到 pod;NIE 不含 ROUTES_TO 故止于 pod(对齐真集群行为)。
        let ni = subgraph(&topo, "node:vm1", 4, NODE_IMPACT_EDGES, TraversalDir::Reverse);
        assert!(ni.nodes.len() >= 3, "node-impact: node + 2 pods, got {}", ni.nodes.len());
        assert!(ni.nodes.iter().any(|n| n.resource_type == resource_type::POD));
        assert!(ni.nodes.iter().any(|n| n.resource_type == resource_type::NODE));

        // config-impact:起点 ConfigMap。USES 反向到 pod,再 ROUTES_TO 反向到 svc。
        let ci = subgraph(&topo, "cm:flagd-config", 4, CONFIG_IMPACT_EDGES, TraversalDir::Reverse);
        assert!(ci.nodes.len() >= 3, "config-impact: cm + pod + svc, got {}", ci.nodes.len());
        assert!(ci.nodes.iter().any(|n| n.resource_type == resource_type::POD));
        assert!(ci.nodes.iter().any(|n| n.resource_type == resource_type::SERVICE));

        // access-link:起点 Application,Both 无向,遍历 CONTAINS/DEPLOYED_AS/BELONGS_TO。
        let al = subgraph(&topo, "app:otel-demo", 5, ACCESS_LINK_EDGES, TraversalDir::Both);
        assert!(al.nodes.len() >= 3, "access-link: app subtree, got {}", al.nodes.len());
        assert!(al.nodes.iter().any(|n| n.resource_type == resource_type::APPLICATION_COMPONENT));

        // image-risk:起点 ContainerImage。USES_IMAGE 反向到 container,再 RUNS 反向到 pod。
        let ir = subgraph(
            &topo,
            "image:otel-demo:frontend:1.0",
            4,
            IMAGE_RISK_EDGES,
            TraversalDir::Reverse,
        );
        assert!(!ir.is_empty(), "image-risk: image -> container -> pod, got empty");
        assert!(ir.nodes.iter().any(|n| n.resource_type == resource_type::CONTAINER_IMAGE));
        assert!(ir.nodes.iter().any(|n| n.resource_type == resource_type::CONTAINER));
        assert!(ir.nodes.iter().any(|n| n.resource_type == resource_type::POD));
    }
}
