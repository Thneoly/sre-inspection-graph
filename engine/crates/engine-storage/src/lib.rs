//! engine-storage
//!
//! 持久化抽象层 — 三个 backend:
//!
//! - **SQLite**(默认 / Tauri 桌面)— 元数据 / executions / approvals / subscriptions
//! - **Parquet** — Fact 历史(append-only,按 `(date, source)` 分区)
//! - **Neo4j**(可选,feature `neo4j`)— 团队 / SaaS 模式留口子
//!
//! Phase 1 仅 trait 占位;具体实现按 storage adapter 顺序复刻。

#![deny(unsafe_code)]
#![warn(missing_docs)]

/// 通用 storage trait。每个 backend 必须实现。
pub trait Storage {
    /// backend 标识(`"sqlite"` / `"parquet"` / `"neo4j"`)。
    fn backend_name(&self) -> &'static str;
}

/// Crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoopStorage;
    impl Storage for NoopStorage {
        fn backend_name(&self) -> &'static str {
            "noop"
        }
    }

    #[test]
    fn trait_is_object_safe() {
        let s: Box<dyn Storage> = Box::new(NoopStorage);
        assert_eq!(s.backend_name(), "noop");
    }
}
