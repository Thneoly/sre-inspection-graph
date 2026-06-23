//! engine-bindings
//!
//! Host-side WIT bindings 集中地。Step 4 引入 `wasmtime::component::bindgen!`
//! 宏从 `specs/wit/*.wit` 生成 host trait;现 Phase 1 仅占位,等 wasmtime 接入
//! 时再展开。
//!
//! 这样设计让 WIT 变更只重建本 crate,避免污染下游 crate 增量编译。

#![deny(unsafe_code)]
#![warn(missing_docs)]

/// Crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_present() {
        assert!(!version().is_empty());
    }
}
