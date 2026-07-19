//! incident_report 报告采集器(PRD-003 Phase 4.2b,对齐 reference incident_modules.py)。
//!
//! 围绕一个"锚点"展开:scope.change_event_id -> ChangeEvent。Rust 无 fault injection,
//! fault_id 不支持(解析失败)。事件 = 锚点 + 反向 BFS 受影响节点 + 时间窗内交叉的变更与恢复。
//! 全 I/O-free 吃 &Topology/&ChangeRegistry/&ExecutionRegistry。
//!
//! **偏差**:丢 fault 锚点(Rust 无 fault injection);丢 Neo4j 全局 store(吃传入 registry)。

#![allow(missing_docs)]

use std::collections::{HashMap, HashSet};

use engine_changes::{derive_propagation, ChangeFilter, ChangeRegistry, ChangeType, Severity};
use engine_identity::Topology;
use engine_recovery::{ExecutionRegistry, RecoveryStatus, suggest_for_change};
use serde::Serialize;

use crate::models::ReportScope;

// 统一锚点(屏蔽 fault / change;Rust 仅 change)。
#[derive(Debug, Clone)]
pub struct IncidentAnchor {
    /// "change"(Rust 仅此一种;"fault" 不支持)。
    pub kind: String,
    pub anchor_id: String,
    pub target_id: String,
    pub target_type: String,
    pub timestamp: String,
    pub description: String,
    pub severity: String,
    /// 变更类型(snake_case;change 锚点给 suggest_for_change 用)。
    pub change_type: String,
}

/// 从 scope 解析锚点。优先 change_event_id;fault_id -> Err(Rust 无 fault injection);
/// 两者皆无 -> Err。
pub fn resolve_anchor(changes: &ChangeRegistry, scope: &ReportScope) -> Result<IncidentAnchor, String> {
    if let Some(fault_id) = &scope.fault_id {
        return Err(format!(
            "fault_id 不支持(Rust 无 fault injection): {fault_id}"
        ));
    }
    let ce_id = scope
        .change_event_id
        .as_deref()
        .ok_or_else(|| "incident scope 需要 change_event_id(fault_id 不支持)".to_string())?;
    let event = changes
        .get(ce_id)
        .ok_or_else(|| format!("change_event_id 未找到: {ce_id}"))?;
    let description = if event.description.is_empty() {
        change_type_str(&event.change_type)
    } else {
        event.description.clone()
    };
    Ok(IncidentAnchor {
        kind: "change".to_string(),
        anchor_id: event.change_event_id.clone(),
        target_id: event.target_resource_id.clone(),
        target_type: event.target_resource_type.clone(),
        timestamp: event.changed_at.clone(),
        description,
        severity: severity_str(&event.severity_estimate),
        change_type: change_type_str(&event.change_type),
    })
}

fn all_changes_filter() -> ChangeFilter {
    ChangeFilter {
        change_type: None,
        target_resource_id: None,
        source: None,
        since: None,
        until: None,
    }
}

fn change_type_str(ct: &ChangeType) -> String {
    serde_json::to_string(ct)
        .unwrap_or_default()
        .trim_matches('"')
        .to_string()
}

fn severity_str(s: &Severity) -> String {
    serde_json::to_string(s)
        .unwrap_or_default()
        .trim_matches('"')
        .to_string()
}

fn status_str(s: &RecoveryStatus) -> String {
    serde_json::to_string(s)
        .unwrap_or_default()
        .trim_matches('"')
        .to_string()
}

/// 锚点类型中文标签(generator 头部 + summary 共用)。
pub fn anchor_kind_label(kind: &str) -> String {
    match kind {
        "fault" => "故障注入".to_string(),
        _ => "变更事件".to_string(),
    }
}

/// 锚点 ±window 秒时间窗比较(ISO8601;任一不可解析 -> false,排除)。
fn within_window(ts: &str, anchor_ts: &str, window_seconds: i64) -> bool {
    match (
        engine_changes::iso::parse_iso_utc(ts),
        engine_changes::iso::parse_iso_utc(anchor_ts),
    ) {
        (Some(t), Some(a)) => (t - a).num_seconds().abs() <= window_seconds,
        _ => false,
    }
}

/// 受影响节点集合(反向 BFS,max_depth=4)+ 锚点自身。
fn propagated_with_target(anchor: &IncidentAnchor, topology: &Topology) -> HashSet<String> {
    let mut set: HashSet<String> = derive_propagation(&anchor.target_id, topology, 4, None)
        .into_iter()
        .collect();
    set.insert(anchor.target_id.clone());
    set
}

// ===== 模块 1: incident_summary =====

#[derive(Debug, Clone, Serialize)]
pub struct AffectedTypeCount {
    pub type_name: String,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct AffectedNode {
    pub resource_id: String,
    pub resource_type: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct IncidentSummary {
    pub kind: String,
    pub kind_label: String,
    pub anchor_id: String,
    pub target_id: String,
    pub target_type: String,
    pub timestamp: String,
    pub description: String,
    pub severity: String,
    pub affected_total: u32,
    /// 按 type_name 字典序排序(确定性,替代 Jinja2 dictsort)。
    pub affected_by_type: Vec<AffectedTypeCount>,
    pub affected_nodes: Vec<AffectedNode>,
}

pub fn gather_incident_summary(anchor: &IncidentAnchor, topology: &Topology) -> IncidentSummary {
    let propagated = derive_propagation(&anchor.target_id, topology, 4, None);

    let mut by_type: HashMap<String, u32> = HashMap::new();
    let mut nodes: Vec<AffectedNode> = Vec::new();
    for nid in &propagated {
        if let Some(n) = topology.nodes.iter().find(|n| &n.resource_id == nid) {
            *by_type.entry(n.resource_type.clone()).or_insert(0) += 1;
            nodes.push(AffectedNode {
                resource_id: n.resource_id.clone(),
                resource_type: n.resource_type.clone(),
                name: n.label.clone(),
            });
        }
    }

    let mut affected_by_type: Vec<AffectedTypeCount> = by_type
        .into_iter()
        .map(|(type_name, count)| AffectedTypeCount { type_name, count })
        .collect();
    affected_by_type.sort_by(|a, b| a.type_name.cmp(&b.type_name));

    IncidentSummary {
        kind: anchor.kind.clone(),
        kind_label: anchor_kind_label(&anchor.kind),
        anchor_id: anchor.anchor_id.clone(),
        target_id: anchor.target_id.clone(),
        target_type: anchor.target_type.clone(),
        timestamp: anchor.timestamp.clone(),
        description: anchor.description.clone(),
        severity: anchor.severity.clone(),
        affected_total: nodes.len() as u32,
        affected_by_type,
        affected_nodes: nodes,
    }
}

// ===== 模块 2: incident_timeline =====

#[derive(Debug, Clone, Serialize)]
pub struct IncidentTimelineItem {
    pub kind: String,
    pub kind_label: String,
    pub timestamp: String,
    /// change_type 或 action_id(序列化为 "type",对齐模板 `it.type`)。
    #[serde(rename = "type")]
    pub type_field: String,
    pub target_id: String,
    pub actor: String,
    pub description: String,
    /// 变更 severity 或恢复 status。
    pub severity: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct IncidentTimeline {
    pub anchor_id: String,
    pub anchor_timestamp: String,
    pub window_seconds: u32,
    pub total: u32,
    pub events: Vec<IncidentTimelineItem>,
}

pub fn gather_incident_timeline(
    anchor: &IncidentAnchor,
    topology: &Topology,
    changes: &ChangeRegistry,
    executions: &ExecutionRegistry,
    window_seconds: i64,
) -> IncidentTimeline {
    let propagated = propagated_with_target(anchor, topology);

    let mut items: Vec<IncidentTimelineItem> = Vec::new();

    for c in changes.list(&all_changes_filter()) {
        if !propagated.contains(&c.target_resource_id) {
            continue;
        }
        if !within_window(&c.changed_at, &anchor.timestamp, window_seconds) {
            continue;
        }
        items.push(IncidentTimelineItem {
            kind: "change".to_string(),
            kind_label: "变更".to_string(),
            timestamp: c.changed_at.clone(),
            type_field: change_type_str(&c.change_type),
            target_id: c.target_resource_id.clone(),
            actor: c.changed_by.clone(),
            description: c.description.clone(),
            severity: severity_str(&c.severity_estimate),
        });
    }

    for e in executions.list() {
        if !propagated.contains(&e.target_resource_id) {
            continue;
        }
        if !within_window(&e.initiated_at, &anchor.timestamp, window_seconds) {
            continue;
        }
        items.push(IncidentTimelineItem {
            kind: "recovery".to_string(),
            kind_label: "恢复".to_string(),
            timestamp: e.initiated_at.clone(),
            type_field: e.action_id.clone(),
            target_id: e.target_resource_id.clone(),
            actor: e.initiated_by.clone(),
            description: e.request_reason.clone(),
            severity: status_str(&e.status),
        });
    }

    items.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

    IncidentTimeline {
        anchor_id: anchor.anchor_id.clone(),
        anchor_timestamp: anchor.timestamp.clone(),
        window_seconds: window_seconds as u32,
        total: items.len() as u32,
        events: items,
    }
}

// ===== 模块 3: incident_recoveries =====

#[derive(Debug, Clone, Serialize)]
pub struct IncidentExecuted {
    /// execution_id 前 12 字符(对齐模板 `e.execution_id[:12]`;Tera 无字符串切片)。
    pub execution_id_short: String,
    pub action_id: String,
    pub target_id: String,
    pub status: String,
    pub initiated_by: String,
    pub initiated_at: String,
    pub completed_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct IncidentRecommended {
    pub action_id: String,
    pub target_id: String,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct IncidentRecoveries {
    pub anchor_id: String,
    pub executed_total: u32,
    pub executed: Vec<IncidentExecuted>,
    pub recommended_total: u32,
    pub recommended: Vec<IncidentRecommended>,
}

pub fn gather_incident_recoveries(
    anchor: &IncidentAnchor,
    topology: &Topology,
    executions: &ExecutionRegistry,
) -> IncidentRecoveries {
    let propagated = propagated_with_target(anchor, topology);

    let mut executed: Vec<IncidentExecuted> = Vec::new();
    for e in executions.list() {
        if !propagated.contains(&e.target_resource_id) {
            continue;
        }
        executed.push(IncidentExecuted {
            execution_id_short: e.execution_id.chars().take(12).collect(),
            action_id: e.action_id.clone(),
            target_id: e.target_resource_id.clone(),
            status: status_str(&e.status),
            initiated_by: e.initiated_by.clone(),
            initiated_at: e.initiated_at.clone(),
            completed_at: e.completed_at.clone(),
        });
    }

    // 推荐后续:change 锚点复用 PRD-001 suggest_for_change(对齐 reference)。
    let mut recommended: Vec<IncidentRecommended> = Vec::new();
    if anchor.kind == "change" {
        for sugg in suggest_for_change(&anchor.change_type) {
            recommended.push(IncidentRecommended {
                action_id: sugg.action.action_id.to_string(),
                target_id: anchor.target_id.clone(),
                rationale: sugg.rationale.to_string(),
            });
        }
    }

    IncidentRecoveries {
        anchor_id: anchor.anchor_id.clone(),
        executed_total: executed.len() as u32,
        executed,
        recommended_total: recommended.len() as u32,
        recommended,
    }
}
