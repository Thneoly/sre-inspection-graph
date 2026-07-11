//! AlertEvent 模型 + 内存注册表(复刻 `reference/app/datasource/models.py` 的
//! `AlertEvent` dataclass + `store.list_alert_events`)。
//!
//! ## 与 reference 的差异
//!
//! - **强类型枚举**:reference `severity`/`status` 是 plain str(默认 `"critical"`/
//!   `"firing"`);本 port 用 [`AlertSeverity`]/[`AlertStatus`] 枚举(snake_case 序列化一致)。
//! - **内存 registry**:reference AlertEvent 落 DSS `store` + Neo4j dual-write;本 port
//!   只内存 [`AlertRegistry`](SQLite 持久化 3.6 接),**丢 Neo4j**。`correlate_alerts` 从
//!   此 registry 读(替代 reference 的 DSS+Neo4j 双源合并)。

#![allow(missing_docs)]

use serde::{Deserialize, Serialize};

/// 告警严重度(对齐 reference `severity`,`"warning" | "critical"`,默认 critical)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertSeverity {
    /// 警告。
    Warning,
    /// 严重(默认)。
    #[default]
    Critical,
}

/// 告警状态(对齐 reference `status`,`"firing" | "resolved"`,默认 firing)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertStatus {
    /// 触发中(默认)。
    #[default]
    Firing,
    /// 已恢复。
    Resolved,
}

/// 告警事件(对齐 reference `AlertEvent` dataclass,13 字段)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlertEvent {
    /// 告警 ID。
    pub alert_event_id: String,
    /// 规则名 / 告警名。
    pub alert_name: String,
    /// 严重度(默认 critical)。
    pub severity: AlertSeverity,
    /// 状态(默认 firing)。
    pub status: AlertStatus,
    /// 触发时间(ISO8601;record_alert 填)。
    pub fired_at: String,
    /// 被告警资源 DSS node_id。
    pub resource_ref: String,
    /// 触发的 AlertRule。
    pub rule_id: String,
    /// 触发指标。
    pub metric_name: String,
    /// 触发时的值。
    pub metric_value: f64,
    /// 摘要。
    pub summary: String,
    /// 描述。
    pub description: String,
    /// 集群 ID。
    pub cluster_id: String,
    /// 恢复时间(ISO8601)。
    pub resolved_at: String,
}

impl AlertEvent {
    /// 新建(默认 severity=critical / status=firing,其余空;对齐 dataclass 默认值)。
    pub fn new(alert_event_id: impl Into<String>, alert_name: impl Into<String>) -> Self {
        Self {
            alert_event_id: alert_event_id.into(),
            alert_name: alert_name.into(),
            severity: AlertSeverity::Critical,
            status: AlertStatus::Firing,
            fired_at: String::new(),
            resource_ref: String::new(),
            rule_id: String::new(),
            metric_name: String::new(),
            metric_value: 0.0,
            summary: String::new(),
            description: String::new(),
            cluster_id: String::new(),
            resolved_at: String::new(),
        }
    }
}

/// 内存 AlertEvent 注册表(对齐 reference DSS `store` 的 alert 部分)。
#[derive(Debug, Clone, Default)]
pub struct AlertRegistry {
    /// 插入序告警列表。
    alerts: Vec<AlertEvent>,
}

impl AlertRegistry {
    /// 新建空注册表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加一个告警。
    pub fn add(&mut self, alert: AlertEvent) {
        self.alerts.push(alert);
    }

    /// 按 ID 取告警。
    pub fn get(&self, alert_event_id: &str) -> Option<&AlertEvent> {
        self.alerts.iter().find(|a| a.alert_event_id == alert_event_id)
    }

    /// 按 `fired_at` 时间窗列出告警(ISO8601 字典序闭区间,对齐 reference
    /// `store.list_alert_events(since, until)`)。
    pub fn list(&self, since: Option<&str>, until: Option<&str>) -> Vec<&AlertEvent> {
        self.alerts
            .iter()
            .filter(|a| match since {
                Some(s) => !a.fired_at.is_empty() && a.fired_at.as_str() >= s,
                None => true,
            })
            .filter(|a| match until {
                Some(u) => !a.fired_at.is_empty() && a.fired_at.as_str() <= u,
                None => true,
            })
            .collect()
    }

    /// 清空。
    pub fn clear(&mut self) {
        self.alerts.clear();
    }

    /// 告警数。
    pub fn len(&self) -> usize {
        self.alerts.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.alerts.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert(id: &str, ref_: &str, fired_at: &str) -> AlertEvent {
        let mut a = AlertEvent::new(id, "HighCPU");
        a.resource_ref = ref_.into();
        a.fired_at = fired_at.into();
        a
    }

    #[test]
    fn defaults_critical_firing() {
        let a = AlertEvent::new("a1", "name");
        assert_eq!(a.severity, AlertSeverity::Critical);
        assert_eq!(a.status, AlertStatus::Firing);
        assert_eq!(a.metric_value, 0.0);
    }

    #[test]
    fn registry_add_get_list() {
        let mut reg = AlertRegistry::new();
        reg.add(alert("a1", "pod:1", "2026-07-10T03:00:00Z"));
        reg.add(alert("a2", "pod:2", "2026-07-10T05:00:00Z"));
        assert!(reg.get("a1").is_some());
        assert!(reg.get("nope").is_none());

        let in_window = reg.list(Some("2026-07-10T04:00:00Z"), Some("2026-07-10T06:00:00Z"));
        assert_eq!(in_window.len(), 1);
        assert_eq!(in_window[0].alert_event_id, "a2");
    }

    #[test]
    fn registry_list_excludes_empty_fired_at() {
        let mut reg = AlertRegistry::new();
        let mut a = AlertEvent::new("a1", "n");
        a.resource_ref = "pod:1".into();
        // fired_at 空 -> 不进时间窗过滤结果
        reg.add(a);
        assert_eq!(reg.list(Some("2000-01-01T00:00:00Z"), Some("2999-01-01T00:00:00Z")).len(), 0);
    }

    #[test]
    fn alert_serializes_snake_case() {
        let a = AlertEvent::new("a1", "HighCPU");
        let v = serde_json::to_value(&a).unwrap();
        assert_eq!(v["severity"], "critical");
        assert_eq!(v["status"], "firing");
    }
}
