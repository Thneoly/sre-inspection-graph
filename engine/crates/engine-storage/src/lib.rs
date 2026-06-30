//! engine-storage
//!
//! 持久化抽象层 — 三个 backend:
//!
//! - **SQLite**(默认 / Tauri 桌面)— 元数据 / executions / approvals / subscriptions
//! - **Parquet** — Fact 历史(append-only,按 `(date, source)` 分区)
//! - **Neo4j**(可选,feature `neo4j`)— 团队 / SaaS 模式留口子
//!
//! Phase 2 第一刀实现 SQLite raw Fact backend;Parquet/Neo4j 仍是后续 adapter。

#![deny(unsafe_code)]
#![warn(missing_docs)]

#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(feature = "sqlite")]
pub use sqlite::SqliteStorage;

/// 通用 storage trait。每个 backend 必须实现。
pub trait Storage {
    /// backend 标识(`"sqlite"` / `"parquet"` / `"neo4j"`)。
    fn backend_name(&self) -> &'static str;
}

/// Storage-layer errors.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// SQLite/sqlx returned an error.
    #[cfg(feature = "sqlite")]
    #[error("sqlite error: {0}")]
    Sqlx(#[from] sqlx::Error),
    /// A timestamp cannot be represented in SQLite's signed integer range.
    #[error("timestamp out of range for {field}: {value}")]
    TimestampOutOfRange {
        /// Field name.
        field: &'static str,
        /// Original unsigned timestamp.
        value: u64,
    },
    /// A stored timestamp is negative and cannot be represented as `u64`.
    #[error("negative timestamp in storage: {value}")]
    NegativeTimestamp {
        /// Stored signed timestamp.
        value: i64,
    },
    /// System clock error while stamping ingestion time.
    #[error("clock error: {0}")]
    Clock(String),
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
