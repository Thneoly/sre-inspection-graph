//! k8s mapper —— **纯函数核心**(不在 `cfg(wasm32)` 内,host `cargo test` 可直接测)。
//!
//! 吃 K8s API 的 list 响应(`serde_json::Value`,形如 `{"items":[...]}`),产
//! canonical [`Fact`](`module_sdk::Fact`,7 字段 host 镜像)。对照 reference
//! `app/datasource/connectors/k8s_mapper.py` 的稳定 ID + 关系语义,但只做 v0
//! 拓扑层级(不做 ConfigMap/Secret 关联、不做 component/middleware owner 抽取)。
//!
//! ## resource_id schema(与 k8s-mini 对齐 + 新增 deploy)
//!
//! - `cluster:{cluster}`                       Cluster(无 parent)
//! - `node:{cluster}:{node}`                    Node      → cluster
//! - `ns:{cluster}:{ns}`                        Namespace → cluster
//! - `deploy:{cluster}:{ns}:{name}`             Deployment→ namespace
//! - `pod:{cluster}:{ns}:{name}`                Pod       → deployment(owner 链)否则 namespace
//! - `service:{cluster}:{ns}:{name}`            Service   → namespace
//!
//! 父子关系写进 `attributes_json.parent_resource_id` —— host 侧
//! `engine_core::facts_to_graph` / `engine_identity::resolve` 据此建 `CONTAINS` 边。
//!
//! ## Pod owner 链(对照 reference)
//!
//! Pod.ownerReferences[kind=ReplicaSet].name → 在 ReplicaSet 列表里查
//! ReplicaSet.ownerReferences[kind=Deployment].name → Deployment。落空(裸 Pod /
//! 找不到)退化 parent = Namespace。
//!
//! ## health 推导
//!
//! Pod:phase + containerStatuses.ready →
//! - `normal`  : phase=Running 且所有容器 ready
//! - `warning` : phase=Running 但有容器未 ready,或 phase=Pending
//! - `critical`: phase ∈ {Failed, Unknown},或任一容器 waiting=CrashLoopBackOff
//!
//! 其余资源默认 `normal`。`risk_level` 由 health 映射(critical→high / warning→medium / normal→low)。

use serde_json::{json, Map, Value};

use module_sdk::Fact;

/// 一次 sync 的全部 K8s list 响应 + 上下文。
pub struct ClusterInput {
    /// 集群逻辑名(resource_id 命名空间用)。
    pub cluster: String,
    /// 目标 namespace。
    pub namespace: String,
    /// 时间戳(Unix 秒)—— fact.id / fact.timestamp 用。host 由 clock capability 给。
    pub now: u64,
    /// `GET /api/v1/nodes` 响应。
    pub nodes: Value,
    /// `GET /apis/apps/v1/namespaces/{ns}/deployments` 响应。
    pub deployments: Value,
    /// `GET /apis/apps/v1/namespaces/{ns}/replicasets` 响应。
    pub replicasets: Value,
    /// `GET /api/v1/namespaces/{ns}/pods` 响应。
    pub pods: Value,
    /// `GET /api/v1/namespaces/{ns}/services` 响应。
    pub services: Value,
}

const SOURCE: &str = "k8s";
const KIND: &str = "topology-node";

/// 把一批 K8s list 响应映射成 topology Fact 列表。
pub fn map_cluster(input: &ClusterInput) -> Vec<Fact> {
    let c = input.cluster.as_str();
    let ns = input.namespace.as_str();
    let now = input.now;
    let mut facts: Vec<Fact> = Vec::new();

    // 1) Cluster
    facts.push(node_fact(
        now,
        &format!("cluster:{c}"),
        "Cluster",
        json!({ "cluster": c, "health_status": "normal", "risk_level": "low" }),
    ));

    // 2) Nodes(集群级,不限 namespace)
    for n in items(&input.nodes) {
        let Some(name) = meta_name(n) else { continue };
        facts.push(node_fact(
            now,
            &format!("node:{c}:{name}"),
            "Node",
            json!({
                "cluster": c,
                "name": name,
                "parent_resource_id": format!("cluster:{c}"),
                "health_status": node_health(n),
                "risk_level": risk_from_health(node_health(n)),
            }),
        ));
    }

    // 3) Namespace
    facts.push(node_fact(
        now,
        &format!("ns:{c}:{ns}"),
        "Namespace",
        json!({
            "cluster": c,
            "namespace": ns,
            "parent_resource_id": format!("cluster:{c}"),
            "health_status": "normal",
            "risk_level": "low",
        }),
    ));

    // 4) Deployments
    for d in items(&input.deployments) {
        let Some(name) = meta_name(d) else { continue };
        facts.push(node_fact(
            now,
            &format!("deploy:{c}:{ns}:{name}"),
            "Deployment",
            json!({
                "cluster": c,
                "namespace": ns,
                "name": name,
                "parent_resource_id": format!("ns:{c}:{ns}"),
                "health_status": "normal",
                "risk_level": "low",
            }),
        ));
    }

    // 5) ReplicaSet name → Deployment name(owner 链中间层,不入图)
    let rs_to_deploy = index_rs_to_deploy(&input.replicasets);

    // 6) Pods —— parent = Deployment(经 rs 链)否则 Namespace
    for p in items(&input.pods) {
        let Some(name) = meta_name(p) else { continue };
        let parent = pod_parent(p, &rs_to_deploy, c, ns);
        let health = pod_health(p);
        let node_name = p
            .get("spec")
            .and_then(|s| s.get("nodeName"))
            .and_then(Value::as_str)
            .unwrap_or("");
        facts.push(node_fact(
            now,
            &format!("pod:{c}:{ns}:{name}"),
            "Pod",
            json!({
                "cluster": c,
                "namespace": ns,
                "name": name,
                "node": if node_name.is_empty() { Value::Null } else { json!(format!("node:{c}:{node_name}")) },
                "phase": p.get("status").and_then(|s| s.get("phase")).and_then(Value::as_str).unwrap_or(""),
                "parent_resource_id": parent,
                "health_status": health,
                "risk_level": risk_from_health(health),
            }),
        ));
    }

    // 7) Services —— parent = Namespace
    for s in items(&input.services) {
        let Some(name) = meta_name(s) else { continue };
        facts.push(node_fact(
            now,
            &format!("service:{c}:{ns}:{name}"),
            "Service",
            json!({
                "cluster": c,
                "namespace": ns,
                "name": name,
                "parent_resource_id": format!("ns:{c}:{ns}"),
                "health_status": "normal",
                "risk_level": "low",
            }),
        ));
    }

    facts
}

/// 构造一条 topology-node Fact。id 含 resource_id + ts,避免跨轮 / 跨源撞 ID。
fn node_fact(now: u64, resource_id: &str, resource_type: &str, attrs: Value) -> Fact {
    Fact {
        id: format!("{SOURCE}:{resource_id}:{now}"),
        kind: KIND.to_string(),
        source: SOURCE.to_string(),
        resource_id: resource_id.to_string(),
        resource_type: resource_type.to_string(),
        timestamp: now,
        attributes_json: attrs.to_string(),
    }
}

/// `{"items":[...]}` → items slice(非数组返空)。
fn items(list: &Value) -> &[Value] {
    list.get("items")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// metadata.name。
fn meta_name(obj: &Value) -> Option<&str> {
    obj.get("metadata")?.get("name")?.as_str()
}

/// ReplicaSet name → 拥有它的 Deployment name。
fn index_rs_to_deploy(replicasets: &Value) -> Map<String, Value> {
    let mut m = Map::new();
    for rs in items(replicasets) {
        let Some(rs_name) = meta_name(rs) else { continue };
        if let Some(deploy) = owner_name(rs, "Deployment") {
            m.insert(rs_name.to_string(), Value::String(deploy.to_string()));
        }
    }
    m
}

/// Pod → parent resource_id:Deployment(经 rs 链)否则 Namespace。
fn pod_parent(pod: &Value, rs_to_deploy: &Map<String, Value>, c: &str, ns: &str) -> String {
    if let Some(rs_name) = owner_name(pod, "ReplicaSet") {
        if let Some(deploy) = rs_to_deploy.get(rs_name).and_then(Value::as_str) {
            return format!("deploy:{c}:{ns}:{deploy}");
        }
    }
    format!("ns:{c}:{ns}")
}

/// metadata.ownerReferences 里第一个匹配 kind 的 name。
fn owner_name<'a>(obj: &'a Value, kind: &str) -> Option<&'a str> {
    obj.get("metadata")?
        .get("ownerReferences")?
        .as_array()?
        .iter()
        .find(|o| o.get("kind").and_then(Value::as_str) == Some(kind))
        .and_then(|o| o.get("name"))
        .and_then(Value::as_str)
}

/// Pod health:phase + containerStatuses.ready / waiting reason。
fn pod_health(pod: &Value) -> &'static str {
    let status = pod.get("status");
    let phase = status
        .and_then(|s| s.get("phase"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let cs = status
        .and_then(|s| s.get("containerStatuses"))
        .and_then(Value::as_array);

    // CrashLoopBackOff(任一容器 waiting.reason)→ critical
    if let Some(cs) = cs {
        let crashloop = cs.iter().any(|c| {
            c.get("state")
                .and_then(|st| st.get("waiting"))
                .and_then(|w| w.get("reason"))
                .and_then(Value::as_str)
                == Some("CrashLoopBackOff")
        });
        if crashloop {
            return "critical";
        }
    }

    match phase {
        "Failed" | "Unknown" => "critical",
        "Pending" => "warning",
        "Running" => {
            let all_ready = cs
                .map(|cs| {
                    !cs.is_empty()
                        && cs
                            .iter()
                            .all(|c| c.get("ready").and_then(Value::as_bool) == Some(true))
                })
                .unwrap_or(false);
            if all_ready {
                "normal"
            } else {
                "warning"
            }
        }
        "Succeeded" => "normal",
        _ => "warning",
    }
}

/// Node health:status.conditions 里 Ready=True → normal,否则 critical。
fn node_health(node: &Value) -> &'static str {
    let ready = node
        .get("status")
        .and_then(|s| s.get("conditions"))
        .and_then(Value::as_array)
        .map(|conds| {
            conds.iter().any(|c| {
                c.get("type").and_then(Value::as_str) == Some("Ready")
                    && c.get("status").and_then(Value::as_str) == Some("True")
            })
        })
        .unwrap_or(false);
    if ready {
        "normal"
    } else {
        "critical"
    }
}

/// health → risk_level 映射(给 Cytoscape border 配色)。
fn risk_from_health(health: &str) -> &'static str {
    match health {
        "critical" => "high",
        "warning" => "medium",
        _ => "low",
    }
}

#[cfg(test)]
mod tests;
