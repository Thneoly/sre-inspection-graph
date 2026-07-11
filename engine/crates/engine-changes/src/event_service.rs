//! ChangeEvent 业务编排(复刻 `reference/app/changes/event_service.py` 的 `record_change`
//! + DSS store CRUD + `_estimate_severity` + `correlated_changes` + `get_recovery_suggestion`
//! + `_serialize_event`)。
//!
//! ## 与 reference 的差异
//!
//! - **I/O-free 注册表**:reference 写全局 DSS `store` + best-effort Neo4j dual-write +
//!   alert 关联;本 port 只写内存 [`ChangeRegistry`](对齐 DSS 主存),**丢弃 Neo4j dual-write**。
//! - **频率提升(3.5 接入)**:`record_change` 末尾调 [`crate::frequency::apply_frequency_check`]
//!   (可能 low->medium + `[过频变更]` tag;计数含当前事件,对齐 reference add 后调)。
//! - **入参结构体**:reference `record_change` 平铺 13 参(默认值);本 port 用 [`ChangeRequest`]
//!   包住(避免 `too_many_arguments`),[`ChangeRequest::default`] 复刻默认值。
//! - **拓扑入参**:reference `record_change`/`correlated_changes`/`get_recovery_suggestion`
//!   内部读全局 `store.get_node` / `derive_propagation` / `find_propagation_path`;本 port
//!   显式接 `&Topology`(纯领域,orchestration 层喂 materialized topology)。
//! - **`get_recovery_suggestion` 桥**:调 [`engine_recovery::suggest_for_change`](已 3.1 移植),
//!   嵌套 `ActionSuggestion { action, rationale, confidence }` -> 扁平化 per-suggestion dict
//!   (对齐 reference `sugg.get(...)` 字段访问),`target_match` direct/propagated/unresolved。

#![allow(missing_docs)]

use engine_identity::Topology;
use serde_json::Value;
use uuid::Uuid;

use crate::iso::{now_iso, shift_iso};
use crate::models::{ChangeError, ChangeEvent, ChangeFilter, ChangeRequest, ChangeType, Severity, Source};
use crate::propagation::{derive_propagation, find_propagation_path, DEFAULT_PROPAGATION_DEPTH};

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

    /// 从已加载的事件列表构造(orchestration 从 storage 恢复用,Phase 3.6)。
    pub fn from_events(events: Vec<ChangeEvent>) -> Self {
        Self { events }
    }

    /// 追加一个事件(对齐 `store.add_change_event`)。
    pub fn add(&mut self, event: ChangeEvent) {
        self.events.push(event);
    }

    /// 按 ID 取事件(对齐 `store.get_change_event`)。
    pub fn get(&self, change_event_id: &str) -> Option<&ChangeEvent> {
        self.events.iter().find(|e| e.change_event_id == change_event_id)
    }

    /// 按 ID 取事件的可变引用(`record_change` 频率检查后回写 severity/description 用)。
    pub fn get_mut(&mut self, change_event_id: &str) -> Option<&mut ChangeEvent> {
        self.events.iter_mut().find(|e| e.change_event_id == change_event_id)
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

    let mut event = ChangeEvent {
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
    // 3.5: 频率检查(计数含刚加入的当前事件,对齐 reference add 后调 `_apply_frequency_check`)
    crate::frequency::apply_frequency_check(&mut event, registry);
    // 同步 mutation(severity/description 可能被频率检查改)回 registry 存储项
    if let Some(stored) = registry.get_mut(&event.change_event_id) {
        stored.severity_estimate = event.severity_estimate;
        stored.description = event.description.clone();
    }
    Ok(event)
}

/// 生成 `ce-<12 hex>` ID(对齐 reference `f"ce-{uuid.uuid4().hex[:12]}"`)。
fn new_change_id() -> String {
    let hex = Uuid::new_v4().simple().to_string();
    format!("ce-{}", &hex[..12])
}

/// 序列化事件为 JSON Value(对齐 reference `_serialize_event`)。
///
/// 全 18 字段(enums snake_case)+ `propagated_count`。
pub fn serialize(event: &ChangeEvent) -> Value {
    let mut v = serde_json::to_value(event).expect("ChangeEvent serializable");
    if let Some(map) = v.as_object_mut() {
        map.insert("propagated_count".to_string(), Value::from(event.propagated_to.len()));
    }
    v
}

/// `correlated_changes` 默认窗口 300s(对齐 reference)。
pub const DEFAULT_CORRELATED_WINDOW_SECONDS: i64 = 300;

/// `correlated_changes` 单条命中(序列化事件 + match_type + propagation_distance)。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CorrelatedChange {
    /// 序列化事件字段(对齐 `_serialize_event`)。
    #[serde(flatten)]
    pub event: Value,
    /// `direct`(event.target == 查询 target)或 `propagated`(查询 target ∈ propagated_to)。
    pub match_type: String,
    /// 传播跳数(direct=0;propagated=`max(path.len()-1, 1)`)。
    pub propagation_distance: usize,
}

/// `correlated_changes` 返回(对齐 reference)。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CorrelatedResult {
    pub target_resource_id: String,
    pub window_start: String,
    pub window_end: String,
    pub now: String,
    pub include_propagated: bool,
    pub changes: Vec<CorrelatedChange>,
    pub total: usize,
}

/// 查询 target 在指定时间窗口内的相关变更(对齐 reference `correlated_changes`)。
///
/// 时间窗:
/// - `since` + `until` -> `[since, until]` 闭区间
/// - `since` only -> `[since, since + window]`
/// - 都不给 -> `[now - window, now]`(`until` only 也走此分支:`[until - window, until]`)
///
/// `match_type`:`direct`(event.target == target)或 `propagated`(target ∈ event.propagated_to
/// 且 `include_propagated`)。`propagation_distance` = `find_propagation_path` 跳数(`max(.., 1)`)。
/// 结果按 `changed_at` 倒序。
pub fn correlated_changes(
    registry: &ChangeRegistry,
    topology: &Topology,
    target_resource_id: &str,
    window_seconds: i64,
    since: Option<&str>,
    until: Option<&str>,
    include_propagated: bool,
) -> CorrelatedResult {
    let now = now_iso();
    let (win_start, win_end) = match (since, until) {
        (Some(s), Some(u)) => (s.to_string(), u.to_string()),
        (Some(s), None) => (s.to_string(), shift_iso(s, window_seconds)),
        (None, u_opt) => {
            let end = u_opt.map(|u| u.to_string()).unwrap_or_else(|| now.clone());
            let start = shift_iso(&end, -window_seconds);
            (start, end)
        }
    };

    let mut matches: Vec<CorrelatedChange> = Vec::new();
    for ev in registry.list(&ChangeFilter {
        since: Some(win_start.clone()),
        until: Some(win_end.clone()),
        ..Default::default()
    }) {
        let (match_type, distance) = if ev.target_resource_id == target_resource_id {
            ("direct".to_string(), 0usize)
        } else if include_propagated && ev.propagated_to.contains(&target_resource_id.to_string()) {
            let path = find_propagation_path(
                &ev.target_resource_id,
                target_resource_id,
                topology,
                DEFAULT_PROPAGATION_DEPTH,
                None,
            );
            let dist = path.len().saturating_sub(1).max(1);
            ("propagated".to_string(), dist)
        } else {
            continue;
        };
        matches.push(CorrelatedChange {
            event: serialize(ev),
            match_type,
            propagation_distance: distance,
        });
    }
    // 按 changed_at 倒序(从 event Value 取 changed_at)
    matches.sort_by(|a, b| {
        let ta = a.event.get("changed_at").and_then(Value::as_str).unwrap_or("");
        let tb = b.event.get("changed_at").and_then(Value::as_str).unwrap_or("");
        tb.cmp(ta)
    });
    let total = matches.len();
    CorrelatedResult {
        target_resource_id: target_resource_id.to_string(),
        window_start: win_start,
        window_end: win_end,
        now,
        include_propagated,
        changes: matches,
        total,
    }
}

/// `get_recovery_suggestion` 单条推荐(对齐 reference `enriched` dict,10 字段)。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecoverySuggestion {
    pub action_id: String,
    pub action_name: String,
    pub rationale: String,
    pub confidence: f32,
    pub risk_level: engine_recovery::RiskLevel,
    pub requires_approval: bool,
    pub target_type: String,
    pub resolved_target_resource_id: Option<String>,
    pub resolved_target_type: String,
    pub target_match: TargetMatch,
}

/// 推荐目标的解析层级(对齐 reference `target_match`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetMatch {
    /// 事件 target 类型 == 动作 target_type。
    Direct,
    /// propagated_to 中找到类型匹配节点。
    Propagated,
    /// 无匹配,resolved_target_resource_id = None。
    Unresolved,
}

/// `get_recovery_suggestion` 返回(对齐 reference)。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecoverySuggestionResult {
    pub change_event_id: String,
    pub change_type: ChangeType,
    pub target_resource_id: String,
    pub target_resource_type: String,
    pub suggestions: Vec<RecoverySuggestion>,
    pub total: usize,
}

/// 给定变更事件 ID,返回可直接调起的 PRD-001 恢复动作推荐
/// (对齐 reference `get_recovery_suggestion`,桥 [`engine_recovery::suggest_for_change`])。
///
/// 解析"可执行目标":
/// - `direct`:事件 target_resource_type == action.target_type -> 用事件 target 本身
/// - `propagated`:否则在 `propagated_to` 里找第一个类型匹配的节点(拓扑查类型)
/// - `unresolved`:propagated_to 里也没有 -> resolved_target 为 None
///
/// 未找到事件 -> [`ChangeError`] 404。
pub fn get_recovery_suggestion(
    registry: &ChangeRegistry,
    topology: &Topology,
    event_id: &str,
) -> Result<RecoverySuggestionResult, ChangeError> {
    let event = registry
        .get(event_id)
        .ok_or_else(|| ChangeError::with_code(format!("change_event not found: {event_id}"), 404))?;

    let suggestions = engine_recovery::suggest_for_change(event.change_type.as_str());
    let enriched: Vec<RecoverySuggestion> = suggestions
        .iter()
        .map(|sugg| {
            let action_target_type = sugg.action.target_type;
            let (resolved_id, resolved_type, match_kind) =
                if !action_target_type.is_empty() && event.target_resource_type == action_target_type {
                    (Some(event.target_resource_id.clone()), event.target_resource_type.clone(), TargetMatch::Direct)
                } else {
                    // 沿 propagated_to 找第一个类型匹配节点
                    let hit = event.propagated_to.iter().find_map(|pid| {
                        topology
                            .nodes
                            .iter()
                            .find(|n| n.resource_id == *pid && n.resource_type == action_target_type)
                    });
                    match hit {
                        Some(node) => (Some(node.resource_id.clone()), node.resource_type.clone(), TargetMatch::Propagated),
                        None => (None, String::new(), TargetMatch::Unresolved),
                    }
                };
            RecoverySuggestion {
                action_id: sugg.action.action_id.to_string(),
                action_name: sugg.action.name.to_string(),
                rationale: sugg.rationale.to_string(),
                confidence: sugg.confidence,
                risk_level: sugg.action.risk_level,
                requires_approval: sugg.action.requires_approval,
                target_type: action_target_type.to_string(),
                resolved_target_resource_id: resolved_id,
                resolved_target_type: resolved_type,
                target_match: match_kind,
            }
        })
        .collect();
    let total = enriched.len();
    Ok(RecoverySuggestionResult {
        change_event_id: event.change_event_id.clone(),
        change_type: event.change_type,
        target_resource_id: event.target_resource_id.clone(),
        target_resource_type: event.target_resource_type.clone(),
        suggestions: enriched,
        total,
    })
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

    // ===== correlated_changes(对齐 TestCorrelatedQuery,sprint1 10 节点 fixture)=====

    fn record_at(ctype: &str, target: &str, changed_at: &str) -> ChangeRequest {
        ChangeRequest {
            change_type: ctype.into(),
            target_resource_id: target.into(),
            changed_at: Some(changed_at.into()),
            source: "manual".into(),
            ..Default::default()
        }
    }

    #[test]
    fn correlated_direct_match() {
        let topo = fixture_topology();
        let mut reg = ChangeRegistry::new();
        let _ev = record_change(&mut reg, &topo, &record_at("configmap_updated", "cm:order-config", "2026-06-19T03:00:00Z")).unwrap();
        let result = correlated_changes(
            &reg,
            &topo,
            "cm:order-config",
            300,
            Some("2026-06-19T02:55:00Z"),
            Some("2026-06-19T03:05:00Z"),
            true,
        );
        assert_eq!(result.total, 1);
        assert_eq!(result.changes[0].match_type, "direct");
        assert_eq!(result.changes[0].propagation_distance, 0);
    }

    #[test]
    fn correlated_propagated_match() {
        let topo = fixture_topology();
        let mut reg = ChangeRegistry::new();
        let _ev = record_change(&mut reg, &topo, &record_at("configmap_updated", "cm:order-config", "2026-06-19T03:00:00Z")).unwrap();
        // pod:order-api-1 在 cm 的 propagated_to 里
        let result = correlated_changes(
            &reg,
            &topo,
            "pod:order-api-1",
            300,
            Some("2026-06-19T02:55:00Z"),
            Some("2026-06-19T03:05:00Z"),
            true,
        );
        assert_eq!(result.total, 1);
        assert_eq!(result.changes[0].match_type, "propagated");
        assert!(result.changes[0].propagation_distance >= 1);
    }

    #[test]
    fn correlated_window_excludes_old() {
        let topo = fixture_topology();
        let mut reg = ChangeRegistry::new();
        let _ev = record_change(&mut reg, &topo, &record_at("configmap_updated", "cm:order-config", "2026-06-19T01:00:00Z")).unwrap();
        let result = correlated_changes(
            &reg,
            &topo,
            "cm:order-config",
            300,
            Some("2026-06-19T02:55:00Z"),
            Some("2026-06-19T03:05:00Z"),
            true,
        );
        assert_eq!(result.total, 0);
    }

    #[test]
    fn correlated_default_window_excludes_ancient() {
        let topo = fixture_topology();
        let mut reg = ChangeRegistry::new();
        let _ev = record_change(&mut reg, &topo, &record_at("configmap_updated", "cm:order-config", "2020-01-01T00:00:00Z")).unwrap();
        let result = correlated_changes(&reg, &topo, "cm:order-config", 300, None, None, true);
        assert_eq!(result.total, 0);
        assert!(!result.window_start.is_empty());
        assert!(!result.window_end.is_empty());
    }

    #[test]
    fn correlated_include_propagated_false_drops_propagated() {
        let topo = fixture_topology();
        let mut reg = ChangeRegistry::new();
        let _ev = record_change(&mut reg, &topo, &record_at("configmap_updated", "cm:order-config", "2026-06-19T03:00:00Z")).unwrap();
        // pod 只能通过 propagated 匹配;关掉 -> 0
        let result = correlated_changes(
            &reg,
            &topo,
            "pod:order-api-1",
            300,
            Some("2026-06-19T02:55:00Z"),
            Some("2026-06-19T03:05:00Z"),
            false,
        );
        assert_eq!(result.total, 0);
    }

    #[test]
    fn correlated_sorted_desc_by_changed_at() {
        let topo = fixture_topology();
        let mut reg = ChangeRegistry::new();
        // 乱序插入:03:00, 03:02, 03:01
        for ts in ["2026-06-19T03:00:00Z", "2026-06-19T03:02:00Z", "2026-06-19T03:01:00Z"] {
            record_change(&mut reg, &topo, &record_at("configmap_updated", "cm:order-config", ts)).unwrap();
        }
        let result = correlated_changes(
            &reg,
            &topo,
            "cm:order-config",
            300,
            Some("2026-06-19T02:55:00Z"),
            Some("2026-06-19T03:05:00Z"),
            true,
        );
        let ts: Vec<&str> = result.changes.iter().map(|c| c.event.get("changed_at").and_then(Value::as_str).unwrap_or("")).collect();
        let mut sorted = ts.clone();
        sorted.sort();
        sorted.reverse();
        assert_eq!(ts, sorted, "should be descending: {:?}", ts);
    }

    // ===== get_recovery_suggestion(对齐 TestRecoverySuggestion,sprint1 10 节点 fixture)=====

    #[test]
    fn suggestion_deployment_rolled_direct_match() {
        let topo = fixture_topology();
        let mut reg = ChangeRegistry::new();
        let ev = record_change(&mut reg, &topo, &req("deployment_rolled", "deploy:order-api")).unwrap();
        let sug = get_recovery_suggestion(&reg, &topo, &ev.change_event_id).unwrap();
        assert_eq!(sug.change_type, ChangeType::DeploymentRolled);
        assert!(sug.total >= 1);
        let top = &sug.suggestions[0];
        assert_eq!(top.action_id, "rollback_deployment");
        assert_eq!(top.target_match, TargetMatch::Direct);
        assert_eq!(top.resolved_target_resource_id.as_deref(), Some("deploy:order-api"));
        assert_eq!(top.resolved_target_type, "Deployment");
        assert!(top.requires_approval);
    }

    #[test]
    fn suggestion_configmap_resolves_deployment_via_propagation() {
        let topo = fixture_topology();
        let mut reg = ChangeRegistry::new();
        let ev = record_change(&mut reg, &topo, &req("configmap_updated", "cm:order-config")).unwrap();
        // 预断言:deploy 在 cm 的 propagated_to 里
        assert!(ev.propagated_to.contains(&"deploy:order-api".to_string()));
        let sug = get_recovery_suggestion(&reg, &topo, &ev.change_event_id).unwrap();
        let top = &sug.suggestions[0];
        assert_eq!(top.action_id, "rollback_deployment");
        assert_eq!(top.target_match, TargetMatch::Propagated);
        assert_eq!(top.resolved_target_resource_id.as_deref(), Some("deploy:order-api"));
        assert_eq!(top.resolved_target_type, "Deployment");
    }

    #[test]
    fn suggestion_image_pushed_unresolved_when_no_deployment_in_propagation() {
        let topo = fixture_topology();
        let mut reg = ChangeRegistry::new();
        let ev = record_change(&mut reg, &topo, &req("image_pushed", "img:order:1.2.3")).unwrap();
        // img 只有 USES_IMAGE 边(非白名单)-> propagated_to 为空
        assert!(ev.propagated_to.is_empty());
        let sug = get_recovery_suggestion(&reg, &topo, &ev.change_event_id).unwrap();
        let top = &sug.suggestions[0];
        assert_eq!(top.action_id, "rollback_deployment");
        assert_eq!(top.target_match, TargetMatch::Unresolved);
        assert!(top.resolved_target_resource_id.is_none());
    }

    #[test]
    fn suggestion_secret_rotated_has_both_direct_and_propagated() {
        let topo = fixture_topology();
        let mut reg = ChangeRegistry::new();
        let ev = record_change(&mut reg, &topo, &req("secret_rotated", "secret:order-db")).unwrap();
        let sug = get_recovery_suggestion(&reg, &topo, &ev.change_event_id).unwrap();
        let by_action: std::collections::HashMap<&str, &RecoverySuggestion> =
            sug.suggestions.iter().map(|s| (s.action_id.as_str(), s)).collect();
        // refresh_secret:直接命中 Secret
        let refresh = by_action.get("refresh_secret").expect("refresh_secret suggested");
        assert_eq!(refresh.target_match, TargetMatch::Direct);
        assert_eq!(refresh.resolved_target_resource_id.as_deref(), Some("secret:order-db"));
        // rollback_deployment:经 propagated 命中 Deployment
        let rb = by_action.get("rollback_deployment").expect("rollback_deployment suggested");
        assert_eq!(rb.target_match, TargetMatch::Propagated);
    }

    #[test]
    fn suggestion_unknown_event_404() {
        let topo = fixture_topology();
        let reg = ChangeRegistry::new();
        let err = get_recovery_suggestion(&reg, &topo, "ce-nope").unwrap_err();
        assert_eq!(err.code, 404);
    }
}
