//! ChangeSet —— 两次 [`Topology`] 之间的增量,Identity Resolver v0 的第二件产出。
//!
//! [`diff`] 拿「当前 materialized 拓扑」与「本次 resolve 出的新拓扑」对比,产出
//! 一个 upsert / remove 清单。engine-storage 据此对 `topology_nodes` /
//! `topology_edges` 表做最小写(变化的才 UPSERT,消失的才 DELETE),而不是每次
//! 全表重写。这也为 Phase 3 PRD-002(ChangeEvent)提供「这次同步到底变了什么」
//! 的原始信号。
//!
//! v0 的变化判定:
//! - **节点**:身份键 `resource_id`;`resource_type` / `label` / `attributes_json`
//!   任一不同即视为 updated(并入 `nodes_upserted`)。
//! - **边**:身份键 `id`(`"{source}->{target}"`);整行不等即 updated。
//! - 新出现 → upserted;在旧拓扑里、新拓扑里没了 → removed。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::topology::{ResolvedEdge, ResolvedNode, Topology};

/// 一次 resolve 相对当前 materialized 状态的增量。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChangeSet {
    /// 新增或属性变化的节点(storage 端 UPSERT)。
    pub nodes_upserted: Vec<ResolvedNode>,
    /// 已消失的节点 `resource_id`(storage 端 DELETE)。
    pub nodes_removed: Vec<String>,
    /// 新增或变化的边(storage 端 UPSERT)。
    pub edges_upserted: Vec<ResolvedEdge>,
    /// 已消失的边 `id`(storage 端 DELETE)。
    pub edges_removed: Vec<String>,
}

impl ChangeSet {
    /// 无任何变化。
    pub fn is_empty(&self) -> bool {
        self.nodes_upserted.is_empty()
            && self.nodes_removed.is_empty()
            && self.edges_upserted.is_empty()
            && self.edges_removed.is_empty()
    }

    /// 计数摘要(给 UI / 日志,不含具体行)。
    pub fn summary(&self) -> ChangeSummary {
        ChangeSummary {
            nodes_upserted: self.nodes_upserted.len(),
            nodes_removed: self.nodes_removed.len(),
            edges_upserted: self.edges_upserted.len(),
            edges_removed: self.edges_removed.len(),
        }
    }
}

/// [`ChangeSet`] 的纯计数视图 —— Tauri / 前端展示「+N ~M -K」用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChangeSummary {
    /// upsert 的节点数。
    pub nodes_upserted: usize,
    /// 删除的节点数。
    pub nodes_removed: usize,
    /// upsert 的边数。
    pub edges_upserted: usize,
    /// 删除的边数。
    pub edges_removed: usize,
}

/// 计算 `current` → `next` 的 [`ChangeSet`]。
///
/// `current` 通常来自 `storage.materialized_topology()`,`next` 来自
/// [`crate::resolve`]。纯函数,无 I/O,可单测。
pub fn diff(current: &Topology, next: &Topology) -> ChangeSet {
    let current_nodes: HashMap<&str, &ResolvedNode> = current
        .nodes
        .iter()
        .map(|n| (n.resource_id.as_str(), n))
        .collect();
    let next_node_ids: std::collections::HashSet<&str> =
        next.nodes.iter().map(|n| n.resource_id.as_str()).collect();

    let mut nodes_upserted = Vec::new();
    for n in &next.nodes {
        match current_nodes.get(n.resource_id.as_str()) {
            Some(prev) if *prev == n => {} // 完全一致 → 不动
            _ => nodes_upserted.push(n.clone()),
        }
    }
    let mut nodes_removed: Vec<String> = current
        .nodes
        .iter()
        .filter(|n| !next_node_ids.contains(n.resource_id.as_str()))
        .map(|n| n.resource_id.clone())
        .collect();
    nodes_removed.sort();

    let current_edges: HashMap<&str, &ResolvedEdge> =
        current.edges.iter().map(|e| (e.id.as_str(), e)).collect();
    let next_edge_ids: std::collections::HashSet<&str> =
        next.edges.iter().map(|e| e.id.as_str()).collect();

    let mut edges_upserted = Vec::new();
    for e in &next.edges {
        match current_edges.get(e.id.as_str()) {
            Some(prev) if *prev == e => {}
            _ => edges_upserted.push(e.clone()),
        }
    }
    let mut edges_removed: Vec<String> = current
        .edges
        .iter()
        .filter(|e| !next_edge_ids.contains(e.id.as_str()))
        .map(|e| e.id.clone())
        .collect();
    edges_removed.sort();

    ChangeSet {
        nodes_upserted,
        nodes_removed,
        edges_upserted,
        edges_removed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(rid: &str, label: &str, attrs: &str) -> ResolvedNode {
        ResolvedNode {
            resource_id: rid.into(),
            resource_type: "Pod".into(),
            label: label.into(),
            attributes_json: attrs.into(),
        }
    }
    fn edge(source: &str, target: &str) -> ResolvedEdge {
        ResolvedEdge {
            id: format!("{source}->{target}"),
            source: source.into(),
            target: target.into(),
            edge_type: "CONTAINS".into(),
        }
    }

    #[test]
    fn diff_from_empty_upserts_everything() {
        let next = Topology {
            nodes: vec![node("a", "a", "{}"), node("b", "b", "{}")],
            edges: vec![edge("a", "b")],
        };
        let cs = diff(&Topology::default(), &next);
        assert_eq!(cs.nodes_upserted.len(), 2);
        assert_eq!(cs.edges_upserted.len(), 1);
        assert!(cs.nodes_removed.is_empty());
        assert!(cs.edges_removed.is_empty());
        assert_eq!(
            cs.summary(),
            ChangeSummary {
                nodes_upserted: 2,
                nodes_removed: 0,
                edges_upserted: 1,
                edges_removed: 0,
            }
        );
    }

    #[test]
    fn diff_detects_unchanged_updated_and_removed() {
        let current = Topology {
            nodes: vec![
                node("a", "a", "{}"),         // 不变
                node("b", "old", r#"{"v":1}"#), // label + attrs 变
                node("c", "c", "{}"),         // 消失
            ],
            edges: vec![edge("a", "b"), edge("b", "c")],
        };
        let next = Topology {
            nodes: vec![
                node("a", "a", "{}"),
                node("b", "new", r#"{"v":2}"#),
                node("d", "d", "{}"), // 新增
            ],
            edges: vec![edge("a", "b")], // b->c 消失
        };
        let cs = diff(&current, &next);

        let ups: Vec<&str> = cs
            .nodes_upserted
            .iter()
            .map(|n| n.resource_id.as_str())
            .collect();
        assert_eq!(ups, vec!["b", "d"]); // a 不变,不进 upsert
        assert_eq!(cs.nodes_removed, vec!["c"]);
        // a->b 不变,b->c 消失
        assert!(cs.edges_upserted.is_empty());
        assert_eq!(cs.edges_removed, vec!["b->c"]);
    }

    #[test]
    fn diff_identical_topology_is_empty() {
        let t = Topology {
            nodes: vec![node("a", "a", "{}")],
            edges: vec![edge("a", "a")],
        };
        let cs = diff(&t, &t);
        assert!(cs.is_empty());
    }
}
