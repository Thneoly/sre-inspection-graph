//! engine-changes
//!
//! PRD-002 复刻 — ChangeEvent + propagation BFS + YAML diff + frequency alert
//! + AlertEvent correlation。Phase 1 占位,Phase 3 起复刻。

#![deny(unsafe_code)]
#![warn(missing_docs)]

/// 变更事件类型,镜像 reference 的 4 种 + 后续扩展。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

/// Crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_types_serialize_snake_case() {
        let s = serde_json::to_string(&ChangeType::ConfigmapUpdated).unwrap();
        assert_eq!(s, "\"configmap_updated\"");
    }
}
