//! engine-core
//!
//! Fact 总线 + canonical Arrow store。
//!
//! - [`Fact`] —— 与 WIT `sre:inspection/connector.fact` 一一对应的 Rust 结构。
//!   guest connector 通过 `sync()` 产出 `host::HostFact`,host 端 `From<HostFact>`
//!   转成此 `Fact`(单一规范型),所有下游(storage / queryable view / Arrow Flight)
//!   只认它。
//! - [`fact_schema`] —— 7 列 Arrow `Schema`,Parquet/Arrow Flight 共用。
//! - [`FactBatch`] —— `Vec<Fact>` 转 `RecordBatch` 的批接口,做零拷贝转储。
//!
//! Phase 2 起 engine-storage 的 parquet backend 直接 `batch.into_record_batch()`
//! 写盘。engine-wasm 的 `WasmRuntime::sync_all()` 直接返 [`FactBatch`]。

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod fact;
mod graph;

pub use fact::{fact_schema, Fact, FactBatch, FactError};
pub use graph::{facts_to_graph, GraphEdge, GraphNode, GraphResponse, GraphSummary};

use serde::{Deserialize, Serialize};

/// Crate version (built-in from Cargo.toml).
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// L1 资源类型(占位枚举,Phase 2 落地完整 14 类型)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ResourceType {
    /// 占位 — 完整列表见 `doc/02-L1-L2-type-and-instance-model.md`。
    Placeholder,
}

/// Errors emitted by the engine-core layer.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// 占位错误,Phase 2 替换为具体语义。
    #[error("not yet implemented: {0}")]
    NotImplemented(&'static str),

    /// Fact 层错误(schema 不匹配、Arrow 转换失败等)。
    #[error("fact error: {0}")]
    Fact(#[from] FactError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_non_empty() {
        assert!(!version().is_empty());
    }

    #[test]
    fn resource_type_serializes() {
        let s = serde_json::to_string(&ResourceType::Placeholder).unwrap();
        assert_eq!(s, "\"Placeholder\"");
    }
}
