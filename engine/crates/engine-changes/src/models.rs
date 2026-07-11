//! ChangeEvent 数据模型 + 枚举 + 入参/过滤/错误(复刻 `reference/app/datasource/models.py`
//! 的 `ChangeEvent` dataclass + `reference/app/changes/event_service.py` 的字符串枚举集合)。
//!
//! ## 与 reference 的差异
//!
//! - **枚举 vs 字符串**:reference 用 plain `str` + `VALID_CHANGE_TYPES`/`VALID_SOURCES`
//!   集合在 `record_change` 里校验;本 port 用 [`ChangeType`]/[`Source`]/[`Severity`]
//!   强类型枚举(snake_case 序列化,与 reference 字符串一致)。校验仍在 `record_change`
//!   入口做(`from_name` 解析失败 -> [`ChangeError`] code 400),对齐 reference 抛
//!   `ChangeEventError` 的契约。
//! - **Phase 2 字段并入主模型**:reference dataclass 把 `commit_sha`/`pipeline_url`/
//!   `git_repo`/`cluster_id`/`yaml_diff` 作为 Phase 2 增量字段;本 port 一次性全收
//!   (避免后续 migration),`record_change` Phase 2 语义(commit_sha 回填 related_commit)
//!   已实装。

#![allow(missing_docs)]

use serde::{Deserialize, Serialize};

/// 变更事件类型,镜像 reference `VALID_CHANGE_TYPES`(4 种,无扩展)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeType {
    /// ConfigMap 更新。
    ConfigmapUpdated,
    /// Secret 轮换。
    SecretRotated,
    /// Deployment rollout。
    DeploymentRolled,
    /// 镜像推送。
    ImagePushed,
}

impl ChangeType {
    /// 全部合法值(对齐 reference `VALID_CHANGE_TYPES`)。
    pub const ALL: &'static [&'static str] = &[
        "configmap_updated",
        "secret_rotated",
        "deployment_rolled",
        "image_pushed",
    ];

    /// 由 snake_case 字符串解析;非法返 `None`(`record_change` 据此报 400)。
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "configmap_updated" => Some(Self::ConfigmapUpdated),
            "secret_rotated" => Some(Self::SecretRotated),
            "deployment_rolled" => Some(Self::DeploymentRolled),
            "image_pushed" => Some(Self::ImagePushed),
            _ => None,
        }
    }

    /// snake_case 字符串(对齐序列化形态)。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ConfigmapUpdated => "configmap_updated",
            Self::SecretRotated => "secret_rotated",
            Self::DeploymentRolled => "deployment_rolled",
            Self::ImagePushed => "image_pushed",
        }
    }
}

/// 变更来源,镜像 reference `VALID_SOURCES`(6 种)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// K8s API watch。
    K8sApi,
    /// Argo CD 同步。
    ArgoCd,
    /// GitOps。
    Gitops,
    /// 人工触发(默认)。
    #[default]
    Manual,
    /// 未知。
    Unknown,
    /// Flagd 配置变更。
    Flagd,
}

impl Source {
    /// 由 snake_case 字符串解析;非法返 `None`。
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "k8s_api" => Some(Self::K8sApi),
            "argo_cd" => Some(Self::ArgoCd),
            "gitops" => Some(Self::Gitops),
            "manual" => Some(Self::Manual),
            "unknown" => Some(Self::Unknown),
            "flagd" => Some(Self::Flagd),
            _ => None,
        }
    }

    /// 默认值(`"manual"`)。
    pub fn default_name() -> &'static str {
        "manual"
    }
}

/// 严重度估算,镜像 reference `severity_estimate`(3 级)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// 低(0-4 受影响)。
    #[default]
    Low,
    /// 中(5-9 受影响,或过频)。
    Medium,
    /// 高(10+ 受影响)。
    High,
}

impl Severity {
    /// 由 snake_case 字符串解析。
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }

    /// snake_case 字符串。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// 变更事件(对齐 reference `ChangeEvent` dataclass,18 字段)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeEvent {
    /// `ce-<12 hex>`。
    pub change_event_id: String,
    /// 变更类型。
    pub change_type: ChangeType,
    /// 目标资源 ID。
    pub target_resource_id: String,
    /// 目标资源类型(目标不在拓扑时 `""`)。
    pub target_resource_type: String,
    /// 发生时间(ISO8601 `YYYY-MM-DDTHH:MM:SSZ`)。
    pub changed_at: String,
    /// 操作人(默认 `""`)。
    pub changed_by: String,
    /// 来源(默认 `manual`)。
    pub source: Source,
    /// 描述(默认 `""`)。
    pub description: String,
    /// 差异摘要(默认 `{}`)。
    pub diff_summary: serde_json::Value,
    /// 关联 commit(优先;若空则回填 `commit_sha`)。
    pub related_commit: String,
    /// 关联 PR。
    pub related_pr: String,
    /// 严重度估算。
    pub severity_estimate: Severity,
    /// 影响范围(resource_id 列表,不含目标自身)。
    pub propagated_to: Vec<String>,
    // --- Phase 2 字段 ---
    /// commit SHA(回填 `related_commit`)。
    pub commit_sha: String,
    /// 流水线 URL。
    pub pipeline_url: String,
    /// Git 仓库。
    pub git_repo: String,
    /// 集群 ID。
    pub cluster_id: String,
    /// YAML diff 文本。
    pub yaml_diff: String,
}

/// `record_change` 的入参(未校验;`record_change` 内部校验 `change_type`/`source`)。
///
/// 用结构体包住 14 个字段,避免 `too_many_arguments`(reference 是平铺 13 参的 Python fn)。
/// [`Default`] 对齐 reference 默认值:`source="manual"`、`diff_summary={}`。
#[derive(Debug, Clone)]
pub struct ChangeRequest {
    /// 变更类型字符串(校验 -> [`ChangeType`])。
    pub change_type: String,
    /// 目标资源 ID。
    pub target_resource_id: String,
    /// 操作人(默认 `""`)。
    pub changed_by: String,
    /// 来源字符串(默认 `"manual"`,校验 -> [`Source`])。
    pub source: String,
    /// 描述(默认 `""`)。
    pub description: String,
    /// 差异摘要(默认 `{}`)。
    pub diff_summary: serde_json::Value,
    /// 关联 commit。
    pub related_commit: String,
    /// 关联 PR。
    pub related_pr: String,
    /// 发生时间(`None` -> now)。
    pub changed_at: Option<String>,
    /// commit SHA。
    pub commit_sha: String,
    /// 流水线 URL。
    pub pipeline_url: String,
    /// Git 仓库。
    pub git_repo: String,
    /// 集群 ID。
    pub cluster_id: String,
    /// YAML diff 文本。
    pub yaml_diff: String,
}

impl Default for ChangeRequest {
    fn default() -> Self {
        Self {
            change_type: String::new(),
            target_resource_id: String::new(),
            changed_by: String::new(),
            source: Source::default_name().to_string(),
            description: String::new(),
            diff_summary: serde_json::json!({}),
            related_commit: String::new(),
            related_pr: String::new(),
            changed_at: None,
            commit_sha: String::new(),
            pipeline_url: String::new(),
            git_repo: String::new(),
            cluster_id: String::new(),
            yaml_diff: String::new(),
        }
    }
}

/// `ChangeRegistry::list` 过滤条件(对齐 reference `list_change_events` 参数)。
///
/// `since`/`until` 按 ISO8601 字符串字典序**闭区间**过滤(同格式同时区与时间序一致)。
#[derive(Debug, Clone, Default)]
pub struct ChangeFilter {
    /// 按变更类型过滤。
    pub change_type: Option<ChangeType>,
    /// 按目标资源 ID 过滤。
    pub target_resource_id: Option<String>,
    /// 按来源过滤。
    pub source: Option<Source>,
    /// `changed_at >= since`。
    pub since: Option<String>,
    /// `changed_at <= until`。
    pub until: Option<String>,
}

/// ChangeEvent 业务异常。`message` 直接给用户看;`code` 是 HTTP-like(400/404),
/// 3.6 Tauri command 据此映射返回。对齐 reference `ChangeEventError`。
#[derive(Debug, Clone)]
pub struct ChangeError {
    /// 人读消息。
    pub message: String,
    /// HTTP-like code(默认 400)。
    pub code: u16,
}

impl ChangeError {
    /// 新建(默认 code=400)。
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 400,
        }
    }

    /// 带 code 新建。
    pub fn with_code(message: impl Into<String>, code: u16) -> Self {
        Self {
            message: message.into(),
            code,
        }
    }
}

impl std::fmt::Display for ChangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for ChangeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_types_serialize_snake_case() {
        let s = serde_json::to_string(&ChangeType::ConfigmapUpdated).unwrap();
        assert_eq!(s, "\"configmap_updated\"");
        assert_eq!(ChangeType::ImagePushed.as_str(), "image_pushed");
    }

    #[test]
    fn change_type_from_name_roundtrip() {
        for name in ChangeType::ALL {
            let t = ChangeType::from_name(name).expect("valid");
            assert_eq!(t.as_str(), *name);
        }
        assert!(ChangeType::from_name("bogus").is_none());
    }

    #[test]
    fn source_default_is_manual() {
        assert_eq!(Source::default(), Source::Manual);
        assert_eq!(Source::default_name(), "manual");
        assert_eq!(Source::from_name("argo_cd"), Some(Source::ArgoCd));
        assert!(Source::from_name("weird").is_none());
    }

    #[test]
    fn severity_roundtrip() {
        assert_eq!(Severity::from_name("medium"), Some(Severity::Medium));
        assert_eq!(Severity::default(), Severity::Low);
        assert!(Severity::from_name("critical").is_none());
    }

    #[test]
    fn change_event_serializes_with_enums() {
        let ev = ChangeEvent {
            change_event_id: "ce-abc123".into(),
            change_type: ChangeType::SecretRotated,
            target_resource_id: "secret:db".into(),
            target_resource_type: "Secret".into(),
            changed_at: "2026-07-10T00:00:00Z".into(),
            changed_by: "alice".into(),
            source: Source::Manual,
            description: "rotate".into(),
            diff_summary: serde_json::json!({"version": {"old": 1, "new": 2}}),
            related_commit: "abc".into(),
            related_pr: "".into(),
            severity_estimate: Severity::Medium,
            propagated_to: vec!["pod:1".into()],
            commit_sha: "".into(),
            pipeline_url: "".into(),
            git_repo: "".into(),
            cluster_id: "".into(),
            yaml_diff: "".into(),
        };
        let v: serde_json::Value = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["change_type"], "secret_rotated");
        assert_eq!(v["source"], "manual");
        assert_eq!(v["severity_estimate"], "medium");
        assert_eq!(v["propagated_to"][0], "pod:1");
    }
}
