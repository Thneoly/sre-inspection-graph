//! engine-changes
//!
//! PRD-002 复刻 - ChangeEvent + propagation BFS + YAML diff + frequency alert
//! + AlertEvent correlation + PRD-001 恢复动作推荐桥。
//!
//! **Phase 3.4**:`ChangeEvent` 模型 + `record_change` + 反向 BFS 传播 + 内存 `ChangeRegistry`。
//! **Phase 3.5**(本增量):frequency 过频检测 + alert 关联 + yaml_diff + `correlated_changes`
//! 以及 `get_recovery_suggestion` 桥(接 `engine_recovery::suggest_for_change`)。
//! **Phase 3.6** 接 Tauri commands + sync_all_now 管线 + SQLite 持久化。
//!
//! ## 与 reference 的差异(详见各模块 doc)
//!
//! - I/O-free:拓扑 / 注册表显式入参,不读全局 DSS;丢弃 Neo4j dual-write + `CORRELATED_WITH` 边。
//! - v0 拓扑只有 `CONTAINS` 边,生产传播实际只沿 CONTAINS 反向;算法认全 8 种白名单边。

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod alert_correlation;
pub mod alerts;
pub mod event_service;
pub mod frequency;
pub mod iso;
pub mod models;
pub mod propagation;
pub mod watch;
pub mod yaml_diff;

pub use alert_correlation::{
    correlate_alerts, correlate_changes_for_alert, CorrelateAlertsResult, CorrelateChangesForAlertResult,
    CorrelatedChangeForAlert, DEFAULT_ALERT_WINDOW_SECONDS, DEFAULT_CHANGE_WINDOW_SECONDS,
};
pub use alerts::{AlertEvent, AlertRegistry, AlertSeverity, AlertStatus};
pub use event_service::{
    correlated_changes, estimate_severity, get_recovery_suggestion, record_change, serialize, ChangeRegistry,
    CorrelatedChange, CorrelatedResult, DEFAULT_CORRELATED_WINDOW_SECONDS, RecoverySuggestion,
    RecoverySuggestionResult, TargetMatch,
};
pub use frequency::{
    apply_frequency_check, check_target_frequency, detect_frequent_changes, FrequentTarget, FrequencyResult,
    DEFAULT_THRESHOLD, DEFAULT_WINDOW_SECONDS,
};
pub use models::{
    ChangeError, ChangeEvent, ChangeFilter, ChangeRequest, ChangeType, Severity, Source,
};
pub use propagation::{
    derive_propagation, find_descendants, find_propagation_path, DEFAULT_DESCENDANTS_DEPTH,
    DEFAULT_PROPAGATION_DEPTH, PROPAGATION_EDGES,
};
pub use yaml_diff::{compute_yaml_diff, summarize_diff, DiffSummary, NOISE_KEYS};
pub use watch::detect_changes;

/// Crate version。
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
