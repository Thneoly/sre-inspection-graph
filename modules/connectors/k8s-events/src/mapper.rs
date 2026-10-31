//! K8s Event → `kind="change"` / `kind="alert"` Fact 的纯映射(host target 可单测)。
//!
//! 对照 reference `k8s_event_connector.py`:poll `GET /api/v1/namespaces/{ns}/events`,
//! 两条产线:
//! - **change**:`INTERESTING_REASONS`(`ScalingReplicaSet`/`SuccessfulRescale` →
//!   `deployment_rolled`)→ ChangeEvent 载荷(desktop run_sync 路由到 `record_change`);
//! - **alert**(live 告警源):`type=Warning` 的事件 → AlertEvent 载荷(路由到
//!   `record_alert` 语义的 registry 录入)。K8s Warning event 本身就是集群自报的
//!   异常信号(BackOff/Unhealthy/FailedScheduling/FailedMount...),全量收、按
//!   UID dedup —— 不做 reason 白名单(任意裁剪反而漏告警);同 UID 的 count
//!   增长视为同一告警(v1 语义,不升计数)。
//!
//! 两种 Fact 都不产节点/边/metric(facts_to_graph 只认 topology-node/edge)。
//! UID dedup + 首次 baseline 是有状态逻辑,在 `lib.rs`(guest static);本文件只做
//! 无状态映射。

use module_sdk::Fact;
use serde::Deserialize;

/// source 标识(Fact.source = connector 名)。
pub const SOURCE: &str = "k8s-events";
/// ChangeRequest.source(record_change 校验 -> Source enum;对照 reference `k8s_api`)。
pub const CHANGE_SOURCE: &str = "k8s_api";
const KIND: &str = "change";
const ALERT_KIND: &str = "alert";

/// K8s Event list 响应。
#[derive(Deserialize, Default)]
pub struct EventList {
    #[serde(default)]
    pub items: Vec<Event>,
}

/// 单条 K8s Event(只取映射需要的字段)。
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Event {
    metadata: Meta,
    /// Normal / Warning(alert 产线的开关)。
    #[serde(rename = "type")]
    event_type: String,
    reason: String,
    message: String,
    /// 事件最近发生时间(RFC3339,alert 的 fired_at)。
    last_timestamp: String,
    count: i64,
    involved_object: InvolvedObject,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Meta {
    uid: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct InvolvedObject {
    kind: String,
    name: String,
}

/// Event UID(供 lib.rs dedup / baseline 读;metadata.uid 私有)。
pub fn event_uid(ev: &Event) -> String {
    ev.metadata.uid.clone()
}

/// 只关心这两个 reason(对照 reference `INTERESTING_REASONS`)。其余 reason -> None(跳过)。
pub fn change_type_for_reason(reason: &str) -> Option<&'static str> {
    match reason {
        "ScalingReplicaSet" | "SuccessfulRescale" => Some("deployment_rolled"),
        _ => None,
    }
}

/// 聚合参数。
#[derive(Clone)]
pub struct Cfg {
    pub cluster: String,
    pub namespace: String,
    pub now: u64,
}

impl Cfg {
    pub fn new(cluster: &str, namespace: &str, now: u64) -> Self {
        Self {
            cluster: cluster.to_string(),
            namespace: namespace.to_string(),
            now,
        }
    }
}

/// `involvedObject` kind/name + cluster/ns → target resource_id(对照 reference
/// `_event_to_change`)。Deployment 直映;ReplicaSet 砍末段 hash;Pod 直映;其余 -> None。
pub fn target_resource_id(kind: &str, name: &str, cfg: &Cfg) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    let (c, ns) = (cfg.cluster.as_str(), cfg.namespace.as_str());
    match kind {
        "Deployment" => Some(format!("deploy:{c}:{ns}:{name}")),
        "ReplicaSet" => {
            // 砍末段 hash:frontend-87bbfc4c9 -> frontend(name 不含 "-" 则原样)。
            let deploy = name.rsplit_once('-').map(|(base, _)| base).unwrap_or(name);
            Some(format!("deploy:{c}:{ns}:{deploy}"))
        }
        "Pod" => Some(format!("pod:{c}:{ns}:{name}")),
        _ => None,
    }
}

/// 单条 K8s Event → change Fact(对照 reference `_event_to_change`)。
/// reason 不在白名单 / kind 未知 / name 空 -> None。
pub fn event_to_change_fact(ev: &Event, cfg: &Cfg) -> Option<Fact> {
    let change_type = change_type_for_reason(&ev.reason)?;
    let target = target_resource_id(&ev.involved_object.kind, &ev.involved_object.name, cfg)?;
    let description = if ev.message.is_empty() {
        ev.reason.clone()
    } else {
        // 对照 reference:msg[:200]
        let m: String = ev.message.chars().take(200).collect();
        format!("{}: {m}", ev.reason)
    };
    let diff_summary = serde_json::json!({
        "reason": ev.reason,
        "kind": ev.involved_object.kind,
        "name": ev.involved_object.name,
    });
    Some(change_fact(cfg.now, &ev.metadata.uid, &target, change_type, description, diff_summary))
}

/// 单条 K8s Event → alert Fact(live 告警源)。`type=Warning` 才产,其余 None。
///
/// attributes_json 载 AlertEvent 字段子集(desktop 解码进 AlertRegistry):
/// `alert_name` = reason、`resource_ref` = involvedObject 映射(target_resource_id
/// 映不出的 kind 用 `k8s:{kind}:{ns}:{name}` 原样兜底 —— AlertEvent.resource_ref
/// 是任意引用串,不像 change 那样要求 canonical rid)、`fired_at` = lastTimestamp。
pub fn event_to_alert_fact(ev: &Event, cfg: &Cfg) -> Option<Fact> {
    if ev.event_type != "Warning" {
        return None;
    }
    let io = &ev.involved_object;
    let resource_ref = target_resource_id(&io.kind, &io.name, cfg)
        .unwrap_or_else(|| format!("k8s:{}:{}:{}", io.kind, cfg.namespace, io.name));
    let summary: String = if ev.message.is_empty() {
        ev.reason.clone()
    } else {
        ev.message.chars().take(200).collect()
    };
    Some(alert_fact(
        cfg.now,
        &ev.metadata.uid,
        &resource_ref,
        &ev.reason,
        summary,
        &ev.last_timestamp,
        &cfg.cluster,
        ev.count,
    ))
}

/// 构造一条 `kind="alert"` Fact。resource_id 用 event uid 保证唯一且不进图
/// (facts_to_graph 只认 topology-node/edge);id 含 uid + ts 防跨轮撞。
#[allow(clippy::too_many_arguments)]
fn alert_fact(
    now: u64,
    uid: &str,
    resource_ref: &str,
    alert_name: &str,
    summary: String,
    fired_at: &str,
    cluster: &str,
    count: i64,
) -> Fact {
    Fact {
        id: format!("{SOURCE}:alert:{uid}:{now}"),
        kind: ALERT_KIND.to_string(),
        source: SOURCE.to_string(),
        resource_id: format!("k8s-event:{uid}"),
        resource_type: "AlertEvent".to_string(),
        timestamp: now,
        attributes_json: serde_json::json!({
            "alert_name": alert_name,
            "resource_ref": resource_ref,
            "severity": "warning",
            "status": "firing",
            // lastTimestamp 缺失(极老 event)时 desktop 侧回退 now
            "fired_at": fired_at,
            "rule_id": "k8s-event",
            "summary": summary,
            "cluster_id": cluster,
            "event_count": count,
        })
        .to_string(),
    }
}

/// 构造一条 `kind="change"` Fact。attributes_json 载 ChangeRequest 子集(desktop 解码 ->
/// `record_change`)。id 含 uid + ts(每事件唯一 + 跨轮不撞);resource_id = target(溯源)。
fn change_fact(
    now: u64,
    uid: &str,
    target: &str,
    change_type: &str,
    description: String,
    diff_summary: serde_json::Value,
) -> Fact {
    Fact {
        id: format!("{SOURCE}:change:{uid}:{now}"),
        kind: KIND.to_string(),
        source: SOURCE.to_string(),
        resource_id: target.to_string(),
        resource_type: "ChangeEvent".to_string(),
        timestamp: now,
        attributes_json: serde_json::json!({
            "change_type": change_type,
            "target_resource_id": target,
            "source": CHANGE_SOURCE,
            "changed_by": "k8s",
            "description": description,
            "diff_summary": diff_summary,
            "cluster_id": "",
        })
        .to_string(),
    }
}

#[cfg(test)]
mod tests;
