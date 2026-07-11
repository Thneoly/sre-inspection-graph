//! ChangeEvent 业务编排(复刻 `reference/app/changes/event_service.py` 的 `record_change`
//! + DSS store CRUD + `_estimate_severity`)。
//!
//! ## 与 reference 的差异
//!
//! - **I/O-free 注册表**:reference 写全局 DSS `store` + best-effort Neo4j dual-write +
//!   alert 关联;本 port 只写内存 [`ChangeRegistry`](对齐 DSS 主存),**丢弃 Neo4j dual-write
//!   与 alert 关联**(best-effort 副本,SQLite-only 架构无需;alert 关联是 3.5 scope)。
//! - **频率提升留 3.5**:reference `record_change` 末尾调 `_apply_frequency_check`(可能
//!   low->medium + `[过频变更]` tag);本 port **不调**(frequency 是 3.5 scope),severity
//!   仅由 `_estimate_severity(propagated_count)` 决定。偏差已在 commit msg 明示。
//! - **入参结构体**:reference `record_change` 平铺 13 参(默认值);本 port 用 [`ChangeRequest`]
//!   包住(避免 `too_many_arguments`),[`ChangeRequest::default`] 复刻默认值。
//! - **拓扑入参**:reference `record_change` 内部读全局 `store.get_node` / `derive_propagation`;
//!   本 port 显式接 `&Topology`(纯领域,orchestration 层喂 materialized topology)。

#![allow(missing_docs)]

use engine_identity::Topology;
use uuid::Uuid;

use crate::models::{ChangeError, ChangeEvent, ChangeFilter, ChangeRequest, ChangeType, Severity, Source};
use crate::propagation::{derive_propagation, DEFAULT_PROPAGATION_DEPTH};

/// 内存 ChangeEvent 注册表(对齐 reference DSS `store.change_events` - 主存,插入序)。
///
/// SQLite 持久化在 3.6 Tauri 管线接;本 registry 是 domain 层的内存形态。
#[derive(Debug, Clone, Default)]
pub struct ChangeRegistry {
    /// 插入序事件列表(对齐 Python dict 的插入序语义)。
    events: Vec<ChangeEvent>,
}

impl ChangeRegistry {
    /// 新建空注册表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加一个事件(对齐 `store.add_change_event`)。
    pub fn add(&mut self, event: ChangeEvent) {
        self.events.push(event);
    }

    /// 按 ID 取事件(对齐 `store.get_change_event`)。
    pub fn get(&self, change_event_id: &str) -> Option<&ChangeEvent> {
        self.events.iter().find(|e| e.change_event_id == change_event_id)
    }

    /// 按过滤条件列出事件(对齐 `store.list_change_events`)。
    ///
    /// 返回插入序的借用切片;`since`/`until` 按 ISO8601 字符串字典序**闭区间**过滤。
    pub fn list(&self, filter: &ChangeFilter) -> Vec<&ChangeEvent> {
        self.events
            .iter()
            .filter(|e| match filter.change_type {
                Some(t) => e.change_type == t,
                None => true,
            })
            .filter(|e| match &filter.target_resource_id {
                Some(t) => &e.target_resource_id == t,
                None => true,
            })
            .filter(|e| match filter.source {
                Some(s) => e.source == s,
                None => true,
            })
            .filter(|e| match &filter.since {
                Some(s) => e.changed_at.as_str() >= s.as_str(),
                None => true,
            })
            .filter(|e| match &filter.until {
                Some(u) => e.changed_at.as_str() <= u.as_str(),
                None => true,
            })
            .collect()
    }

    /// 清空(对齐 `store.clear_change_events`)。
    pub fn clear(&mut self) {
        self.events.clear();
    }

    /// 事件数。
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// 估算严重度(对齐 reference `_estimate_severity`;`frequent` 参数在 reference 是 dead code,
/// 此处不接 -- 频率提升是 3.5 scope)。
///
/// - `>= 10` -> [`Severity::High`]
/// - `>= 5`  -> [`Severity::Medium`]
/// - else    -> [`Severity::Low`]
pub fn estimate_severity(propagated_count: usize) -> Severity {
    if propagated_count >= 10 {
        Severity::High
    } else if propagated_count >= 5 {
        Severity::Medium
    } else {
        Severity::Low
    }
}

/// 记录一个变更事件(对齐 reference `record_change`)。
///
/// 流程:校验 `change_type`/`source`(非法 -> [`ChangeError`] 400)-> 生成 `ce-<12 hex>` ID
/// -> 从拓扑查 `target_resource_type`(不在拓扑则 `""`)-> 反向 BFS 算 `propagated_to`
/// (目标不在拓扑则 `[]`)-> `estimate_severity(propagated.len())` -> `commit_sha` 回填
/// `related_commit`(related_commit 优先)-> 入注册表 -> 返回事件。
///
/// **偏差**:不调 `_apply_frequency_check`(3.5);不写 Neo4j / 不做 alert 关联(丢弃)。
pub fn record_change(
    registry: &mut ChangeRegistry,
    topology: &Topology,
    req: &ChangeRequest,
) -> Result<ChangeEvent, ChangeError> {
    let change_type = ChangeType::from_name(&req.change_type).ok_or_else(|| {
        ChangeError::new(format!(
            "invalid change_type: '{}' (valid: {:?})",
            req.change_type, ChangeType::ALL
        ))
    })?;
    let source = Source::from_name(&req.source)
        .ok_or_else(|| ChangeError::new(format!("invalid source: '{}'", req.source)))?;

    // target_resource_type:目标不在拓扑 -> ""(仍记录,watcher 可能先于 node sync 推)
    let target_node = topology.nodes.iter().find(|n| n.resource_id == req.target_resource_id);
    let target_resource_type = target_node.map(|n| n.resource_type.clone()).unwrap_or_default();

    // propagated_to:目标不在拓扑 -> [](derive_propagation 自身也守这条,这里显式短路对齐 reference)
    let propagated_to = if target_node.is_some() {
        derive_propagation(&req.target_resource_id, topology, DEFAULT_PROPAGATION_DEPTH, None)
    } else {
        Vec::new()
    };

    let severity_estimate = estimate_severity(propagated_to.len());
    let changed_at = req.changed_at.clone().unwrap_or_else(now_iso);

    // effective_commit = related_commit or commit_sha(related_commit 优先;commit_sha 回填)
    let related_commit = if !req.related_commit.is_empty() {
        req.related_commit.clone()
    } else {
        req.commit_sha.clone()
    };

    let event = ChangeEvent {
        change_event_id: new_change_id(),
        change_type,
        target_resource_id: req.target_resource_id.clone(),
        target_resource_type,
        changed_at,
        changed_by: req.changed_by.clone(),
        source,
        description: req.description.clone(),
        diff_summary: req.diff_summary.clone(),
        related_commit,
        related_pr: req.related_pr.clone(),
        severity_estimate,
        propagated_to,
        commit_sha: req.commit_sha.clone(),
        pipeline_url: req.pipeline_url.clone(),
        git_repo: req.git_repo.clone(),
        cluster_id: req.cluster_id.clone(),
        yaml_diff: req.yaml_diff.clone(),
    };
    registry.add(event.clone());
    Ok(event)
}

/// 生成 `ce-<12 hex>` ID(对齐 reference `f"ce-{uuid.uuid4().hex[:12]}"`)。
fn new_change_id() -> String {
    let hex = Uuid::new_v4().simple().to_string();
    format!("ce-{}", &hex[..12])
}

/// 当前 UTC 时间 ISO8601 `YYYY-MM-DDTHH:MM:SSZ`(对齐 reference `_now_iso`)。
fn now_iso() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ChangeFilter, ChangeRequest, ChangeType, Severity, Source};
    use crate::propagation::tests::fixture_topology;
    use serde_json::json;

    // ===== dataclass 默认值 =====

    #[test]
    fn estimate_severity_boundaries() {
        // 0-4 low, 5-9 medium, 10+ high
        assert_eq!(estimate_severity(0), Severity::Low);
        assert_eq!(estimate_severity(4), Severity::Low);
        assert_eq!(estimate_severity(5), Severity::Medium);
        assert_eq!(estimate_severity(9), Severity::Medium);
        assert_eq!(estimate_severity(10), Severity::High);
        assert_eq!(estimate_severity(50), Severity::High);
    }

    // ===== ChangeRegistry CRUD(对齐 TestStore)=====

    fn make_event(id: &str, ctype: &str, target: &str, ttype: &str, changed_at: &str) -> ChangeEvent {
        ChangeEvent {
            change_event_id: id.into(),
            change_type: ChangeType::from_name(ctype).unwrap(),
            target_resource_id: target.into(),
            target_resource_type: ttype.into(),
            changed_at: changed_at.into(),
            changed_by: String::new(),
            source: Source::Manual,
            description: String::new(),
            diff_summary: json!({}),
            related_commit: String::new(),
            related_pr: String::new(),
            severity_estimate: Severity::Low,
            propagated_to: Vec::new(),
            commit_sha: String::new(),
            pipeline_url: String::new(),
            git_repo: String::new(),
            cluster_id: String::new(),
            yaml_diff: String::new(),
        }
    }

    #[test]
    fn store_add_and_get() {
        let mut reg = ChangeRegistry::new();
        let e = make_event("ce-store-1", "configmap_updated", "cm:order-config", "ConfigMap", "2026-06-19T03:00:00Z");
        reg.add(e);
        let got = reg.get("ce-store-1");
        assert!(got.is_some());
        assert_eq!(got.unwrap().change_event_id, "ce-store-1");
        assert!(reg.get("ce-missing").is_none());
    }

    #[test]
    fn store_filter_by_type() {
        let mut reg = ChangeRegistry::new();
        reg.add(make_event("ce-a", "configmap_updated", "cm:order-config", "ConfigMap", "2026-06-19T03:00:00Z"));
        reg.add(make_event("ce-b", "secret_rotated", "secret:order-db", "Secret", "2026-06-19T03:01:00Z"));
        let cm = reg.list(&ChangeFilter {
            change_type: Some(ChangeType::ConfigmapUpdated),
            ..Default::default()
        });
        assert_eq!(cm.len(), 1);
        assert_eq!(cm[0].change_event_id, "ce-a");
    }

    #[test]
    fn store_filter_by_target() {
        let mut reg = ChangeRegistry::new();
        reg.add(make_event("ce-c", "configmap_updated", "cm:order-config", "ConfigMap", "2026-06-19T03:00:00Z"));
        reg.add(make_event("ce-d", "configmap_updated", "cm:other", "ConfigMap", "2026-06-19T03:00:00Z"));
        let hits = reg.list(&ChangeFilter {
            target_resource_id: Some("cm:order-config".into()),
            ..Default::default()
        });
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].change_event_id, "ce-c");
    }

    #[test]
    fn store_filter_by_time_window_inclusive() {
        let mut reg = ChangeRegistry::new();
        reg.add(make_event("ce-old", "configmap_updated", "cm:x", "ConfigMap", "2026-06-18T00:00:00Z"));
        reg.add(make_event("ce-mid", "configmap_updated", "cm:x", "ConfigMap", "2026-06-19T03:00:00Z"));
        reg.add(make_event("ce-new", "configmap_updated", "cm:x", "ConfigMap", "2026-06-19T05:00:00Z"));
        let hits = reg.list(&ChangeFilter {
            since: Some("2026-06-19T00:00:00Z".into()),
            until: Some("2026-06-19T04:00:00Z".into()),
            ..Default::default()
        });
        let ids: std::collections::HashSet<&str> = hits.iter().map(|e| e.change_event_id.as_str()).collect();
        assert_eq!(ids, std::collections::HashSet::from(["ce-mid"]));
    }

    // ===== record_change(对齐 TestRecordChange)=====

    fn req(ctype: &str, target: &str) -> ChangeRequest {
        ChangeRequest {
            change_type: ctype.into(),
            target_resource_id: target.into(),
            ..Default::default()
        }
    }

    #[test]
    fn record_basic() {
        let topo = fixture_topology();
        let mut reg = ChangeRegistry::new();
        let ev = record_change(
            &mut reg,
            &topo,
            &ChangeRequest {
                change_type: "configmap_updated".into(),
                target_resource_id: "cm:order-config".into(),
                changed_by: "alice@x".into(),
                source: "manual".into(),
                description: "池大小 20 -> 50".into(),
                diff_summary: json!({"max_pool_size": {"old": 20, "new": 50}}),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(ev.change_event_id.starts_with("ce-"));
        assert_eq!(ev.target_resource_type, "ConfigMap");
        assert!(ev.propagated_to.contains(&"pod:order-api-1".to_string()));
        assert!(ev.propagated_to.contains(&"pod:order-api-2".to_string()));
        // 入了注册表
        assert!(reg.get(&ev.change_event_id).is_some());
    }

    #[test]
    fn record_severity_from_real_propagation() {
        let topo = fixture_topology();
        let mut reg = ChangeRegistry::new();
        // cm:order-config 反向命中 pods + deploy + comp + app = 5 -> medium
        let ev = record_change(&mut reg, &topo, &req("configmap_updated", "cm:order-config")).unwrap();
        assert!(ev.propagated_to.len() >= 5);
        assert_eq!(ev.severity_estimate, Severity::Medium);
        // orphan:lonely 没下游 -> low
        let ev2 = record_change(&mut reg, &topo, &req("configmap_updated", "orphan:lonely")).unwrap();
        assert!(ev2.propagated_to.is_empty());
        assert_eq!(ev2.severity_estimate, Severity::Low);
    }

    #[test]
    fn record_target_not_in_topology() {
        let topo = fixture_topology();
        let mut reg = ChangeRegistry::new();
        // PRD: target 不存在仍记录,propagated_to 为空
        let ev = record_change(&mut reg, &topo, &req("configmap_updated", "cm:does-not-exist")).unwrap();
        assert!(ev.propagated_to.is_empty());
        assert_eq!(ev.target_resource_type, "");
        assert_eq!(ev.severity_estimate, Severity::Low);
    }

    #[test]
    fn record_invalid_change_type() {
        let topo = fixture_topology();
        let mut reg = ChangeRegistry::new();
        let err = record_change(&mut reg, &topo, &req("bogus_type", "cm:order-config")).unwrap_err();
        assert_eq!(err.code, 400);
        assert!(err.message.contains("invalid change_type"));
    }

    #[test]
    fn record_invalid_source() {
        let topo = fixture_topology();
        let mut reg = ChangeRegistry::new();
        let err = record_change(
            &mut reg,
            &topo,
            &ChangeRequest {
                change_type: "configmap_updated".into(),
                target_resource_id: "cm:order-config".into(),
                source: "weird".into(),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(err.code, 400);
        assert!(err.message.contains("invalid source"));
    }

    #[test]
    fn record_defaults_source_manual_and_id_format() {
        // ChangeRequest::default() source = "manual";省略 source 仍合法
        let topo = fixture_topology();
        let mut reg = ChangeRegistry::new();
        let r = ChangeRequest {
            change_type: "configmap_updated".into(),
            target_resource_id: "cm:order-config".into(),
            ..Default::default()
        };
        assert_eq!(r.source, "manual");
        assert_eq!(r.diff_summary, json!({}));
        let ev = record_change(&mut reg, &topo, &r).unwrap();
        assert_eq!(ev.source, Source::Manual);
        // ce-<12 hex>:15 字符前缀(3 + 12)
        assert!(ev.change_event_id.starts_with("ce-"));
        assert_eq!(ev.change_event_id.len(), 15);
    }

    #[test]
    fn record_commit_sha_backfills_related_commit() {
        // related_commit 优先;仅给 commit_sha 时回填 related_commit
        let topo = fixture_topology();
        let mut reg = ChangeRegistry::new();
        let ev = record_change(
            &mut reg,
            &topo,
            &ChangeRequest {
                change_type: "deployment_rolled".into(),
                target_resource_id: "deploy:order-api".into(),
                commit_sha: "abc123".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ev.related_commit, "abc123");
        assert_eq!(ev.commit_sha, "abc123");

        // 显式 related_commit 胜出
        let ev2 = record_change(
            &mut reg,
            &topo,
            &ChangeRequest {
                change_type: "deployment_rolled".into(),
                target_resource_id: "deploy:order-api".into(),
                related_commit: "explicit".into(),
                commit_sha: "abc123".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ev2.related_commit, "explicit");
    }

    #[test]
    fn record_changed_at_override_or_now() {
        let topo = fixture_topology();
        let mut reg = ChangeRegistry::new();
        let ev = record_change(
            &mut reg,
            &topo,
            &ChangeRequest {
                change_type: "configmap_updated".into(),
                target_resource_id: "cm:order-config".into(),
                changed_at: Some("2026-06-19T03:00:00Z".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ev.changed_at, "2026-06-19T03:00:00Z");

        // 省略 -> now_iso(格式校验)
        let ev2 = record_change(&mut reg, &topo, &req("configmap_updated", "cm:order-config")).unwrap();
        assert!(ev2.changed_at.ends_with('Z'));
        assert_eq!(ev2.changed_at.len(), "2026-06-19T03:00:00Z".len());
    }
}
