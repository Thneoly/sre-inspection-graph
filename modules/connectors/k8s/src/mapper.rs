//! k8s mapper -- **纯函数核心**(不在 `cfg(wasm32)` 内,host `cargo test` 可直接测)。
//!
//! 吃 K8s API 的 list 响应(`serde_json::Value`,形如 `{"items":[...]}`),产
//! canonical [`Fact`](`module_sdk::Fact`,7 字段 host 镜像)。对照 reference
//! `app/datasource/connectors/k8s_mapper.py` 的稳定 ID + 关系语义。
//!
//! ## resource_id schema(与 k8s-mini 对齐 + deploy/cm/secret)
//!
//! - `cluster:{cluster}`                       Cluster(无 parent)
//! - `node:{cluster}:{node}`                    Node      -> cluster
//! - `ns:{cluster}:{ns}`                        Namespace -> cluster
//! - `deploy:{cluster}:{ns}:{name}`             Deployment-> namespace
//! - `pod:{cluster}:{ns}:{name}`                Pod       -> deployment(owner 链)否则 namespace
//! - `service:{cluster}:{ns}:{name}`            Service   -> namespace
//! - `cm:{cluster}:{ns}:{name}`                 ConfigMap -> namespace
//! - `secret:{cluster}:{ns}:{name}`             Secret    -> namespace
//! - `container:{cluster}:{ns}:{pod}:{name}`    Container -> (无 parent;经 RUNS 边挂 pod)
//! - `image:{cluster}:{ns}:{image-ref}`        ContainerImage -> (无 parent;经 USES_IMAGE 边被 Container 引用)
//!
//! 父子关系写进 `attributes_json.parent_resource_id` -- host 侧
//! `engine_core::facts_to_graph` / `engine_identity::resolve` 据此建 `CONTAINS` 边。
//!
//! ## 富化边(Phase 3.7+,作 `topology-edge` fact,对照 reference k8s_mapper)
//!
//! - `SCHEDULED_ON`(pod -> node):`pod.spec.nodeName`
//! - `ROUTES_TO`(svc -> pod):`service.spec.selector` 匹配 `pod.metadata.labels`
//! - `USES`(pod -> configmap/secret):`spec.volumes`(configMap/secret)
//!   + `envFrom`(configMapRef/secretRef);**不解析 `env[].valueFrom`**(对齐 reference)
//! - `RUNS`(pod -> container,Phase 3.9):`pod.spec.containers[]` 每个容器产
//!   Container node(`container:{c}:{ns}:{pod}:{container}`)+ RUNS 边;ready /
//!   restart_count / health 从 `status.containerStatuses[]` 按 name 匹配取。
//!   对照 doc/03 REL-036(Pod RUNS Container)。**不产 initContainers**(v0)。
//! - `USES_IMAGE`(container -> image,Phase 5.2):每个 container 产 ContainerImage node
//!   (`image:{c}:{ns}:{image-ref}`,同镜像靠 resource_id 在 `facts_to_graph` 去重)+ USES_IMAGE 边。
//!   image-risk 视图据此反向找用此镜像的负载;语义区别于 config 的 `USES`(对齐 reference USES_IMAGE)。
//! - `EXPOSES`(svc -> deploy,Phase 3.9):`service.spec.selector` 匹配
//!   `deploy.spec.template.metadata.labels`。对照 doc/02 L2(Service EXPOSES
//!   Deployment,e006)。reference 未实现(static CSV 种子),属增量。
//!
//! Phase 3.8 另产 Application/Component/Middleware 层 + `CONTAINS`(app->comp
//! 派生)/ `DEPLOYED_AS`(comp/mw->deploy)/ `BELONGS_TO`(deploy->comp、
//! comp->app 反向)边。不产 `RUNS_IN` / Ingress `ROUTES_TO`(无 ingress 拉取)。
//!
//! ## Pod owner 链(对照 reference)
//!
//! Pod.ownerReferences[kind=ReplicaSet].name -> 在 ReplicaSet 列表里查
//! ReplicaSet.ownerReferences[kind=Deployment].name -> Deployment。落空(裸 Pod /
//! 找不到)退化 parent = Namespace。
//!
//! ## health 推导
//!
//! Pod:phase + containerStatuses.ready ->
//! - `normal`  : phase=Running 且所有容器 ready
//! - `warning` : phase=Running 但有容器未 ready,或 phase=Pending
//! - `critical`: phase ∈ {Failed, Unknown},或任一容器 waiting=CrashLoopBackOff
//!
//! 其余资源默认 `normal`。`risk_level` 由 health 映射(critical->high / warning->medium / normal->low)。
//! ConfigMap/Secret 不存 data 值,只存 `data_keys`(逗号分隔的 key 名)。

use serde_json::{json, Map, Value};

use module_sdk::Fact;

/// 一次 sync 的全部 K8s list 响应 + 上下文。
pub struct ClusterInput {
    /// 集群逻辑名(resource_id 命名空间用)。
    pub cluster: String,
    /// 目标 namespace。
    pub namespace: String,
    /// 时间戳(Unix 秒)-- fact.id / fact.timestamp 用。host 由 clock capability 给。
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
    /// `GET /api/v1/namespaces/{ns}/configmaps` 响应(Phase 3.7:ConfigMap node + Pod USES 边)。
    pub configmaps: Value,
    /// `GET /api/v1/namespaces/{ns}/secrets` 响应(Phase 3.7:Secret node + Pod USES 边;只取 name + data_keys)。
    pub secrets: Value,
    /// Helm release 前缀(Phase 3.8:Application 名 + component 名 strip 用,默认 "otel-demo")。
    pub release_prefix: String,
}

const SOURCE: &str = "k8s";
const KIND: &str = "topology-node";
const EDGE_KIND: &str = "topology-edge";

/// OTel Demo Helm release 默认前缀(对照 reference DEFAULT_RELEASE_PREFIX)。
pub const DEFAULT_RELEASE_PREFIX: &str = "otel-demo";

/// infra deploy(不挂 Application,对照 reference INFRA_NAMES)。
const INFRA_NAMES: &[&str] = &[
    "loadgenerator",
    "otelcol",
    "prometheus-server",
    "jaeger",
    "opensearch",
    "grafana",
    "kibana",
];

/// 中间件识别 -> (keyword, 节点 type, id 前缀)(对照 reference MIDDLEWARE_PATTERNS)。
const MIDDLEWARE_PATTERNS: &[(&str, &str, &str)] = &[
    ("valkey", "Redis", "redis"),
    ("redis", "Redis", "redis"),
    ("kafka", "Kafka", "kafka"),
    ("postgres", "PostgreSQL", "postgres"),
    ("postgresql", "PostgreSQL", "postgres"),
    ("mysql", "MySQL", "mysql"),
];

// normalize_component_name / strip_release_prefix / component_id 已抽到
// module_sdk::naming(k8s + jaeger 共享,防 resource_id 漂移)。见 modules/sdk/src/lib.rs。

/// 检测中间件 -> (node_type, id_prefix)(对照 reference detect_middleware)。
/// 先整名匹配,再 keyword 子串兜底。
fn detect_middleware(
    deploy_name: &str,
    release_prefix: &str,
) -> Option<(&'static str, &'static str)> {
    let short = module_sdk::strip_release_prefix(deploy_name, release_prefix);
    for &(kw, mw_type, prefix) in MIDDLEWARE_PATTERNS {
        if short == kw {
            return Some((mw_type, prefix));
        }
    }
    for &(kw, mw_type, prefix) in MIDDLEWARE_PATTERNS {
        if short.contains(kw) {
            return Some((mw_type, prefix));
        }
    }
    None
}

/// infra deploy 不挂 Application(对照 reference is_infra)。
fn is_infra(deploy_name: &str, release_prefix: &str) -> bool {
    let short = module_sdk::strip_release_prefix(deploy_name, release_prefix);
    if INFRA_NAMES.contains(&short) {
        return true;
    }
    INFRA_NAMES.iter().any(|infra| short.starts_with(infra))
}

/// 把一批 K8s list 响应映射成 topology Fact 列表(节点 + 富化边)。
pub fn map_cluster(input: &ClusterInput) -> Vec<Fact> {
    let c = input.cluster.as_str();
    let ns = input.namespace.as_str();
    let release = input.release_prefix.as_str();
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

    // 4) Application(1 个 per c:ns:release,parent = ns)
    let app_rid = format!("app:{c}:{ns}:{release}");
    facts.push(node_fact(
        now,
        &app_rid,
        "Application",
        json!({
            "cluster": c,
            "namespace": ns,
            "name": release,
            "parent_resource_id": format!("ns:{c}:{ns}"),
            "release": release,
            "health_status": "normal",
            "risk_level": "low",
        }),
    ));

    // 5) Deployments + Component/Middleware owner(infra 不挂 application)
    let mut comp_seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut mw_seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for d in items(&input.deployments) {
        let Some(name) = meta_name(d) else { continue };
        let deploy_rid = format!("deploy:{c}:{ns}:{name}");
        facts.push(node_fact(
            now,
            &deploy_rid,
            "Deployment",
            json!({
                "cluster": c,
                "namespace": ns,
                "name": name,
                "parent_resource_id": format!("ns:{c}:{ns}"),
                "current_revision": deploy_revision(d),
                "images": deploy_images(d),
                "replicas_desired": deploy_replicas_desired(d),
                "replicas_ready": deploy_replicas_ready(d),
                "health_status": "normal",
                "risk_level": "low",
            }),
        ));

        if is_infra(name, release) {
            continue;
        }
        if let Some((mw_type, mw_prefix)) = detect_middleware(name, release) {
            // Middleware node(dedup)+ DEPLOYED_AS(mw -> deploy)
            let mw_rid = format!("{mw_prefix}:{c}:{ns}:{name}");
            if mw_seen.insert(mw_rid.clone()) {
                facts.push(node_fact(
                    now,
                    &mw_rid,
                    mw_type,
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
            facts.push(edge_fact(now, "DEPLOYED_AS", &mw_rid, &deploy_rid));
        } else {
            // ApplicationComponent node(dedup,parent=app 派生 CONTAINS)+ DEPLOYED_AS/BELONGS_TO
            let comp_name = module_sdk::normalize_component_name(name, release);
            let comp_rid = module_sdk::component_id(c, ns, &comp_name);
            if comp_seen.insert(comp_rid.clone()) {
                facts.push(node_fact(
                    now,
                    &comp_rid,
                    "ApplicationComponent",
                    json!({
                        "cluster": c,
                        "namespace": ns,
                        "name": comp_name,
                        "parent_resource_id": app_rid.clone(),
                        "health_status": "normal",
                        "risk_level": "low",
                    }),
                ));
                // BELONGS_TO(comp -> app)反向边
                facts.push(edge_fact(now, "BELONGS_TO", &comp_rid, &app_rid));
            }
            // DEPLOYED_AS(comp -> deploy) + BELONGS_TO(deploy -> comp)
            facts.push(edge_fact(now, "DEPLOYED_AS", &comp_rid, &deploy_rid));
            facts.push(edge_fact(now, "BELONGS_TO", &deploy_rid, &comp_rid));
        }
    }

    // 5) ReplicaSet name -> Deployment name(owner 链中间层,不入图)
    let rs_to_deploy = index_rs_to_deploy(&input.replicasets);

    // 6) ConfigMaps(node + 供 Pod USES 边引用)
    for cm in items(&input.configmaps) {
        let Some(name) = meta_name(cm) else { continue };
        facts.push(node_fact(
            now,
            &format!("cm:{c}:{ns}:{name}"),
            "ConfigMap",
            json!({
                "cluster": c,
                "namespace": ns,
                "name": name,
                "parent_resource_id": format!("ns:{c}:{ns}"),
                "data_keys": data_keys_of(cm),
                "health_status": "normal",
                "risk_level": "low",
            }),
        ));
    }

    // 7) Secrets(node + 供 Pod USES 边引用;不存 data 值,只存 data_keys)
    for s in items(&input.secrets) {
        let Some(name) = meta_name(s) else { continue };
        facts.push(node_fact(
            now,
            &format!("secret:{c}:{ns}:{name}"),
            "Secret",
            json!({
                "cluster": c,
                "namespace": ns,
                "name": name,
                "parent_resource_id": format!("ns:{c}:{ns}"),
                "data_keys": data_keys_of(s),
                "health_status": "normal",
                "risk_level": "low",
            }),
        ));
    }

    // 8) Pods -- parent(经 rs 链)+ SCHEDULED_ON 边 + USES 边
    for p in items(&input.pods) {
        let Some(name) = meta_name(p) else { continue };
        let parent = pod_parent(p, &rs_to_deploy, c, ns);
        let health = pod_health(p);
        let node_name = p
            .get("spec")
            .and_then(|s| s.get("nodeName"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let pod_id = format!("pod:{c}:{ns}:{name}");
        facts.push(node_fact(
            now,
            &pod_id,
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

        // SCHEDULED_ON:pod -> node
        if !node_name.is_empty() {
            facts.push(edge_fact(
                now,
                "SCHEDULED_ON",
                &pod_id,
                &format!("node:{c}:{node_name}"),
            ));
        }
        // USES:pod -> configmap(volumes.configMap + envFrom.configMapRef)
        for cm_name in pod_cm_refs(p) {
            facts.push(edge_fact(now, "USES", &pod_id, &format!("cm:{c}:{ns}:{cm_name}")));
        }
        // USES:pod -> secret(volumes.secret + envFrom.secretRef)
        for sec_name in pod_secret_refs(p) {
            facts.push(edge_fact(now, "USES", &pod_id, &format!("secret:{c}:{ns}:{sec_name}")));
        }
        // RUNS:pod -> container(spec.containers[];status 按 name 匹配 containerStatuses)
        for ctr in pod_containers(p) {
            let ctr_id = format!("container:{c}:{ns}:{name}:{}", ctr.name);
            facts.push(node_fact(
                now,
                &ctr_id,
                "Container",
                json!({
                    "cluster": c,
                    "namespace": ns,
                    "name": ctr.name,
                    "pod": pod_id,
                    "image": ctr.image,
                    "ready": ctr.ready,
                    "restart_count": ctr.restart_count,
                    "health_status": ctr.health,
                    "risk_level": risk_from_health(ctr.health),
                }),
            ));
            facts.push(edge_fact(now, "RUNS", &pod_id, &ctr_id));
            // USES_IMAGE:container -> image(Phase 5.2 image 富化)。每容器用镜像产
            // ContainerImage node(`image:{c}:{ns}:{image-ref}`,同镜像靠 resource_id
            // 在 facts_to_graph 去重)+ USES_IMAGE 边。image-risk 视图据此反向找用此镜像的负载。
            if !ctr.image.is_empty() {
                let img_id = format!("image:{c}:{ns}:{}", ctr.image);
                facts.push(node_fact(
                    now,
                    &img_id,
                    "ContainerImage",
                    json!({
                        "cluster": c,
                        "namespace": ns,
                        "name": ctr.image,
                        "image": ctr.image,
                    }),
                ));
                facts.push(edge_fact(now, "USES_IMAGE", &ctr_id, &img_id));
            }
        }
    }

    // 9) Services -- ROUTES_TO:selector 匹配 pod labels;EXPOSES:selector 匹配
    //    deploy pod template labels(doc/02 L2:Service EXPOSES Deployment)
    let pod_labels = index_labels(&input.pods, c, ns, "pod", pod_template_labels);
    let deploy_labels = index_labels(&input.deployments, c, ns, "deploy", deploy_template_labels);
    for s in items(&input.services) {
        let Some(name) = meta_name(s) else { continue };
        let svc_id = format!("service:{c}:{ns}:{name}");
        facts.push(node_fact(
            now,
            &svc_id,
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
        let selector = s
            .get("spec")
            .and_then(|sp| sp.get("selector"))
            .and_then(Value::as_object);
        if let Some(selector) = selector {
            if !selector.is_empty() {
                for ple in &pod_labels {
                    if labels_match(&ple.labels, selector) {
                        facts.push(edge_fact(now, "ROUTES_TO", &svc_id, &ple.id));
                    }
                }
                for dle in &deploy_labels {
                    if labels_match(&dle.labels, selector) {
                        facts.push(edge_fact(now, "EXPOSES", &svc_id, &dle.id));
                    }
                }
            }
        }
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

/// 构造一条 topology-edge Fact(富化边)。resource_id 含 edge_type 避免同 src->tgt
/// 不同 edge_type 撞 latest 去重 key;id 含 ts 避免跨轮撞。
fn edge_fact(now: u64, edge_type: &str, source: &str, target: &str) -> Fact {
    let resource_id = format!("edge:{edge_type}:{source}->{target}");
    Fact {
        id: format!("{SOURCE}:edge:{edge_type}:{source}->{target}:{now}"),
        kind: EDGE_KIND.to_string(),
        source: SOURCE.to_string(),
        resource_id,
        resource_type: "Edge".to_string(),
        timestamp: now,
        attributes_json: json!({ "source": source, "target": target, "edge_type": edge_type })
            .to_string(),
    }
}

/// `{"items":[...]}` -> items slice(非数组返空)。
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

/// Deployment `metadata.annotations["deployment.kubernetes.io/revision"]`(变更检测信号)。
fn deploy_revision(d: &Value) -> String {
    d.get("metadata")
        .and_then(|m| m.get("annotations"))
        .and_then(|a| a.get("deployment.kubernetes.io/revision"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Deployment `spec.template.spec.containers[].image`(排序去重,逗号分隔;变更检测信号)。
fn deploy_images(d: &Value) -> String {
    let mut imgs: Vec<String> = d
        .get("spec")
        .and_then(|s| s.get("template"))
        .and_then(|t| t.get("spec"))
        .and_then(|s| s.get("containers"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|c| c.get("image").and_then(Value::as_str).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    imgs.sort();
    imgs.dedup();
    imgs.join(",")
}

/// Deployment `spec.replicas`(缺失 -> 0;变更检测信号)。
fn deploy_replicas_desired(d: &Value) -> i64 {
    d.get("spec")
        .and_then(|s| s.get("replicas"))
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

/// Deployment `status.readyReplicas`(缺失 -> 0;变更检测信号)。
fn deploy_replicas_ready(d: &Value) -> i64 {
    d.get("status")
        .and_then(|s| s.get("readyReplicas"))
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

/// ReplicaSet name -> 拥有它的 Deployment name。
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

/// Pod -> parent resource_id:Deployment(经 rs 链)否则 Namespace。
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

    // CrashLoopBackOff(任一容器 waiting.reason)-> critical
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

/// Node health:status.conditions 里 Ready=True -> normal,否则 critical。
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

/// health -> risk_level 映射(给 Cytoscape border 配色)。
fn risk_from_health(health: &str) -> &'static str {
    match health {
        "critical" => "high",
        "warning" => "medium",
        _ => "low",
    }
}

/// ConfigMap/Secret 的 data key 名(排序后逗号分隔)。不取 data 值。
fn data_keys_of(obj: &Value) -> String {
    let mut keys: Vec<&str> = obj
        .get("data")
        .and_then(Value::as_object)
        .map(|m| m.keys().map(String::as_str).collect())
        .unwrap_or_default();
    keys.sort();
    keys.join(",")
}

/// 去重保序地把 name 推进 refs(空名跳过)。
fn push_ref(
    refs: &mut Vec<String>,
    seen: &mut std::collections::HashSet<String>,
    name: &str,
) {
    if !name.is_empty() && seen.insert(name.to_string()) {
        refs.push(name.to_string());
    }
}

/// Pod 引用的 ConfigMap name(volumes.configMap + envFrom.configMapRef,去重保序)。
fn pod_cm_refs(pod: &Value) -> Vec<String> {
    let mut refs: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    if let Some(vols) = pod
        .get("spec")
        .and_then(|s| s.get("volumes"))
        .and_then(Value::as_array)
    {
        for v in vols {
            if let Some(name) = v
                .get("configMap")
                .and_then(|c| c.get("name"))
                .and_then(Value::as_str)
            {
                push_ref(&mut refs, &mut seen, name);
            }
        }
    }
    if let Some(containers) = pod
        .get("spec")
        .and_then(|s| s.get("containers"))
        .and_then(Value::as_array)
    {
        for c in containers {
            if let Some(envfrom) = c.get("envFrom").and_then(Value::as_array) {
                for ef in envfrom {
                    if let Some(name) = ef
                        .get("configMapRef")
                        .and_then(|r| r.get("name"))
                        .and_then(Value::as_str)
                    {
                        push_ref(&mut refs, &mut seen, name);
                    }
                }
            }
        }
    }
    refs
}

/// Pod 引用的 Secret name(volumes.secret.secretName + envFrom.secretRef,去重保序)。
fn pod_secret_refs(pod: &Value) -> Vec<String> {
    let mut refs: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    if let Some(vols) = pod
        .get("spec")
        .and_then(|s| s.get("volumes"))
        .and_then(Value::as_array)
    {
        for v in vols {
            if let Some(name) = v
                .get("secret")
                .and_then(|s| s.get("secretName"))
                .and_then(Value::as_str)
            {
                push_ref(&mut refs, &mut seen, name);
            }
        }
    }
    if let Some(containers) = pod
        .get("spec")
        .and_then(|s| s.get("containers"))
        .and_then(Value::as_array)
    {
        for c in containers {
            if let Some(envfrom) = c.get("envFrom").and_then(Value::as_array) {
                for ef in envfrom {
                    if let Some(name) = ef
                        .get("secretRef")
                        .and_then(|r| r.get("name"))
                        .and_then(Value::as_str)
                    {
                        push_ref(&mut refs, &mut seen, name);
                    }
                }
            }
        }
    }
    refs
}

/// label 索引项(供 Service ROUTES_TO / EXPOSES selector 匹配)。
struct LabelEntry {
    id: String,
    labels: Map<String, Value>,
}

/// Pod 的匹配 labels:`metadata.labels`(K8s selector 直接匹配 pod labels)。
fn pod_template_labels(obj: &Value) -> Option<Map<String, Value>> {
    obj.get("metadata")
        .and_then(|m| m.get("labels"))
        .and_then(Value::as_object)
        .cloned()
}

/// Deployment 的匹配 labels:`spec.template.metadata.labels`(pod 模板 labels,
/// selector 匹配它等价于匹配该 deploy 的全部 pod)。
fn deploy_template_labels(obj: &Value) -> Option<Map<String, Value>> {
    obj.get("spec")
        .and_then(|s| s.get("template"))
        .and_then(|t| t.get("metadata"))
        .and_then(|m| m.get("labels"))
        .and_then(Value::as_object)
        .cloned()
}

/// 建 `{prefix}:{c}:{ns}:{name}` resource_id -> labels 索引(label 取法由 `pick` 决定)。
fn index_labels(
    list: &Value,
    c: &str,
    ns: &str,
    prefix: &str,
    pick: fn(&Value) -> Option<Map<String, Value>>,
) -> Vec<LabelEntry> {
    let mut out = Vec::new();
    for obj in items(list) {
        let Some(name) = meta_name(obj) else { continue };
        out.push(LabelEntry {
            id: format!("{prefix}:{c}:{ns}:{name}"),
            labels: pick(obj).unwrap_or_default(),
        });
    }
    out
}

/// Pod 容器规格 + 运行态(供 Container node + RUNS 边)。
struct ContainerSpec {
    name: String,
    image: String,
    ready: bool,
    restart_count: i64,
    health: &'static str,
}

/// `spec.containers[]` 每个容器一条;ready / restart_count / health 从
/// `status.containerStatuses[]` 按 name 匹配取(无 status -> ready=false /
/// restart=0 / warning;waiting=CrashLoopBackOff -> critical)。initContainers 不产。
fn pod_containers(pod: &Value) -> Vec<ContainerSpec> {
    let mut out = Vec::new();
    let specs = pod
        .get("spec")
        .and_then(|s| s.get("containers"))
        .and_then(Value::as_array);
    let statuses = pod
        .get("status")
        .and_then(|s| s.get("containerStatuses"))
        .and_then(Value::as_array);
    let Some(specs) = specs else { return out };
    for c in specs {
        let Some(name) = c.get("name").and_then(Value::as_str) else {
            continue;
        };
        let image = c
            .get("image")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let st = statuses.and_then(|arr| {
            arr.iter()
                .find(|s| s.get("name").and_then(Value::as_str) == Some(name))
        });
        let ready = st
            .and_then(|s| s.get("ready").and_then(Value::as_bool))
            .unwrap_or(false);
        let restart_count = st
            .and_then(|s| s.get("restartCount").and_then(Value::as_i64))
            .unwrap_or(0);
        let crashloop = st
            .map(|s| {
                s.get("state")
                    .and_then(|st| st.get("waiting"))
                    .and_then(|w| w.get("reason"))
                    .and_then(Value::as_str)
                    == Some("CrashLoopBackOff")
            })
            .unwrap_or(false);
        let health = if crashloop {
            "critical"
        } else if ready {
            "normal"
        } else {
            "warning"
        };
        out.push(ContainerSpec {
            name: name.to_string(),
            image,
            ready,
            restart_count,
            health,
        });
    }
    out
}

/// selector 的所有 k=v 都在 labels 里匹配。
fn labels_match(labels: &Map<String, Value>, selector: &Map<String, Value>) -> bool {
    selector
        .iter()
        .all(|(k, v)| labels.get(k) == Some(v))
}

#[cfg(test)]
mod tests;
