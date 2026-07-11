//! ChangeEvent ↔ AlertEvent 关联(复刻 `reference/app/changes/alert_correlation.py`)。
//!
//! - [`correlate_alerts`]:给定变更,找窗口内 `resource_ref` 落在影响面
//!   ({target} ∪ propagated_to)的 AlertEvent。
//! - [`correlate_changes_for_alert`]:反向,给定 AlertEvent 找窗口内对其 resource 关联的 ChangeEvent。
//!
//! ## 与 reference 的差异
//!
//! - **丢 Neo4j**:reference `_fetch_alerts_in_window` 双源(DSS + Neo4j legacy)合并去重 +
//!   `persist_correlation` 写 `CORRELATED_WITH` 边;本 port **只从 [`AlertRegistry`] 读**
//!   (DSS 等价),`neo4j_available` 恒 `false`,丢 `persist_correlation`/`correlate_and_persist`。
//! - **`correlate_changes_for_alert` 空 ref 分支**:reference 查 Neo4j 取 alert.resource_ref;
//!   本 port 查 [`AlertRegistry::get`],查无则返空(对齐"不可解析 ref -> 空返"语义)。

#![allow(missing_docs)]

use serde::{Deserialize, Serialize};
use std::cmp::Reverse;

use crate::alerts::{AlertEvent, AlertRegistry};
use crate::event_service::ChangeRegistry;
use crate::iso::{now_iso, parse_iso_utc, shift_iso};
use crate::models::{ChangeError, ChangeFilter, ChangeType, Severity, Source};

/// `correlate_alerts` 默认窗口 600s(对齐 reference `DEFAULT_WINDOW_SECONDS`)。
pub const DEFAULT_ALERT_WINDOW_SECONDS: i64 = 600;
/// `correlate_changes_for_alert` 默认窗口 300s。
pub const DEFAULT_CHANGE_WINDOW_SECONDS: i64 = 300;

/// `correlate_alerts` 返回(对齐 reference)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrelateAlertsResult {
    /// 变更事件 ID。
    pub change_event_id: String,
    /// 变更发生时间。
    pub changed_at: String,
    /// 关联窗口起点。
    pub window_start: String,
    /// 关联窗口终点。
    pub window_end: String,
    /// 影响资源 ID(target ∪ propagated_to,排序)。
    pub affected_resource_ids: Vec<String>,
    /// 命中的告警。
    pub alerts: Vec<AlertEvent>,
    /// 命中数。
    pub total: usize,
    /// Neo4j 是否可用(本 port 恒 false)。
    pub neo4j_available: bool,
}

/// `correlate_changes_for_alert` 单条命中(对齐 reference dict)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrelatedChangeForAlert {
    pub change_event_id: String,
    pub change_type: ChangeType,
    pub target_resource_id: String,
    pub changed_at: String,
    pub source: Source,
    pub severity_estimate: Severity,
}

/// `correlate_changes_for_alert` 返回(对齐 reference)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrelateChangesForAlertResult {
    pub alert_event_id: String,
    pub resource_ref: String,
    pub fired_at: String,
    pub changes: Vec<CorrelatedChangeForAlert>,
    pub total: usize,
    pub neo4j_available: bool,
}

/// 给定变更事件,找窗口内资源关联的 AlertEvent(对齐 reference `correlate_alerts`)。
///
/// 关联判定:`AlertEvent.resource_ref ∈ {变更 target} ∪ 变更 propagated_to`。
/// 时间窗:`[changed_at - window, changed_at + window]`。
pub fn correlate_alerts(
    registry: &ChangeRegistry,
    alerts: &AlertRegistry,
    change_event_id: &str,
    window_seconds: i64,
) -> Result<CorrelateAlertsResult, ChangeError> {
    let event = registry
        .get(change_event_id)
        .ok_or_else(|| ChangeError::with_code(format!("change_event not found: {change_event_id}"), 404))?;

    let mut affected: Vec<String> = std::iter::once(event.target_resource_id.clone())
        .chain(event.propagated_to.iter().cloned())
        .collect();
    affected.sort();

    let (win_start, win_end) = match parse_iso_utc(&event.changed_at) {
        Some(_) => (shift_iso(&event.changed_at, -window_seconds), shift_iso(&event.changed_at, window_seconds)),
        None => {
            let now = now_iso();
            (now.clone(), now)
        }
    };

    let affected_set: std::collections::HashSet<&str> = affected.iter().map(|s| s.as_str()).collect();
    let matched: Vec<AlertEvent> = alerts
        .list(Some(&win_start), Some(&win_end))
        .into_iter()
        .filter(|a| affected_set.contains(a.resource_ref.as_str()))
        .cloned()
        .collect();
    let total = matched.len();

    Ok(CorrelateAlertsResult {
        change_event_id: change_event_id.to_string(),
        changed_at: event.changed_at.clone(),
        window_start: win_start,
        window_end: win_end,
        affected_resource_ids: affected,
        alerts: matched,
        total,
        neo4j_available: false,
    })
}

/// 反向:给定 AlertEvent,找窗口内对其 resource 关联的 ChangeEvent
/// (对齐 reference `correlate_changes_for_alert`)。
///
/// 关联:`ChangeEvent.target_resource_id == resource_ref` 或 `resource_ref ∈ propagated_to`。
/// 时间窗:`[fired_at - window, fired_at + window]`(无 fired_at 则不限,全收命中)。
/// `resource_ref` 为空时从 [`AlertRegistry`] 查 alert 取 ref;查无 -> 空返。
pub fn correlate_changes_for_alert(
    registry: &ChangeRegistry,
    alerts: &AlertRegistry,
    alert_event_id: &str,
    window_seconds: i64,
    resource_ref: &str,
) -> CorrelateChangesForAlertResult {
    let (ref_id, fired_at) = if resource_ref.is_empty() {
        match alerts.get(alert_event_id) {
            Some(a) => (a.resource_ref.clone(), a.fired_at.clone()),
            None => {
                return CorrelateChangesForAlertResult {
                    alert_event_id: alert_event_id.to_string(),
                    resource_ref: String::new(),
                    fired_at: String::new(),
                    changes: Vec::new(),
                    total: 0,
                    neo4j_available: false,
                };
            }
        }
    } else {
        (resource_ref.to_string(), alerts.get(alert_event_id).map(|a| a.fired_at.clone()).unwrap_or_default())
    };

    if ref_id.is_empty() {
        return CorrelateChangesForAlertResult {
            alert_event_id: alert_event_id.to_string(),
            resource_ref: String::new(),
            fired_at,
            changes: Vec::new(),
            total: 0,
            neo4j_available: false,
        };
    }

    let fired_dt = parse_iso_utc(&fired_at);
    let mut matched: Vec<CorrelatedChangeForAlert> = registry
        .list(&ChangeFilter::default())
        .into_iter()
        .filter(|ev| ev.target_resource_id == ref_id || ev.propagated_to.contains(&ref_id))
        .filter(|ev| match fired_dt {
            Some(fd) => match parse_iso_utc(&ev.changed_at) {
                Some(ed) => (ed - fd).num_seconds().abs() <= window_seconds,
                None => false,
            },
            None => true, // 无 fired_at -> 不限时间
        })
        .map(|ev| CorrelatedChangeForAlert {
            change_event_id: ev.change_event_id.clone(),
            change_type: ev.change_type,
            target_resource_id: ev.target_resource_id.clone(),
            changed_at: ev.changed_at.clone(),
            source: ev.source,
            severity_estimate: ev.severity_estimate,
        })
        .collect();
    matched.sort_by_key(|c| Reverse(c.changed_at.clone()));

    let total = matched.len();
    CorrelateChangesForAlertResult {
        alert_event_id: alert_event_id.to_string(),
        resource_ref: ref_id,
        fired_at,
        changes: matched,
        total,
        neo4j_available: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alerts::{AlertEvent, AlertRegistry};
    use crate::event_service::ChangeRegistry;
    use crate::models::ChangeRequest;
    use crate::propagation::tests::fixture_topology_phase2;
    use crate::record_change;

    fn record_cm(reg: &mut ChangeRegistry) -> String {
        let ev = record_change(
            reg,
            &fixture_topology_phase2(),
            &ChangeRequest {
                change_type: "configmap_updated".into(),
                target_resource_id: "cm:order-config".into(),
                source: "k8s_api".into(),
                ..Default::default()
            },
        )
        .unwrap();
        ev.change_event_id
    }

    #[test]
    fn correlate_alerts_matches_resource_in_propagated_to() {
        let mut reg = ChangeRegistry::new();
        let ev_id = record_cm(&mut reg);
        let changed_at = reg.get(&ev_id).unwrap().changed_at.clone();
        // cm propagated_to 含 pod:order-api-1
        assert!(reg.get(&ev_id).unwrap().propagated_to.contains(&"pod:order-api-1".to_string()));

        let mut alerts = AlertRegistry::new();
        let mut a = AlertEvent::new("fault_alert_1", "PodCrashloop");
        a.resource_ref = "pod:order-api-1".into();
        a.fired_at = changed_at; // 窗口内
        alerts.add(a);

        let result = correlate_alerts(&reg, &alerts, &ev_id, 600).unwrap();
        assert_eq!(result.total, 1);
        assert_eq!(result.alerts[0].alert_event_id, "fault_alert_1");
        assert!(!result.neo4j_available);
    }

    #[test]
    fn correlate_alerts_no_match_when_resource_outside_impact() {
        let mut reg = ChangeRegistry::new();
        let ev_id = record_cm(&mut reg);
        let changed_at = reg.get(&ev_id).unwrap().changed_at.clone();

        let mut alerts = AlertRegistry::new();
        let mut a = AlertEvent::new("alert_other", "X");
        a.resource_ref = "pod:unrelated-9".into(); // 不在影响面
        a.fired_at = changed_at;
        alerts.add(a);

        let result = correlate_alerts(&reg, &alerts, &ev_id, 600).unwrap();
        assert_eq!(result.total, 0);
    }

    #[test]
    fn correlate_alerts_empty_when_no_alerts() {
        let mut reg = ChangeRegistry::new();
        let ev_id = record_cm(&mut reg);
        let alerts = AlertRegistry::new(); // 无告警
        let result = correlate_alerts(&reg, &alerts, &ev_id, 600).unwrap();
        assert_eq!(result.total, 0);
        assert!(!result.neo4j_available);
    }

    #[test]
    fn correlate_alerts_404_unknown_event() {
        let reg = ChangeRegistry::new();
        let alerts = AlertRegistry::new();
        let err = correlate_alerts(&reg, &alerts, "ce-nope", 600).unwrap_err();
        assert_eq!(err.code, 404);
    }

    #[test]
    fn correlate_alerts_window_excludes_old_alert() {
        let mut reg = ChangeRegistry::new();
        let ev_id = record_cm(&mut reg);
        let changed_at = reg.get(&ev_id).unwrap().changed_at.clone();

        let mut alerts = AlertRegistry::new();
        let mut a = AlertEvent::new("old_alert", "X");
        a.resource_ref = "pod:order-api-1".into();
        // fired_at 远早于 changed_at(> 600s 窗口)
        a.fired_at = crate::iso::shift_iso(&changed_at, -3600);
        alerts.add(a);

        let result = correlate_alerts(&reg, &alerts, &ev_id, 600).unwrap();
        assert_eq!(result.total, 0);
    }

    #[test]
    fn correlate_changes_for_alert_matches_change_target() {
        // 反向:alert.resource_ref == change.target -> 命中
        let mut reg = ChangeRegistry::new();
        let ev_id = record_cm(&mut reg);
        let changed_at = reg.get(&ev_id).unwrap().changed_at.clone();

        let mut alerts = AlertRegistry::new();
        let mut a = AlertEvent::new("alert-1", "X");
        a.resource_ref = "cm:order-config".into();
        a.fired_at = changed_at;
        alerts.add(a);

        let result = correlate_changes_for_alert(&reg, &alerts, "alert-1", 300, "");
        assert_eq!(result.resource_ref, "cm:order-config");
        assert_eq!(result.total, 1);
        assert_eq!(result.changes[0].change_event_id, ev_id);
    }

    #[test]
    fn correlate_changes_for_alert_empty_when_alert_missing() {
        let reg = ChangeRegistry::new();
        let alerts = AlertRegistry::new();
        let result = correlate_changes_for_alert(&reg, &alerts, "nope", 300, "");
        assert_eq!(result.total, 0);
        assert_eq!(result.resource_ref, "");
    }
}
