//! engine-identity
//!
//! Identity Resolver — DataFusion SQL over Arrow facts。Phase 2 起填实,
//! 现仅占位以让 workspace `cargo check` 通过。

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
    fn version_is_non_empty() {
        assert!(!version().is_empty());
    }
}
