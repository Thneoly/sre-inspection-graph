//! ReportTask + ReportScope + ReportStatus + ReportStore(PRD-003 复刻,Phase 4.1)。
//!
//! 对齐 reference `app/reports/store.py`:`ReportTask` + `ReportStore`(内存 registry,
//! 对齐 3.6 registry 模式;Neo4j 持久化丢,SQLite 留后续)。

#![allow(missing_docs)]

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::ReportTemplate;

/// 报告生成状态(对齐 reference `VALID_STATUSES`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportStatus {
    Pending,
    Generating,
    Completed,
    Failed,
}

/// 报告范围(对齐 reference `ReportTask.scope`)。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReportScope {
    /// 应用 ID(application_health 模板用)。
    pub application_id: Option<String>,
    /// 集群 ID(cluster_overview 模板用;None = 全集群)。
    pub cluster_id: Option<String>,
    /// 时间范围起点(ISO8601)。
    pub time_range_start: Option<String>,
    /// 时间范围终点(ISO8601)。
    pub time_range_end: Option<String>,
}

/// 一次报告生成任务(对齐 reference `ReportTask`)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportTask {
    /// 报告 ID(`rpt-<12 hex>`)。
    pub report_id: String,
    /// 模板 ID。
    pub template_id: ReportTemplate,
    /// 范围(application_id / cluster_id / time_range)。
    pub scope: ReportScope,
    /// 启用的模块子集(按模板合法模块校验;空 = 全模块)。
    pub modules: Vec<String>,
    /// 输出格式(当前只 "markdown")。
    pub format: String,
    /// 状态。
    pub status: ReportStatus,
    /// 进度 0-100。
    pub progress: u32,
    /// 当前步骤描述。
    pub current_step: String,
    /// 失败原因(status=failed 时)。
    pub error_message: Option<String>,
    /// 生成的 Markdown(status=completed 时)。
    pub markdown: Option<String>,
    /// 创建时间(ISO8601)。
    pub created_at: String,
    /// 完成时间(ISO8601)。
    pub completed_at: Option<String>,
}

impl ReportTask {
    /// 合法模块清单(按 template_id,对齐 reference `modules_for_template`)。
    pub fn valid_modules(template: ReportTemplate) -> &'static [&'static str] {
        match template {
            ReportTemplate::ApplicationHealth => &[
                "health_score",
                "seven_views",
                "risk_list",
                "recommended_actions",
                "historical_trends",
            ],
            ReportTemplate::ClusterOverview => &[
                "cluster_health",
                "cluster_risk_top_n",
                "cluster_changes",
                "cluster_recoveries",
            ],
            ReportTemplate::IncidentReport => &[
                "incident_summary",
                "incident_timeline",
                "incident_recoveries",
            ],
        }
    }
}

/// 报告任务内存 registry(对齐 reference `ReportStore`;持久化在 orchestration 层)。
#[derive(Debug, Clone, Default)]
pub struct ReportStore {
    tasks: HashMap<String, ReportTask>,
}

impl ReportStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 从已加载任务列表构造(orchestration 从 storage 恢复用)。
    pub fn from_tasks(tasks: Vec<ReportTask>) -> Self {
        Self {
            tasks: tasks.into_iter().map(|t| (t.report_id.clone(), t)).collect(),
        }
    }

    pub fn add(&mut self, task: ReportTask) {
        self.tasks.insert(task.report_id.clone(), task);
    }

    pub fn get(&self, id: &str) -> Option<&ReportTask> {
        self.tasks.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut ReportTask> {
        self.tasks.get_mut(id)
    }

    /// 列表(新到旧,可按 template_id / application_id 过滤)。
    pub fn list(
        &self,
        template_id: Option<ReportTemplate>,
        application_id: Option<&str>,
    ) -> Vec<&ReportTask> {
        let mut v: Vec<&ReportTask> = self
            .tasks
            .values()
            .filter(|t| template_id.is_none_or(|tid| t.template_id == tid))
            .filter(|t| application_id.is_none_or(|aid| t.scope.application_id.as_deref() == Some(aid)))
            .collect();
        v.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        v
    }

    pub fn clear(&mut self) {
        self.tasks.clear();
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }
}
