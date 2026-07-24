//! K8s Event → `kind="change"` Fact 的纯映射(host target 可单测)。
//!
//! 对照 reference `k8s_event_connector.py`:poll `GET /api/v1/namespaces/{ns}/events`,
//! 只挑 `INTERESTING_REASONS`(`ScalingReplicaSet`/`SuccessfulRescale` → `deployment_rolled`),
//! 把 `involvedObject`(Deployment / ReplicaSet[strip hash] / Pod)映成 target resource_id,
//! 产 ChangeEvent 载荷(写入 `kind="change"` Fact 的 attributes_json;desktop run_sync 路由
//! 到 `record_change`)。**不产节点/边/metric/alert**(reference 行为)。UID dedup + 首次 baseline
//! 是有状态逻辑,在 `lib.rs`(guest static);本文件只做无状态映射。

use module_sdk::Fact;
use serde::Deserialize;

/// source 标识(Fact.source = connector 名)。
pub const SOURCE: &str = "k8s-events";
/// ChangeRequest.source(record_change 校验 -> Source enum;对照 reference `k8s_api`)。
pub const CHANGE_SOURCE: &str = "k8s_api";
const KIND: &str = "change";

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
    reason: String,
    message: String,
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
