//! engine-reports
//!
//! PRD-003 复刻 — Markdown 报告生成 + 3 模板 + APScheduler 替换为 tokio-cron
//! + SMTP via lettre。Phase 1 占位,Phase 4 复刻。
//!
//! 模板引擎选 Tera(Rust 原生)替换 Jinja2,语法兼容度 ~90%。

#![deny(unsafe_code)]
#![warn(missing_docs)]

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
