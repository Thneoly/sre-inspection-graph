//! mapper 纯函数单测 —— 移植 reference `TestK8sEventConnector`(4 例)+ 补充。host target,CI-safe。

use super::*;
use module_sdk::Fact;

fn cfg() -> Cfg {
    Cfg::new("vm-cluster", "otel-demo", 1_700_000_000)
}

fn event(uid: &str, reason: &str, kind: &str, name: &str, msg: &str) -> Event {
    Event {
        metadata: Meta { uid: uid.to_string() },
        event_type: "Normal".to_string(),
        reason: reason.to_string(),
        message: msg.to_string(),
        last_timestamp: "2026-10-31T01:02:03Z".to_string(),
        count: 1,
        involved_object: InvolvedObject { kind: kind.to_string(), name: name.to_string() },
    }
}

/// Warning 版 event(alert 产线)。
fn warning_event(uid: &str, reason: &str, kind: &str, name: &str, msg: &str) -> Event {
    let mut ev = event(uid, reason, kind, name, msg);
    ev.event_type = "Warning".to_string();
    ev
}

fn attr_str(f: &Fact, key: &str) -> String {
    let m: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&f.attributes_json).unwrap();
    m.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

#[test]
fn deployment_scaled_maps_to_change() {
    let ev = event("u1", "ScalingReplicaSet", "Deployment", "frontend", "Scaled up replica set");
    let f = event_to_change_fact(&ev, &cfg()).expect("some fact");
    assert_eq!(f.kind, "change");
    assert_eq!(f.source, "k8s-events");
    assert_eq!(f.resource_type, "ChangeEvent");
    assert_eq!(attr_str(&f, "change_type"), "deployment_rolled");
    assert_eq!(attr_str(&f, "target_resource_id"), "deploy:vm-cluster:otel-demo:frontend");
    assert_eq!(attr_str(&f, "source"), "k8s_api"); // ChangeRequest source, not connector name
    assert_eq!(attr_str(&f, "changed_by"), "k8s");
    assert!(attr_str(&f, "description").contains("ScalingReplicaSet"));
    assert_eq!(f.resource_id, "deploy:vm-cluster:otel-demo:frontend");
}

#[test]
fn replicaset_strips_hash() {
    let ev = event("u2", "ScalingReplicaSet", "ReplicaSet", "frontend-87bbfc4c9", "");
    let f = event_to_change_fact(&ev, &cfg()).expect("some fact");
    assert_eq!(attr_str(&f, "target_resource_id"), "deploy:vm-cluster:otel-demo:frontend");
}

#[test]
fn replicaset_without_dash_kept_as_is() {
    let ev = event("u2b", "ScalingReplicaSet", "ReplicaSet", "plainrs", "");
    let f = event_to_change_fact(&ev, &cfg()).expect("some fact");
    assert_eq!(attr_str(&f, "target_resource_id"), "deploy:vm-cluster:otel-demo:plainrs");
}

#[test]
fn uninteresting_reason_skipped() {
    let ev = event("u3", "FailedScheduling", "Pod", "frontend-abc", "0/3 nodes available");
    assert!(event_to_change_fact(&ev, &cfg()).is_none());
}

#[test]
fn unknown_kind_skipped() {
    let ev = event("u4", "ScalingReplicaSet", "DaemonSet", "fluentd", "");
    assert!(event_to_change_fact(&ev, &cfg()).is_none());
}

#[test]
fn successful_rescale_also_maps() {
    let ev = event("u5", "SuccessfulRescale", "Deployment", "cart", "");
    let f = event_to_change_fact(&ev, &cfg()).expect("some fact");
    assert_eq!(attr_str(&f, "change_type"), "deployment_rolled");
    assert_eq!(attr_str(&f, "target_resource_id"), "deploy:vm-cluster:otel-demo:cart");
}

#[test]
fn pod_kind_maps_to_pod_id() {
    let ev = event("u6", "ScalingReplicaSet", "Pod", "cart-xyz", "");
    let f = event_to_change_fact(&ev, &cfg()).expect("some fact");
    assert_eq!(attr_str(&f, "target_resource_id"), "pod:vm-cluster:otel-demo:cart-xyz");
}

#[test]
fn message_truncated_to_200_chars() {
    let long = "x".repeat(500);
    let ev = event("u7", "ScalingReplicaSet", "Deployment", "frontend", &long);
    let f = event_to_change_fact(&ev, &cfg()).expect("some fact");
    // description = "ScalingReplicaSet: " + 200 chars -> 总长 18 + 200
    assert_eq!(attr_str(&f, "description").chars().count(), "ScalingReplicaSet: ".chars().count() + 200);
}

#[test]
fn diff_summary_carries_reason_kind_name() {
    let ev = event("u8", "ScalingReplicaSet", "Deployment", "frontend", "");
    let f = event_to_change_fact(&ev, &cfg()).unwrap();
    let m: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&f.attributes_json).unwrap();
    let ds = m.get("diff_summary").unwrap().as_object().unwrap();
    assert_eq!(ds["reason"], "ScalingReplicaSet");
    assert_eq!(ds["kind"], "Deployment");
    assert_eq!(ds["name"], "frontend");
}

// ---------- alert 产线(live 告警源)----------

#[test]
fn warning_event_maps_to_alert() {
    let ev = warning_event("w1", "BackOff", "Pod", "cart-abc", "Back-off restarting failed container");
    let f = event_to_alert_fact(&ev, &cfg()).expect("alert fact");
    assert_eq!(f.kind, "alert");
    assert_eq!(f.source, "k8s-events");
    assert_eq!(f.resource_type, "AlertEvent");
    // resource_id 用 event uid(不进图,facts_to_graph 只认 topology-*)
    assert_eq!(f.resource_id, "k8s-event:w1");
    assert_eq!(attr_str(&f, "alert_name"), "BackOff");
    assert_eq!(attr_str(&f, "resource_ref"), "pod:vm-cluster:otel-demo:cart-abc");
    assert_eq!(attr_str(&f, "severity"), "warning");
    assert_eq!(attr_str(&f, "status"), "firing");
    assert_eq!(attr_str(&f, "fired_at"), "2026-10-31T01:02:03Z");
    assert_eq!(attr_str(&f, "rule_id"), "k8s-event");
    assert_eq!(attr_str(&f, "cluster_id"), "vm-cluster");
    assert!(attr_str(&f, "summary").contains("Back-off"));
}

#[test]
fn normal_event_does_not_map_to_alert() {
    // change 白名单里的 Normal event 不产 alert(两产线互斥)
    let ev = event("n1", "ScalingReplicaSet", "Deployment", "frontend", "");
    assert!(event_to_alert_fact(&ev, &cfg()).is_none());
}

#[test]
fn unmappable_kind_falls_back_to_k8s_ref() {
    // involvedObject 是 Node(target_resource_id 映不出)→ k8s:{kind}:{ns}:{name} 兜底
    let ev = warning_event("w2", "NodeNotReady", "Node", "worker-1", "node is not ready");
    let f = event_to_alert_fact(&ev, &cfg()).expect("alert fact");
    assert_eq!(attr_str(&f, "resource_ref"), "k8s:Node:otel-demo:worker-1");
}

#[test]
fn warning_event_with_empty_message_uses_reason_as_summary() {
    let ev = warning_event("w3", "Unhealthy", "Pod", "web-1", "");
    let f = event_to_alert_fact(&ev, &cfg()).expect("alert fact");
    assert_eq!(attr_str(&f, "summary"), "Unhealthy");
}

#[test]
fn serde_parses_k8s_event_type_and_last_timestamp() {
    // camelCase(lastTimestamp)+ type 的反序列化端到端(真实 K8s 响应形态)
    let raw = serde_json::json!({
        "metadata": { "uid": "u9" },
        "type": "Warning",
        "reason": "FailedMount",
        "message": "Unable to attach volume",
        "lastTimestamp": "2026-10-31T08:00:00Z",
        "count": 3,
        "involvedObject": { "kind": "Pod", "name": "p1" }
    });
    let ev: Event = serde_json::from_value(raw).expect("parse");
    let f = event_to_alert_fact(&ev, &cfg()).expect("alert fact");
    assert_eq!(attr_str(&f, "alert_name"), "FailedMount");
    assert_eq!(attr_str(&f, "fired_at"), "2026-10-31T08:00:00Z");
}
