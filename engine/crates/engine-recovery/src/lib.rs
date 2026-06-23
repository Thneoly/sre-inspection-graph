//! engine-recovery
//!
//! PRD-001 复刻 — 8 action / dry-run / approval / rollback / verifier / chain。
//! Phase 1 占位;Phase 3 起按 `reference/MIGRATION_STATUS.md` 的 Recovery 段落
//! 逐模块复刻,每模块完成 contract test `tests/contract/parity_recovery_*.rs` 后
//! 在 MIGRATION_STATUS 表标 ✅。

#![deny(unsafe_code)]
#![warn(missing_docs)]

/// 动作风险等级,与 reference 的 `RiskLevel` 字段一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// 同步执行,无需审批。
    Low,
    /// 需审批,影响面单实例。
    Medium,
    /// 需审批,影响面跨实例。
    High,
}

/// Crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn risk_levels_serialize_snake_case() {
        assert_eq!(serde_json::to_string(&RiskLevel::Low).unwrap(), "\"low\"");
    }
}
