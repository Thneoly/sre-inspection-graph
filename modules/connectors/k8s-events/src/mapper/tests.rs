//! mapper 纯函数单测 —— 移植 reference `TestK8sEventConnector`(4 例)+ 补充。host target,CI-safe。

use super::*;
use module_sdk::Fact;

fn cfg() -> Cfg {
    Cfg::new("vm-cluster", "otel-demo", 1_700_000_000)
}

fn event(uid: &str, reason: &str, kind: &str, name: &str, msg: &str) -> Event {
    Event {
        metadata: Meta { uid: uid.to_string() },
        reason: reason.to_string(),
        message: msg.to_string(),
        involved_object: InvolvedObject { kind: kind.to_string(), name: name.to_string() },
    }
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
