//! 变更频率告警(复刻 `reference/app/changes/frequency.py` + `event_service._apply_frequency_check`)。
//!
//! 检测"过频变更":同一资源在指定时间窗内变更次数超阈值。`record_change` 写入后调
//! [`apply_frequency_check`],命中则把 severity 至少提到 medium + description 追加
//! 「[过频变更]」标记。
//!
//! ## 与 reference 的差异
//!
//! - **I/O-free**:reference 读全局 DSS `store.list_change_events`;本 port 接
//!   `&ChangeRegistry`(纯领域)。3.4 时 `record_change` 不调频率检查,3.5 接入。
//! - **命中判定严格 `>`**:`count > threshold`(等于阈值不算过频),逐字对齐。
//! - **只升 low->medium**:reference `_apply_frequency_check` 仅 `severity == "low"` 时提
//!   medium,已 medium/high 不动但仍追加 tag。逐字对齐。

#![allow(missing_docs)]

use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;

use crate::event_service::ChangeRegistry;
use crate::iso::{now_iso, parse_iso_utc};
use crate::models::{ChangeEvent, ChangeFilter, Severity};

/// 默认时间窗 1h(对齐 reference `DEFAULT_WINDOW_SECONDS`)。
pub const DEFAULT_WINDOW_SECONDS: i64 = 3600;
/// 默认阈值 5(对齐 reference `DEFAULT_THRESHOLD`)。
pub const DEFAULT_THRESHOLD: usize = 5;

/// 单资源频率检查结果(对齐 reference `check_target_frequency` 返回)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrequencyResult {
    /// 是否过频(`count > threshold`)。
    pub is_frequent: bool,
    /// 窗口内变更数。
    pub count: usize,
    /// 时间窗(秒)。
    pub window_seconds: i64,
    /// 阈值。
    pub threshold: usize,
    /// 窗口内事件 ID(按 changed_at 倒序)。
    pub event_ids: Vec<String>,
}

/// 过频变更分桶(对齐 reference `detect_frequent_changes` 返回项)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrequentTarget {
    /// 目标资源 ID。
    pub target_resource_id: String,
    /// 窗口内变更数。
    pub count: usize,
    /// 窗口起点(ISO8601)。
    pub window_start: String,
    /// 窗口终点(ISO8601)。
    pub window_end: String,
    /// 阈值。
    pub threshold: usize,
    /// 事件 ID(按 changed_at 倒序)。
    pub event_ids: Vec<String>,
}

/// 检查单个资源在最近 window 内的变更频次(对齐 reference `check_target_frequency`)。
///
/// `is_frequent = count > threshold`(严格 `>`)。`event_ids` 按 `changed_at` 倒序。
pub fn check_target_frequency(
    registry: &ChangeRegistry,
    target_resource_id: &str,
    window_seconds: i64,
    threshold: usize,
) -> FrequencyResult {
    let now = Utc::now();
    let win_start = now - Duration::seconds(window_seconds);

    let mut recent: Vec<&ChangeEvent> = registry
        .list(&ChangeFilter {
            target_resource_id: Some(target_resource_id.to_string()),
            ..Default::default()
        })
        .into_iter()
        .filter(|e| parse_iso_utc(&e.changed_at).map(|dt| dt >= win_start).unwrap_or(false))
        .collect();
    recent.sort_by_key(|e| Reverse(e.changed_at.as_str()));

    let count = recent.len();
    FrequencyResult {
        is_frequent: count > threshold,
        count,
        window_seconds,
        threshold,
        event_ids: recent.iter().map(|e| e.change_event_id.clone()).collect(),
    }
}

/// 扫所有 ChangeEvent,按 target 分桶,返回过频变更列表(对齐 reference
/// `detect_frequent_changes`)。按 count 倒序;空列表表示无过频。
pub fn detect_frequent_changes(
    registry: &ChangeRegistry,
    window_seconds: i64,
    threshold: usize,
) -> Vec<FrequentTarget> {
    let now = Utc::now();
    let win_start = now - Duration::seconds(window_seconds);
    let win_start_iso = win_start.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let win_end_iso = now_iso();

    // 按目标分桶(只收窗口内的)
    let mut buckets: std::collections::BTreeMap<String, Vec<&ChangeEvent>> = std::collections::BTreeMap::new();
    for ev in registry.list(&ChangeFilter::default()) {
        if parse_iso_utc(&ev.changed_at).map(|dt| dt >= win_start).unwrap_or(false) {
            buckets.entry(ev.target_resource_id.clone()).or_default().push(ev);
        }
    }

    let mut frequent: Vec<FrequentTarget> = buckets
        .into_iter()
        .filter_map(|(target, mut evs)| {
            if evs.len() <= threshold {
                return None;
            }
            evs.sort_by_key(|e| Reverse(e.changed_at.as_str()));
            Some(FrequentTarget {
                target_resource_id: target,
                count: evs.len(),
                window_start: win_start_iso.clone(),
                window_end: win_end_iso.clone(),
                threshold,
                event_ids: evs.iter().map(|e| e.change_event_id.clone()).collect(),
            })
        })
        .collect();
    frequent.sort_by_key(|f| Reverse(f.count));
    frequent
}

/// 过频变更检测 -- 命中则把 severity 至少提到 medium,description 追加标记
/// (对齐 reference `_apply_frequency_check`)。
///
/// 幂等:只升不降(仅 `low` -> `medium`,已 medium/high 不动);tag 已存在不重复追加。
/// 用默认窗 [`DEFAULT_WINDOW_SECONDS`] + 阈值 [`DEFAULT_THRESHOLD`]。
///
/// **调用时机**:在事件**已入 registry 后**调(计数含当前事件,对齐 reference
/// `store.add_change_event` 后调 `_apply_frequency_check`)。
pub fn apply_frequency_check(event: &mut ChangeEvent, registry: &ChangeRegistry) {
    let result = check_target_frequency(
        registry,
        &event.target_resource_id,
        DEFAULT_WINDOW_SECONDS,
        DEFAULT_THRESHOLD,
    );
    if result.is_frequent {
        if event.severity_estimate == Severity::Low {
            event.severity_estimate = Severity::Medium;
        }
        if !event.description.contains("[过频变更]") {
            let tag = format!("[过频变更:{}次/{}s]", result.count, result.window_seconds);
            let new_desc = format!("{} {}", event.description, tag);
            event.description = new_desc.trim().to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ChangeRequest, Severity};
    use crate::propagation::tests::fixture_topology_phase2;
    use crate::record_change;

    fn req(ctype: &str, target: &str) -> ChangeRequest {
        ChangeRequest {
            change_type: ctype.into(),
            target_resource_id: target.into(),
            source: "k8s_api".into(),
            ..Default::default()
        }
    }

    #[test]
    fn frequent_change_elevates_or_tags() {
        // cm:order-config propagated_to=5(pod1/pod2/deploy/comp/app)-> 基础 medium。
        // 连续记 6 次(默认阈值 5,>5 命中)-> 末条 description 带 [过频变更] 标记。
        let topo = fixture_topology_phase2();
        let mut reg = ChangeRegistry::new();
        for _ in 0..6 {
            record_change(&mut reg, &topo, &req("configmap_updated", "cm:order-config")).unwrap();
        }
        let cm_events = reg.list(&ChangeFilter {
            target_resource_id: Some("cm:order-config".into()),
            ..Default::default()
        });
        let last = cm_events.last().unwrap();
        assert_eq!(last.severity_estimate, Severity::Medium);
        assert!(last.description.contains("过频变更"), "desc: {}", last.description);
    }

    #[test]
    fn below_threshold_keeps_severity() {
        // 3 次(< 5,不命中)。cm 基础 medium(propagated=5),频率不触发也不提升。
        let topo = fixture_topology_phase2();
        let mut reg = ChangeRegistry::new();
        for _ in 0..3 {
            record_change(&mut reg, &topo, &req("configmap_updated", "cm:order-config")).unwrap();
        }
        let cm_events = reg.list(&ChangeFilter {
            target_resource_id: Some("cm:order-config".into()),
            ..Default::default()
        });
        assert!(cm_events.iter().all(|e| !e.description.contains("过频变更")));
        assert!(cm_events.iter().all(|e| e.severity_estimate == Severity::Medium));
    }

    #[test]
    fn detect_frequent_changes_buckets_by_target() {
        let topo = fixture_topology_phase2();
        let mut reg = ChangeRegistry::new();
        // 6× cm(过频)+ 2× secret(不过频)
        for _ in 0..6 {
            record_change(&mut reg, &topo, &req("configmap_updated", "cm:order-config")).unwrap();
        }
        for _ in 0..2 {
            record_change(&mut reg, &topo, &req("secret_rotated", "secret:order-db")).unwrap();
        }
        let freq = detect_frequent_changes(&reg, 3600, 5);
        let cm = freq.iter().find(|f| f.target_resource_id == "cm:order-config");
        assert!(cm.is_some(), "cm should be frequent");
        let cm = cm.unwrap();
        assert_eq!(cm.count, 6);
        assert_eq!(cm.event_ids.len(), 6);
        assert!(freq.iter().all(|f| f.target_resource_id != "secret:order-db"));
    }

    #[test]
    fn frequency_elevates_low_to_medium_for_low_base_target() {
        // img:order:1.2.3 无边 -> propagated=0 -> 基础 low。6 次 -> 频率提 low->medium + tag。
        let topo = fixture_topology_phase2();
        let mut reg = ChangeRegistry::new();
        for _ in 0..6 {
            record_change(&mut reg, &topo, &req("image_pushed", "img:order:1.2.3")).unwrap();
        }
        let img_events = reg.list(&ChangeFilter {
            target_resource_id: Some("img:order:1.2.3".into()),
            ..Default::default()
        });
        let last = img_events.last().unwrap();
        assert_eq!(last.severity_estimate, Severity::Medium); // low -> medium
        assert!(last.description.contains("过频变更"));
    }

    #[test]
    fn check_target_frequency_strict_greater() {
        // count == threshold(5)不算过频(严格 >)
        let topo = fixture_topology_phase2();
        let mut reg = ChangeRegistry::new();
        for _ in 0..5 {
            record_change(&mut reg, &topo, &req("configmap_updated", "cm:order-config")).unwrap();
        }
        let r = check_target_frequency(&reg, "cm:order-config", 3600, 5);
        assert_eq!(r.count, 5);
        assert!(!r.is_frequent); // 5 > 5 is false
    }
}
