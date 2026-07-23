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
pub mod types;

pub use fact::{fact_schema, Fact, FactBatch, FactError};
pub use graph::{facts_to_graph, summarize, GraphEdge, GraphNode, GraphResponse, GraphSummary};

/// Crate version (built-in from Cargo.toml).
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
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
    fn version_is_nonempty() {
        assert!(!version().is_empty());
    }
}
