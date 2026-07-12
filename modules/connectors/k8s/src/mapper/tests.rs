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
        release_prefix: "otel-demo".to_string(),
        nodes: json!({ "items": [
            { "metadata": { "name": "vm1" }, "status": { "conditions": [ { "type": "Ready", "status": "True" } ] } },
            { "metadata": { "name": "vm2" }, "status": { "conditions": [ { "type": "Ready", "status": "False" } ] } }
        ]}),
        deployments: json!({ "items": [
            { "metadata": { "name": "frontend" } },
            { "metadata": { "name": "cartservice" } },
            // middleware:otel-demo-valkey -> Redis(strip prefix 后 "valkey" 命中)
            { "metadata": { "name": "otel-demo-valkey" } }
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
    // Application -> namespace(Phase 3.8)
    assert_eq!(attr(find(&facts, "app:vm:otel-demo:otel-demo").unwrap(), "parent_resource_id"), "ns:vm:otel-demo");
    // ApplicationComponent -> application(派生 CONTAINS)
    assert_eq!(attr(find(&facts, "comp:vm:otel-demo:frontend").unwrap(), "parent_resource_id"), "app:vm:otel-demo:otel-demo");
    assert_eq!(attr(find(&facts, "comp:vm:otel-demo:cart").unwrap(), "parent_resource_id"), "app:vm:otel-demo:otel-demo");
    // Deployment -> namespace
    assert_eq!(attr(find(&facts, "deploy:vm:otel-demo:frontend").unwrap(), "parent_resource_id"), "ns:vm:otel-demo");
    // Middleware(Redis)-> namespace
    assert_eq!(attr(find(&facts, "redis:vm:otel-demo:otel-demo-valkey").unwrap(), "parent_resource_id"), "ns:vm:otel-demo");
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
    // 节点:1 cluster + 2 node + 1 ns + 1 app + 3 deploy + 2 comp + 1 mw + 3 pod + 1 svc + 2 cm + 1 secret = 18
    let nodes = facts.iter().filter(|f| f.kind == "topology-node").count();
    assert_eq!(nodes, 18);
    // 边:SCHEDULED_ON 3 + USES 3 + ROUTES_TO 1 + BELONGS_TO(comp->app 2 + deploy->comp 2)
    //     + DEPLOYED_AS(comp->deploy 2 + mw->deploy 1)= 14
    let edges = facts.iter().filter(|f| f.kind == "topology-edge").count();
    assert_eq!(edges, 14);
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
    assert!(find_kind(
        &facts,
        "edge:USES:pod:vm:otel-demo:cartservice-xyz-1->secret:vm:otel-demo:order-db",
        "topology-edge"
    )
    .is_some());
    let uses = facts
        .iter()
        .filter(|f| f.kind == "topology-edge" && attr(f, "edge_type") == "USES")
        .count();
    assert_eq!(uses, 3);
}

#[test]
fn routes_to_edge_matches_selector() {
    let facts = map_cluster(&base_input());
    let routes: Vec<&Fact> = facts
        .iter()
        .filter(|f| f.kind == "topology-edge" && attr(f, "edge_type") == "ROUTES_TO")
        .collect();
    assert_eq!(routes.len(), 1);
    assert_eq!(attr(routes[0], "source"), "service:vm:otel-demo:frontend");
    assert_eq!(attr(routes[0], "target"), "pod:vm:otel-demo:frontend-abc-1");
}

// ===== Phase 3.8:Application/Component/Middleware 层 =====

#[test]
fn normalize_component_name_cases() {
    // strip release prefix + 砍 service 后缀 + 拆混淆名(对照 reference)
    assert_eq!(normalize_component_name("otel-demo-cartservice", "otel-demo"), "cart");
    assert_eq!(normalize_component_name("otel-demo-frontend", "otel-demo"), "frontend");
    assert_eq!(normalize_component_name("otel-demo-frontendproxy", "otel-demo"), "frontend-proxy");
    assert_eq!(normalize_component_name("otel-demo-recommendationservice", "otel-demo"), "recommendation");
    assert_eq!(normalize_component_name("otel-demo-frauddetectionservice", "otel-demo"), "fraud-detection");
    assert_eq!(normalize_component_name("otel-demo-productcatalogservice", "otel-demo"), "product-catalog");
    // 无 release 前缀也不挂
    assert_eq!(normalize_component_name("frontend", "otel-demo"), "frontend");
    // 短名不砍 service(避免吃光)
    assert_eq!(normalize_component_name("ad", "otel-demo"), "ad");
}

#[test]
fn detect_middleware_cases() {
    // 整名匹配
    assert_eq!(detect_middleware("otel-demo-valkey", "otel-demo"), Some(("Redis", "redis")));
    assert_eq!(detect_middleware("otel-demo-kafka", "otel-demo"), Some(("Kafka", "kafka")));
    assert_eq!(detect_middleware("otel-demo-postgres", "otel-demo"), Some(("PostgreSQL", "postgres")));
    // 普通业务服务不是中间件
    assert_eq!(detect_middleware("otel-demo-frontend", "otel-demo"), None);
    assert_eq!(detect_middleware("otel-demo-cartservice", "otel-demo"), None);
}

#[test]
fn is_infra_cases() {
    assert!(is_infra("otel-demo-loadgenerator", "otel-demo"));
    assert!(is_infra("otel-demo-jaeger", "otel-demo"));
    // 普通业务不是 infra
    assert!(!is_infra("otel-demo-frontend", "otel-demo"));
    assert!(!is_infra("otel-demo-cartservice", "otel-demo"));
}

#[test]
fn application_component_middleware_layer() {
    let facts = map_cluster(&base_input());

    // Application node
    assert_eq!(
        find(&facts, "app:vm:otel-demo:otel-demo").unwrap().resource_type,
        "Application"
    );

    // ApplicationComponent:frontend(无 service 后缀)+ cart(砍 service)
    assert_eq!(
        find(&facts, "comp:vm:otel-demo:frontend").unwrap().resource_type,
        "ApplicationComponent"
    );
    assert_eq!(
        attr(find(&facts, "comp:vm:otel-demo:cart").unwrap(), "name"),
        "cart"
    );

    // Middleware:otel-demo-valkey -> Redis
    let mw = find(&facts, "redis:vm:otel-demo:otel-demo-valkey").unwrap();
    assert_eq!(mw.resource_type, "Redis");

    // DEPLOYED_AS:comp -> deploy(frontend + cart)
    assert!(find_kind(
        &facts,
        "edge:DEPLOYED_AS:comp:vm:otel-demo:frontend->deploy:vm:otel-demo:frontend",
        "topology-edge"
    )
    .is_some());
    assert!(find_kind(
        &facts,
        "edge:DEPLOYED_AS:comp:vm:otel-demo:cart->deploy:vm:otel-demo:cartservice",
        "topology-edge"
    )
    .is_some());
    // DEPLOYED_AS:mw -> deploy(valkey)
    assert!(find_kind(
        &facts,
        "edge:DEPLOYED_AS:redis:vm:otel-demo:otel-demo-valkey->deploy:vm:otel-demo:otel-demo-valkey",
        "topology-edge"
    )
    .is_some());

    // BELONGS_TO:deploy -> comp(反向,action BELONGS_TO forward 命中)
    assert!(find_kind(
        &facts,
        "edge:BELONGS_TO:deploy:vm:otel-demo:frontend->comp:vm:otel-demo:frontend",
        "topology-edge"
    )
    .is_some());
    // BELONGS_TO:comp -> app(反向)
    assert!(find_kind(
        &facts,
        "edge:BELONGS_TO:comp:vm:otel-demo:frontend->app:vm:otel-demo:otel-demo",
        "topology-edge"
    )
    .is_some());

    // CONTAINS(app -> comp)由 comp.parent_resource_id=app 派生(非 edge fact)
    assert!(
        find_kind(&facts, "edge:CONTAINS:app:vm:otel-demo:otel-demo->comp:vm:otel-demo:frontend", "topology-edge")
            .is_none(),
        "CONTAINS(app->comp)应派生(非 edge fact)"
    );

    // BELONGS_TO 边总数:comp->app 2 + deploy->comp 2 = 4
    let belongs = facts
        .iter()
        .filter(|f| f.kind == "topology-edge" && attr(f, "edge_type") == "BELONGS_TO")
        .count();
    assert_eq!(belongs, 4);
    // DEPLOYED_AS 边总数:comp->deploy 2 + mw->deploy 1 = 3
    let deployed = facts
        .iter()
        .filter(|f| f.kind == "topology-edge" && attr(f, "edge_type") == "DEPLOYED_AS")
        .count();
    assert_eq!(deployed, 3);
}

#[test]
fn empty_or_null_lists_yield_only_cluster_namespace_application() {
    let input = ClusterInput {
        cluster: "vm".to_string(),
        namespace: "otel-demo".to_string(),
        now: 1,
        release_prefix: "otel-demo".to_string(),
        nodes: serde_json::Value::Null,
        deployments: json!({}),
        replicasets: json!({ "items": [] }),
        pods: serde_json::Value::Null,
        services: json!({ "items": [] }),
        configmaps: serde_json::Value::Null,
        secrets: json!({ "items": [] }),
    };
    let facts = map_cluster(&input);
    // cluster + namespace + application(无 deploy 故无 component)
    assert_eq!(facts.len(), 3);
    assert!(find(&facts, "cluster:vm").is_some());
    assert!(find(&facts, "ns:vm:otel-demo").is_some());
    assert!(find(&facts, "app:vm:otel-demo:otel-demo").is_some());
    assert!(facts.iter().all(|f| f.kind == "topology-node"));
}
