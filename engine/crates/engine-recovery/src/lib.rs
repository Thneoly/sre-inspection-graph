//! engine-recovery
//!
//! PRD-001 复刻 - 8 action / dry-run / approval / rollback / verifier / chain。
//! Phase 3 起按 `reference/MIGRATION_STATUS.md` 的 Recovery 段落逐模块复刻,每模块完成
//! contract test 后在 MIGRATION_STATUS 表标 ✅。
//!
//! ## Phase 3.1 范围(本切片)
//!
//! - [`action_defs`]:8 个 `ActionDef` 元数据 + `propagation` 规则 + rule/change 推荐。
//! - [`cascade`]:`dry_run` 影响范围 BFS(I/O-free,吃 `&Topology`)。
//!
//! ## 后续切片
//!
//! - 3.2:`execution` 管线 + `approval`(桌面单机确认门)+ rollback + mock handler +
//!   SQLite executions 表。
//! - 3.3:`verifiers` + auto-rollback + `chains`。
//!
//! ## 审批语义(Phase 3 决策,doc/14 §9)
//!
//! 桌面单机语境,审批 = 操作者确认门(保留 risk->status 映射,丢 reference 的
//! `approver_team` / 24h TTL / 多人 approve-reject)。3.2 落地。

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod action_defs;
pub mod cascade;
pub mod chains;
pub mod execution;
pub mod handlers;
pub mod models;
pub mod verifiers;

pub use action_defs::{
    get_action, list_actions, list_actions_filtered, suggest_for_change, suggest_for_rule,
    ActionDef, ActionSuggestion, Direction, Impact, ParamKind, ParamSpec, PropagationRule,
    RiskLevel, ACTION_DEFS, CHANGE_ACTION_SUGGESTIONS, RULE_ACTION_SUGGESTIONS,
};
pub use cascade::{dry_run, AffectedResource, DryRunResult};
pub use chains::{
    abort_chain, cancel_chain, confirm_chain, execute_chain, get_chain_template,
    list_chain_template_ids, ChainRegistry, ChainStep, ChainTemplate,
};
pub use execution::{
    cancel_execution, confirm_execution, execute, reverify, rollback, ExecutionRegistry,
};
pub use handlers::{get_handler, is_executable, HandlerFn, HANDLERS};
pub use models::{
    ChainStatus, ExecutionContext, ExecutionError, OnFailureStrategy, RecoveryChain,
    RecoveryExecution, RecoveryStatus, VerifyStatus,
};
pub use verifiers::{get_verifier, run_verifier, VerifierFn, VerifierVerdict, VERIFIERS};

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
        assert_eq!(serde_json::to_string(&RiskLevel::Medium).unwrap(), "\"medium\"");
        assert_eq!(serde_json::to_string(&RiskLevel::High).unwrap(), "\"high\"");
    }

    #[test]
    fn version_is_non_empty() {
        assert!(!version().is_empty());
    }
}
