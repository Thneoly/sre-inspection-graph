//! engine-changes
//!
//! PRD-002 复刻 - ChangeEvent + propagation BFS + YAML diff + frequency alert
//! + AlertEvent correlation。
//!
//! **Phase 3.4 scope**(本增量):[`ChangeEvent`] 模型 + [`record_change`] + 反向 BFS
//! 传播([`derive_propagation`]/[`find_propagation_path`]/[`find_descendants`])+
//! 内存 [`ChangeRegistry`] CRUD。
//!
//! Phase 3.5 接 frequency + alert 关联 + yaml_diff + correlated_changes +
//! `suggest_for_change` 桥;3.6 接 Tauri commands + sync_all_now 管线。
//!
//! ## 与 reference 的差异(详见各模块 doc)
//!
//! - I/O-free:拓扑 / 注册表显式入参,不读全局 DSS;丢弃 Neo4j dual-write + alert 关联。
//! - v0 拓扑只有 `CONTAINS` 边,生产传播实际只沿 CONTAINS 反向;算法认全 8 种白名单边。

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod event_service;
pub mod models;
pub mod propagation;

pub use event_service::{estimate_severity, record_change, ChangeRegistry};
pub use models::{
    ChangeError, ChangeEvent, ChangeFilter, ChangeRequest, ChangeType, Severity, Source,
};
pub use propagation::{
    derive_propagation, find_descendants, find_propagation_path, DEFAULT_DESCENDANTS_DEPTH,
    DEFAULT_PROPAGATION_DEPTH, PROPAGATION_EDGES,
};

/// Crate version。
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
