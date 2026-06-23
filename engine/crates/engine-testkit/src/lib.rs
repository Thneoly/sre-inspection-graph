//! engine-testkit
//!
//! 共享 test fixture + parity contract runner。
//!
//! 用途:`tests/contract/parity_<module>.rs` 调用本 crate 的 helper,把 Rust
//! engine 实现的输出与 reference Python 输出 diff。Phase 1 占位。

#![deny(unsafe_code)]
#![warn(missing_docs)]

/// 占位 fixture path 计算。Phase 2 起接 Arrow JSON 录像。
pub fn fixtures_dir() -> &'static str {
    "tests/fixtures"
}

/// Crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixtures_dir_is_relative() {
        assert!(!fixtures_dir().starts_with('/'));
    }
}
