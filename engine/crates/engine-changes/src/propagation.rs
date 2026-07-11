//! ChangeEvent 影响范围反向 BFS(复刻 `reference/app/changes/propagation.py`)。
//!
//! 设计思路与 `engine_recovery::cascade::walk` 同源 -- 都基于拓扑边反向走"被谁依赖"。
//! PRD-002 简化为单一规则:固定 [`PROPAGATION_EDGES`] + 固定深度上限,不带每条规则的
//! target_type / impact 严重度推导(那是 recovery cascade 的事)。
//!
//! ## 与 reference 的差异
//!
//! - **I/O-free**:reference 读全局 DSS `store.get_all_edges()` / `store.get_node()`;
//!   本 port 把 [`engine_identity::Topology`] 作入参传入(对齐 engine-identity 纯领域 +
//!   薄持久化约定,与 `engine_recovery::cascade::dry_run` 一致)。orchestration 层
//!   (Tauri/CLI)负责从 storage 取 materialized topology 喂进来。
//! - 算法逐字对齐:`derive_propagation`(反向 incoming 索引 + depth 上限 + 排除自身)、
//!   `find_propagation_path`(反向 BFS + parents 回溯最短路径)、`find_descendants`
//!   (镜像 forward outgoing 索引)。
//! - **v0 拓扑只有 `CONTAINS` 边**(`facts_to_graph` 仅派生 CONTAINS),故生产环境传播
//!   实际只沿 CONTAINS 反向(子->父)走;算法本身认全 8 种白名单边,待 Phase 3 延后的
//!   k8s connector 边富化(USES/ROUTES_TO)接入即自动生效。契约测试用合成拓扑覆盖全边种。

#![allow(missing_docs)]

use std::collections::{HashMap, HashSet, VecDeque};

use engine_identity::Topology;

/// 影响传播沿这些"强依赖"关系(对齐 reference `PROPAGATION_EDGES`)。
///
/// `USES_IMAGE` **刻意不在**此列(`test_non_propagation_edge_skipped` 钉死)。
pub const PROPAGATION_EDGES: &[&str] = &[
    "USES",
    "CONTAINS",
    "DEPLOYED_AS",
    "BELONGS_TO",
    "RUNS",
    "SCHEDULED_ON",
    "EXPOSES",
    "ROUTES_TO",
];

/// `derive_propagation` / `find_propagation_path` 默认深度上限(对齐 reference)。
pub const DEFAULT_PROPAGATION_DEPTH: usize = 4;

/// `find_descendants` 默认深度上限(对齐 reference,比反向深一档)。
pub const DEFAULT_DESCENDANTS_DEPTH: usize = 6;

/// 从 target 沿 `edge_types`(默认 [`PROPAGATION_EDGES`])**反向** BFS,返回所有受影响
/// 资源 ID。
///
/// "反向"语义:`edge.target == current` -> `next = edge.source`。例:`(Pod) -[USES]-> (ConfigMap)`,
/// 从 ConfigMap 反向走能命中 Pod(Pod 依赖 ConfigMap,ConfigMap 变更影响 Pod)。
///
/// - 返回**不含 target 自身**。
/// - target 不在拓扑时返回 `[]`(对齐 reference `store.get_node is None`)。
/// - `max_depth`:`depth >= max_depth` 时不再展开(故 `max_depth=1` 只收 1 跳邻居)。
pub fn derive_propagation(
    target_resource_id: &str,
    topology: &Topology,
    max_depth: usize,
    edge_types: Option<&[&str]>,
) -> Vec<String> {
    if topology.nodes.iter().all(|n| n.resource_id != target_resource_id) {
        return Vec::new();
    }
    let edges_set: HashSet<&str> = edge_types.unwrap_or(PROPAGATION_EDGES).iter().copied().collect();

    // 一次性按 target 端建反向索引:incoming[edge.target] -> [edge.source, ...]
    let mut incoming: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &topology.edges {
        if !edges_set.contains(edge.edge_type.as_str()) {
            continue;
        }
        incoming.entry(edge.target.as_str()).or_default().push(edge.source.as_str());
    }

    let mut visited: HashSet<&str> = HashSet::new();
    visited.insert(target_resource_id);
    let mut propagated: Vec<String> = Vec::new();
    let mut frontier: VecDeque<(&str, usize)> = VecDeque::new();
    frontier.push_back((target_resource_id, 0));

    while let Some((node_id, depth)) = frontier.pop_front() {
        if depth >= max_depth {
            continue;
        }
        if let Some(neighbors) = incoming.get(node_id) {
            for &neighbor in neighbors {
                if visited.contains(neighbor) {
                    continue;
                }
                visited.insert(neighbor);
                propagated.push(neighbor.to_string());
                frontier.push_back((neighbor, depth + 1));
            }
        }
    }
    propagated
}

/// 重建从 `source` -> `affected` 的反向 BFS 最短路径(节点 ID 序列)。
///
/// 用于 `/correlated` 返回 propagation_distance、`/impact` 返回路径(3.5/3.6 接)。
///
/// - 返回 `[]` 表示 `affected` 不可达,或 `source == affected`,或 source 不在拓扑。
/// - 路径形态 `[source, ..., affected]`,首是 source、尾是 affected。
pub fn find_propagation_path(
    source: &str,
    affected: &str,
    topology: &Topology,
    max_depth: usize,
    edge_types: Option<&[&str]>,
) -> Vec<String> {
    if source == affected {
        return Vec::new();
    }
    if topology.nodes.iter().all(|n| n.resource_id != source) {
        return Vec::new();
    }
    let edges_set: HashSet<&str> = edge_types.unwrap_or(PROPAGATION_EDGES).iter().copied().collect();

    let mut incoming: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &topology.edges {
        if !edges_set.contains(edge.edge_type.as_str()) {
            continue;
        }
        incoming.entry(edge.target.as_str()).or_default().push(edge.source.as_str());
    }

    // parents[node] = 其反向 BFS 前驱;source 的前驱是 ""(哨兵,回溯终止)
    let mut parents: HashMap<&str, &str> = HashMap::new();
    parents.insert(source, "");
    let mut frontier: VecDeque<(&str, usize)> = VecDeque::new();
    frontier.push_back((source, 0));

    while let Some((node_id, depth)) = frontier.pop_front() {
        if depth >= max_depth {
            continue;
        }
        if let Some(neighbors) = incoming.get(node_id) {
            for &neighbor in neighbors {
                if parents.contains_key(neighbor) {
                    continue;
                }
                parents.insert(neighbor, node_id);
                if neighbor == affected {
                    // 回溯路径:affected -> ... -> source,再 reverse
                    let mut path: Vec<&str> = vec![neighbor];
                    let mut cur = node_id;
                    while !cur.is_empty() {
                        path.push(cur);
                        cur = parents.get(cur).copied().unwrap_or("");
                    }
                    return path.into_iter().rev().map(|s| s.to_string()).collect();
                }
                frontier.push_back((neighbor, depth + 1));
            }
        }
    }
    Vec::new()
}

/// 从 start **正向** BFS,返回所有"下属"资源 ID(对齐 reference `find_descendants`)。
///
/// 与 [`derive_propagation`] 镜像:derive 走反向边("谁依赖我",ConfigMap 影响范围);
/// descendants 走正向边("我下属是谁",application 子树范围)。
///
/// - 返回**不含 start 自身**。
/// - start 不在拓扑时返回 `[]`。
/// - 默认深度 [`DEFAULT_DESCENDANTS_DEPTH`](6),比反向深一档。
pub fn find_descendants(
    start: &str,
    topology: &Topology,
    max_depth: usize,
    edge_types: Option<&[&str]>,
) -> Vec<String> {
    if topology.nodes.iter().all(|n| n.resource_id != start) {
        return Vec::new();
    }
    let edges_set: HashSet<&str> = edge_types.unwrap_or(PROPAGATION_EDGES).iter().copied().collect();

    // 正向索引:outgoing[edge.source] -> [edge.target, ...]
    let mut outgoing: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &topology.edges {
        if !edges_set.contains(edge.edge_type.as_str()) {
            continue;
        }
        outgoing.entry(edge.source.as_str()).or_default().push(edge.target.as_str());
    }

    let mut visited: HashSet<&str> = HashSet::new();
    visited.insert(start);
    let mut descendants: Vec<String> = Vec::new();
    let mut frontier: VecDeque<(&str, usize)> = VecDeque::new();
    frontier.push_back((start, 0));

    while let Some((node_id, depth)) = frontier.pop_front() {
        if depth >= max_depth {
            continue;
        }
        if let Some(neighbors) = outgoing.get(node_id) {
            for &neighbor in neighbors {
                if visited.contains(neighbor) {
                    continue;
                }
                visited.insert(neighbor);
                descendants.push(neighbor.to_string());
                frontier.push_back((neighbor, depth + 1));
            }
        }
    }
    descendants
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use engine_identity::{ResolvedEdge, ResolvedNode, Topology};

    /// 构造 reference `test_change_events.py::_seed_store` 的等价拓扑(10 节点 / 10 边)。
    ///
    /// ```text
    /// app:order -CONTAINS-> comp:order-api -DEPLOYED_AS-> deploy:order-api
    ///   deploy -CONTAINS-> {pod1, pod2}
    ///   {pod1, pod2} -USES-> {cm:order-config, secret:order-db}
    /// svc:order-api -ROUTES_TO-> pod1
    /// deploy:order-api -USES_IMAGE-> img:order:1.2.3   (非白名单)
    /// orphan:lonely                                  (无依赖)
    /// ```
    pub(crate) fn fixture_topology() -> Topology {
        let n = |id: &str, t: &str, label: &str| ResolvedNode {
            resource_id: id.into(),
            resource_type: t.into(),
            label: label.into(),
            attributes_json: "{}".into(),
        };
        let e = |id: &str, src: &str, tgt: &str, rel: &str| ResolvedEdge {
            id: id.into(),
            source: src.into(),
            target: tgt.into(),
            edge_type: rel.into(),
        };
        Topology {
            nodes: vec![
                n("app:order", "Application", "订单应用"),
                n("comp:order-api", "ApplicationComponent", "订单API组件"),
                n("deploy:order-api", "Deployment", "order-api"),
                n("pod:order-api-1", "Pod", "order-api-1"),
                n("pod:order-api-2", "Pod", "order-api-2"),
                n("cm:order-config", "ConfigMap", "order-config"),
                n("secret:order-db", "Secret", "order-db-secret"),
                n("svc:order-api", "Service", "order-api-svc"),
                n("img:order:1.2.3", "ContainerImage", "order:1.2.3"),
                n("orphan:lonely", "ConfigMap", "lonely-cm"),
            ],
            edges: vec![
                e("e1", "app:order", "comp:order-api", "CONTAINS"),
                e("e2", "comp:order-api", "deploy:order-api", "DEPLOYED_AS"),
                e("e3", "deploy:order-api", "pod:order-api-1", "CONTAINS"),
                e("e4", "deploy:order-api", "pod:order-api-2", "CONTAINS"),
                e("e5", "pod:order-api-1", "cm:order-config", "USES"),
                e("e6", "pod:order-api-2", "cm:order-config", "USES"),
                e("e7", "pod:order-api-1", "secret:order-db", "USES"),
                e("e8", "pod:order-api-2", "secret:order-db", "USES"),
                e("e9", "svc:order-api", "pod:order-api-1", "ROUTES_TO"),
                e("e10", "deploy:order-api", "img:order:1.2.3", "USES_IMAGE"),
            ],
        }
    }

    fn derive(target: &str, max_depth: usize) -> Vec<String> {
        derive_propagation(target, &fixture_topology(), max_depth, None)
    }

    #[test]
    fn configmap_to_pods_one_hop() {
        // cm:order-config 反向走 USES 命中 2 个 Pod
        let propagated = derive("cm:order-config", DEFAULT_PROPAGATION_DEPTH);
        assert!(propagated.contains(&"pod:order-api-1".to_string()));
        assert!(propagated.contains(&"pod:order-api-2".to_string()));
    }

    #[test]
    fn secret_to_application_multi_hop() {
        // secret -> pods(1) -> deploy(2) -> comp(3) -> app(4)
        let propagated = derive("secret:order-db", DEFAULT_PROPAGATION_DEPTH);
        assert!(propagated.contains(&"pod:order-api-1".to_string()));
        assert!(propagated.contains(&"pod:order-api-2".to_string()));
        assert!(propagated.contains(&"deploy:order-api".to_string()));
        assert!(propagated.contains(&"comp:order-api".to_string()));
        assert!(propagated.contains(&"app:order".to_string()));
    }

    #[test]
    fn orphan_no_propagation() {
        assert_eq!(derive("orphan:lonely", DEFAULT_PROPAGATION_DEPTH), Vec::<String>::new());
    }

    #[test]
    fn max_depth_cap() {
        // depth=1 from secret 只命中 2 个 pod(1 跳),不继续往 deploy/comp/app
        let propagated = derive("secret:order-db", 1);
        assert_eq!(
            {
                let mut v = propagated.clone();
                v.sort();
                v
            },
            vec!["pod:order-api-1".to_string(), "pod:order-api-2".to_string()]
        );
    }

    #[test]
    fn non_propagation_edge_skipped() {
        // img 只有 USES_IMAGE 边过来,不在白名单 -> []
        assert_eq!(derive("img:order:1.2.3", DEFAULT_PROPAGATION_DEPTH), Vec::<String>::new());
    }

    #[test]
    fn unknown_target() {
        assert_eq!(
            derive_propagation("cm:does-not-exist", &fixture_topology(), DEFAULT_PROPAGATION_DEPTH, None),
            Vec::<String>::new()
        );
    }

    #[test]
    fn propagation_path_secret_to_app() {
        let topo = fixture_topology();
        let path = find_propagation_path(
            "secret:order-db",
            "app:order",
            &topo,
            DEFAULT_PROPAGATION_DEPTH,
            None,
        );
        // 起点是 secret,终点是 app,中间含 deploy + comp
        assert_eq!(path.first().map(|s| s.as_str()), Some("secret:order-db"));
        assert_eq!(path.last().map(|s| s.as_str()), Some("app:order"));
        assert!(path.iter().any(|s| s == "deploy:order-api"));
        assert!(path.iter().any(|s| s == "comp:order-api"));
    }

    #[test]
    fn propagation_path_source_equals_affected_is_empty() {
        let topo = fixture_topology();
        assert_eq!(
            find_propagation_path("secret:order-db", "secret:order-db", &topo, DEFAULT_PROPAGATION_DEPTH, None),
            Vec::<String>::new()
        );
    }

    #[test]
    fn propagation_path_unreachable_is_empty() {
        let topo = fixture_topology();
        // orphan 无边,不可达
        assert_eq!(
            find_propagation_path("orphan:lonely", "app:order", &topo, DEFAULT_PROPAGATION_DEPTH, None),
            Vec::<String>::new()
        );
    }

    #[test]
    fn descendants_forward_subtree_excludes_non_whitelisted() {
        let topo = fixture_topology();
        let desc = find_descendants("app:order", &topo, DEFAULT_DESCENDANTS_DEPTH, None);
        // forward: app -> comp -> deploy -> {pod1,pod2} -> {cm,secret}
        assert!(desc.contains(&"comp:order-api".to_string()));
        assert!(desc.contains(&"deploy:order-api".to_string()));
        assert!(desc.contains(&"cm:order-config".to_string()));
        assert!(desc.contains(&"secret:order-db".to_string()));
        // USES_IMAGE 非白名单 -> img 不在子树;svc 不在 app 子树
        assert!(!desc.contains(&"img:order:1.2.3".to_string()));
        assert!(!desc.contains(&"svc:order-api".to_string()));
        // 不含自身
        assert!(!desc.contains(&"app:order".to_string()));
    }

    #[test]
    fn descendants_unknown_start_is_empty() {
        assert_eq!(
            find_descendants("nope", &fixture_topology(), DEFAULT_DESCENDANTS_DEPTH, None),
            Vec::<String>::new()
        );
    }
}
