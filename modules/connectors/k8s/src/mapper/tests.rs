//! mapper 纯函数单测 -- canned K8s API JSON fixtures(host target,CI-safe)。

use super::*;
use serde_json::json;

/// 找某 resource_id 的 fact(任意 kind)。
fn find<'a>(facts: &'a [Fact], rid: &str) -> Option<&'a Fact> {
    facts.iter().find(|f| f.resource_id == rid)
}

/// 找某 resource_id 且指定 kind 的 fact。
fn find_kind<'a>(facts: &'a [Fact], rid: &str, kind: &str) -> Option<&'a Fact> {
    facts.iter().find(|f| f.resource_id == rid && f.kind == kind)
}

/// 取某 fact 的 attributes 字段。
fn attr(f: &Fact, key: &str) -> serde_json::Value {
    let v: serde_json::Value = serde_json::from_str(&f.attributes_json).unwrap();
    v.get(key).cloned().unwrap_or(serde_json::Value::Null)
}

fn base_input() -> ClusterInput {
    ClusterInput {
        cluster: "vm".to_string(),
        namespace: "otel-demo".to_string(),
        now: 1_700_000_000,
        nodes: json!({ "items": [
            { "metadata": { "name": "vm1" }, "status": { "conditions": [ { "type": "Ready", "status": "True" } ] } },
            { "metadata": { "name": "vm2" }, "status": { "conditions": [ { "type": "Ready", "status": "False" } ] } }
        ]}),
        deployments: json!({ "items": [
            { "metadata": { "name": "frontend" } },
            { "metadata": { "name": "cartservice" } }
        ]}),
        replicasets: json!({ "items": [
            { "metadata": { "name": "frontend-abc", "ownerReferences": [ { "kind": "Deployment", "name": "frontend" } ] } },
            { "metadata": { "name": "cartservice-xyz", "ownerReferences": [ { "kind": "Deployment", "name": "cartservice" } ] } }
        ]}),
        pods: json!({ "items": [
            // healthy: Running + ready;labels app=frontend;volumes configMap feature-flags + envFrom configMap order-config
            { "metadata": { "name": "frontend-abc-1", "labels": { "app": "frontend" },
                            "ownerReferences": [ { "kind": "ReplicaSet", "name": "frontend-abc" } ] },
              "spec": { "nodeName": "vm1",
                        "volumes": [ { "name": "flags", "configMap": { "name": "feature-flags" } } ],
                        "containers": [ { "name": "frontend", "envFrom": [ { "configMapRef": { "name": "order-config" } } ] } ] },
              "status": { "phase": "Running", "containerStatuses": [ { "name": "frontend", "ready": true } ] } },
            // critical: crashloop;labels app=cartservice;volumes secret order-db
            { "metadata": { "name": "cartservice-xyz-1", "labels": { "app": "cartservice" },
                            "ownerReferences": [ { "kind": "ReplicaSet", "name": "cartservice-xyz" } ] },
              "spec": { "nodeName": "vm2",
                        "volumes": [ { "name": "db", "secret": { "secretName": "order-db" } } ] },
              "status": { "phase": "Running", "containerStatuses": [ { "name": "cart", "ready": false, "state": { "waiting": { "reason": "CrashLoopBackOff" } } } ] } },
            // dangling owner (rs not found) -> parent = namespace; phase Pending -> warning
            { "metadata": { "name": "orphan-1", "labels": { "app": "orphan" },
                            "ownerReferences": [ { "kind": "ReplicaSet", "name": "ghost-rs" } ] },
              "spec": { "nodeName": "vm3" },
              "status": { "phase": "Pending" } }
        ]}),
        services: json!({ "items": [
            { "metadata": { "name": "frontend" },
              "spec": { "selector": { "app": "frontend" } } }
        ]}),
        configmaps: json!({ "items": [
            { "metadata": { "name": "feature-flags" }, "data": { "flag1": "true" } },
            { "metadata": { "name": "order-config" }, "data": { "k1": "v1", "k2": "v2" } }
        ]}),
        secrets: json!({ "items": [
            { "metadata": { "name": "order-db" }, "data": { "password": "cGFzcw==" } }
        ]}),
    }
}

#[test]
fn builds_full_hierarchy_with_parents() {
    let facts = map_cluster(&base_input());

    // Cluster 无 parent
    let cluster = find(&facts, "cluster:vm").expect("cluster");
    assert_eq!(cluster.resource_type, "Cluster");
    assert_eq!(attr(cluster, "parent_resource_id"), serde_json::Value::Null);

    // Node -> cluster
    assert_eq!(attr(find(&facts, "node:vm:vm1").unwrap(), "parent_resource_id"), "cluster:vm");
    // Namespace -> cluster
    assert_eq!(attr(find(&facts, "ns:vm:otel-demo").unwrap(), "parent_resource_id"), "cluster:vm");
    // Deployment -> namespace
    assert_eq!(attr(find(&facts, "deploy:vm:otel-demo:frontend").unwrap(), "parent_resource_id"), "ns:vm:otel-demo");
    // Service -> namespace
    assert_eq!(attr(find(&facts, "service:vm:otel-demo:frontend").unwrap(), "parent_resource_id"), "ns:vm:otel-demo");
    // ConfigMap -> namespace
    assert_eq!(attr(find(&facts, "cm:vm:otel-demo:feature-flags").unwrap(), "parent_resource_id"), "ns:vm:otel-demo");
    // Secret -> namespace
    assert_eq!(attr(find(&facts, "secret:vm:otel-demo:order-db").unwrap(), "parent_resource_id"), "ns:vm:otel-demo");
}

#[test]
fn pod_parent_follows_owner_chain_to_deployment() {
    let facts = map_cluster(&base_input());
    // frontend-abc-1 -> rs frontend-abc -> deploy frontend
    let pod = find(&facts, "pod:vm:otel-demo:frontend-abc-1").unwrap();
    assert_eq!(attr(pod, "parent_resource_id"), "deploy:vm:otel-demo:frontend");
    assert_eq!(attr(pod, "node"), "node:vm:vm1");
}

#[test]
fn pod_with_dangling_owner_falls_back_to_namespace() {
    let facts = map_cluster(&base_input());
    let pod = find(&facts, "pod:vm:otel-demo:orphan-1").unwrap();
    assert_eq!(attr(pod, "parent_resource_id"), "ns:vm:otel-demo");
}

#[test]
fn pod_health_normal_warning_critical() {
    let facts = map_cluster(&base_input());
    // Running + ready -> normal
    assert_eq!(attr(find(&facts, "pod:vm:otel-demo:frontend-abc-1").unwrap(), "health_status"), "normal");
    // CrashLoopBackOff -> critical
    assert_eq!(attr(find(&facts, "pod:vm:otel-demo:cartservice-xyz-1").unwrap(), "health_status"), "critical");
    assert_eq!(attr(find(&facts, "pod:vm:otel-demo:cartservice-xyz-1").unwrap(), "risk_level"), "high");
    // Pending -> warning
    assert_eq!(attr(find(&facts, "pod:vm:otel-demo:orphan-1").unwrap(), "health_status"), "warning");
}

#[test]
fn node_health_from_ready_condition() {
    let facts = map_cluster(&base_input());
    assert_eq!(attr(find(&facts, "node:vm:vm1").unwrap(), "health_status"), "normal");
    assert_eq!(attr(find(&facts, "node:vm:vm2").unwrap(), "health_status"), "critical");
}

#[test]
fn configmap_and_secret_store_data_keys_only() {
    let facts = map_cluster(&base_input());
    // ConfigMap data_keys 排序后逗号分隔
    let cm = find(&facts, "cm:vm:otel-demo:order-config").unwrap();
    assert_eq!(attr(cm, "data_keys"), "k1,k2");
    // Secret 只存 data_keys,不存 data 值
    let sec = find(&facts, "secret:vm:otel-demo:order-db").unwrap();
    assert_eq!(attr(sec, "data_keys"), "password");
    let sec_attrs: serde_json::Value = serde_json::from_str(&sec.attributes_json).unwrap();
    assert!(sec_attrs.get("data").is_none(), "Secret 不得存 data 值");
}

#[test]
fn all_facts_carry_k8s_source() {
    let facts = map_cluster(&base_input());
    assert!(facts.iter().all(|f| f.source == "k8s"));
    // 节点:1 cluster + 2 node + 1 ns + 2 deploy + 3 pod + 1 svc + 2 cm + 1 secret = 13
    let nodes = facts.iter().filter(|f| f.kind == "topology-node").count();
    assert_eq!(nodes, 13);
    // 边:3 SCHEDULED_ON + 2 USES(cm)+ 1 USES(secret)+ 1 ROUTES_TO = 7
    let edges = facts.iter().filter(|f| f.kind == "topology-edge").count();
    assert_eq!(edges, 7);
}

#[test]
fn scheduled_on_edge_per_pod_with_nodename() {
    let facts = map_cluster(&base_input());
    // 3 个 pod 都有 nodeName -> 3 条 SCHEDULED_ON
    let sched: Vec<&Fact> = facts
        .iter()
        .filter(|f| f.kind == "topology-edge" && attr(f, "edge_type") == "SCHEDULED_ON")
        .collect();
    assert_eq!(sched.len(), 3);
    // frontend-abc-1 调度在 vm1
    assert!(find_kind(
        &facts,
        "edge:SCHEDULED_ON:pod:vm:otel-demo:frontend-abc-1->node:vm:vm1",
        "topology-edge"
    )
    .is_some());
}

#[test]
fn uses_edge_for_configmap_and_secret_refs() {
    let facts = map_cluster(&base_input());
    // frontend-abc-1 USES cm:feature-flags(volumes)+ cm:order-config(envFrom)
    assert!(find_kind(
        &facts,
        "edge:USES:pod:vm:otel-demo:frontend-abc-1->cm:vm:otel-demo:feature-flags",
        "topology-edge"
    )
    .is_some());
    assert!(find_kind(
        &facts,
        "edge:USES:pod:vm:otel-demo:frontend-abc-1->cm:vm:otel-demo:order-config",
        "topology-edge"
    )
    .is_some());
    // cartservice-xyz-1 USES secret:order-db(volumes)
    assert!(find_kind(
        &facts,
        "edge:USES:pod:vm:otel-demo:cartservice-xyz-1->secret:vm:otel-demo:order-db",
        "topology-edge"
    )
    .is_some());
    // USES 边总数 = 2 cm + 1 secret = 3
    let uses = facts
        .iter()
        .filter(|f| f.kind == "topology-edge" && attr(f, "edge_type") == "USES")
        .count();
    assert_eq!(uses, 3);
}

#[test]
fn routes_to_edge_matches_selector() {
    let facts = map_cluster(&base_input());
    // service frontend selector {app:frontend} 只匹配 frontend-abc-1(不匹配 cartservice/orphan)
    let routes: Vec<&Fact> = facts
        .iter()
        .filter(|f| f.kind == "topology-edge" && attr(f, "edge_type") == "ROUTES_TO")
        .collect();
    assert_eq!(routes.len(), 1);
    assert_eq!(attr(routes[0], "source"), "service:vm:otel-demo:frontend");
    assert_eq!(attr(routes[0], "target"), "pod:vm:otel-demo:frontend-abc-1");
}

#[test]
fn empty_or_null_lists_yield_only_cluster_and_namespace() {
    let input = ClusterInput {
        cluster: "vm".to_string(),
        namespace: "otel-demo".to_string(),
        now: 1,
        nodes: serde_json::Value::Null,
        deployments: json!({}),
        replicasets: json!({ "items": [] }),
        pods: serde_json::Value::Null,
        services: json!({ "items": [] }),
        configmaps: serde_json::Value::Null,
        secrets: json!({ "items": [] }),
    };
    let facts = map_cluster(&input);
    // 只剩 cluster + namespace(无 edge fact)
    assert_eq!(facts.len(), 2);
    assert!(find(&facts, "cluster:vm").is_some());
    assert!(find(&facts, "ns:vm:otel-demo").is_some());
    assert!(facts.iter().all(|f| f.kind == "topology-node"));
}
