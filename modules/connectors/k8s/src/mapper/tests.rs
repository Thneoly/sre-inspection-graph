//! mapper 纯函数单测 —— canned K8s API JSON fixtures(host target,CI-safe)。

use super::*;
use serde_json::json;

/// 找某 resource_id 的 fact。
fn find<'a>(facts: &'a [Fact], rid: &str) -> Option<&'a Fact> {
    facts.iter().find(|f| f.resource_id == rid)
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
            // healthy: Running + ready
            { "metadata": { "name": "frontend-abc-1", "ownerReferences": [ { "kind": "ReplicaSet", "name": "frontend-abc" } ] },
              "spec": { "nodeName": "vm1" },
              "status": { "phase": "Running", "containerStatuses": [ { "name": "frontend", "ready": true } ] } },
            // critical: crashloop
            { "metadata": { "name": "cartservice-xyz-1", "ownerReferences": [ { "kind": "ReplicaSet", "name": "cartservice-xyz" } ] },
              "spec": { "nodeName": "vm2" },
              "status": { "phase": "Running", "containerStatuses": [ { "name": "cart", "ready": false, "state": { "waiting": { "reason": "CrashLoopBackOff" } } } ] } },
            // dangling owner (rs not found) → parent = namespace; phase Pending → warning
            { "metadata": { "name": "orphan-1", "ownerReferences": [ { "kind": "ReplicaSet", "name": "ghost-rs" } ] },
              "spec": { "nodeName": "vm3" },
              "status": { "phase": "Pending" } }
        ]}),
        services: json!({ "items": [
            { "metadata": { "name": "frontend" } }
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

    // Node → cluster
    assert_eq!(attr(find(&facts, "node:vm:vm1").unwrap(), "parent_resource_id"), "cluster:vm");
    // Namespace → cluster
    assert_eq!(attr(find(&facts, "ns:vm:otel-demo").unwrap(), "parent_resource_id"), "cluster:vm");
    // Deployment → namespace
    assert_eq!(attr(find(&facts, "deploy:vm:otel-demo:frontend").unwrap(), "parent_resource_id"), "ns:vm:otel-demo");
    // Service → namespace
    assert_eq!(attr(find(&facts, "service:vm:otel-demo:frontend").unwrap(), "parent_resource_id"), "ns:vm:otel-demo");
}

#[test]
fn pod_parent_follows_owner_chain_to_deployment() {
    let facts = map_cluster(&base_input());
    // frontend-abc-1 → rs frontend-abc → deploy frontend
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
    // Running + ready → normal
    assert_eq!(attr(find(&facts, "pod:vm:otel-demo:frontend-abc-1").unwrap(), "health_status"), "normal");
    // CrashLoopBackOff → critical
    assert_eq!(attr(find(&facts, "pod:vm:otel-demo:cartservice-xyz-1").unwrap(), "health_status"), "critical");
    assert_eq!(attr(find(&facts, "pod:vm:otel-demo:cartservice-xyz-1").unwrap(), "risk_level"), "high");
    // Pending → warning
    assert_eq!(attr(find(&facts, "pod:vm:otel-demo:orphan-1").unwrap(), "health_status"), "warning");
}

#[test]
fn node_health_from_ready_condition() {
    let facts = map_cluster(&base_input());
    assert_eq!(attr(find(&facts, "node:vm:vm1").unwrap(), "health_status"), "normal");
    assert_eq!(attr(find(&facts, "node:vm:vm2").unwrap(), "health_status"), "critical");
}

#[test]
fn all_facts_are_topology_nodes_from_k8s_source() {
    let facts = map_cluster(&base_input());
    assert!(facts.iter().all(|f| f.kind == "topology-node" && f.source == "k8s"));
    // 1 cluster + 2 node + 1 ns + 2 deploy + 3 pod + 1 svc = 10
    assert_eq!(facts.len(), 10);
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
    };
    let facts = map_cluster(&input);
    // 只剩 cluster + namespace
    assert_eq!(facts.len(), 2);
    assert!(find(&facts, "cluster:vm").is_some());
    assert!(find(&facts, "ns:vm:otel-demo").is_some());
}
