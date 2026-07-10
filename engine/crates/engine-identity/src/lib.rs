//! engine-identity
//!
//! Identity Resolver —— 把 N 个 connector 产出的 canonical `Fact` 解析成单一
//! topology,并算出相对上次 materialized 状态的增量(ChangeSet)。
//!
//! ## v0 范围(Phase 2.5)
//!
//! - `resource_id` 直接当 canonical 身份键(不做 correlation-key 模糊合并 /
//!   冲突仲裁 —— 那是 PRD-005 完整版,见 doc/11 §4-5)。
//! - 派生逻辑复用 `engine-core::facts_to_graph`(单一 facts→graph 入口),本 crate
//!   只负责 (1) 平移成持久化形态 [`Topology`];(2) [`diff`] 算 [`ChangeSet`];
//!   (3) [`topology_to_graph`] 从 materialized 拓扑反建前端 `GraphResponse`。
//! - 持久化(`topology_nodes` / `topology_edges` 表)在 engine-storage,本 crate
//!   保持 **I/O-free 纯领域逻辑**,可单测。
//!
//! Phase 2.6+ 再上 DataFusion SQL / correlation-key 合并 / Unknown Dep Queue。
//!
//! ## Phase 2.7 - metric -> topology health 合并(doc/11 §4.3)
//!
//! [`health_merge`] 把 prometheus `metric` Fact 按阈值推成 health,按 field-ownership
//! (v0:最严重胜出)合进 [`Topology`] 节点的 `health` 字段。orchestration 层在
//! [`resolve`] 之后、[`diff`] 之前调 [`health_merge::merge_metric_health`]。

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod changeset;
mod health_merge;
mod topology;

pub use changeset::{diff, ChangeSet, ChangeSummary};
pub use health_merge::{
    derive_metric_health, merge_metric_health, DerivedHealth, HealthThreshold, HealthThresholds,
};
pub use topology::{resolve, topology_to_graph, ResolvedEdge, ResolvedNode, Topology};

/// Crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_non_empty() {
        assert!(!version().is_empty());
    }
}
