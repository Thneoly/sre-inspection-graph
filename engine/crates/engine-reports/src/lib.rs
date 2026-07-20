//! engine-reports
//!
//! PRD-003 复刻 — Markdown 报告生成 + 3 模板 + APScheduler 替换为 tokio-cron
//! + SMTP via lettre。Phase 1 占位,Phase 4 复刻。
//!
//! 模板引擎选 Tera(Rust 原生)替换 Jinja2,语法兼容度 ~90%。

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod models;
pub mod health_score;
pub mod gatherers;
pub mod cluster_gatherers;
pub mod incident_gatherers;
pub mod generator;
pub mod subscription;
pub mod email;
pub mod scheduler;
pub mod runner;

pub use models::{ReportScope, ReportStatus, ReportStore, ReportTask};
pub use generator::{generate_report, ReportError};
pub use subscription::{parse_cron, ReportSubscription, SubscriptionStatus, SubscriptionStore};
pub use email::{EmailError, EmailSender, InMemoryEmailSender, SentEmail};
pub use scheduler::{check_fire, default_grace, FireDecision, DEFAULT_GRACE_SECS};
pub use runner::{run_subscription, RunError, RunResult};

/// 3 个内置报告模板。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportTemplate {
    /// 单应用健康巡检报告。
    ApplicationHealth,
    /// 集群级总览报告。
    ClusterOverview,
    /// 单事件(fault / change)调查报告。
    IncidentReport,
}

/// Crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_serializes_snake_case() {
        let s = serde_json::to_string(&ReportTemplate::ApplicationHealth).unwrap();
        assert_eq!(s, "\"application_health\"");
    }
}
