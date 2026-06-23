//! engine-core
//!
//! Fact 总线 + canonical Arrow store。Phase 1 仅占位,
//! 真实实现自 Phase 2 起按 `reference/MIGRATION_STATUS.md` 推进。

#![deny(unsafe_code)]
#![warn(missing_docs)]

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
