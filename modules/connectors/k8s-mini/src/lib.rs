//! k8s-mini — 第二条 connector。把 K8s namespace 列表当 topology Fact 喂回 host。
//!
//! 本期作用是验证 **multi-connector in WasmRuntime**:跟 hello-world 同样接
//! connector-world,但用 config_json 读 `namespaces` 列表 + 每个 namespace 产
//! 一条 Fact。
//!
//! 输入 `config_json`(可选):
//! ```json
//! {
//!   "cluster": "vm-cluster",
//!   "namespaces": ["default", "kube-system", "otel-demo"],
//!   "with_topology": false
//! }
//! ```
//! - `cluster` 缺省 → `"local"`
//! - `namespaces` 缺省 → `["default"]`
//! - `with_topology` 缺省 → `false`(只产 Namespace Fact,向后兼容现有测试)
//!
//! ## with_topology = true(Phase 1 拓扑视图用)
//!
//! 在 namespace Fact 基础上,额外吐分层 mock topology Fact,供 desktop 拓扑视图
//! 渲染。**全是 mock 数据**,不调 K8s API。schema 严格按 doc/02 L1-L2 模型:
//!
//! - **1 个 Cluster** —— `cluster:<name>`,无 parent
//! - **2 个 Node** —— `node:<cluster>:<role>`,parent = cluster
//! - **N 个 Namespace** —— `ns:<cluster>:<ns>`,parent = cluster(N = config.namespaces.len)
//! - **2N 个 Pod** —— `pod:<cluster>:<ns>:<n>`,parent = namespace,attrs.node = node1/node2 轮转
//! - **N 个 Service** —— `service:<cluster>:<ns>:web`,parent = namespace
//!
//! N=1(默认)→ 1+2+1+2+1 = 7 Fact;N=3 → 1+2+3+6+3 = 15 Fact。
//! 5-10 区间靠默认 N=1 命中,3+ namespace 给重一点的拓扑展示。
//!
//! 父子关系通过 `attributes_json` 的 `parent_resource_id` 字段表达 —— host
//! 端 / 前端按此字段建 edge,**这一约定也会被 Phase 2 真 connector 继承**。
//!
//! 当前不调 K8s API —— http-client capability 在 Phase 1 G 已实装,但 k8s-mini
//! 是占位 connector,真 K8s API 接入留 Phase 2 `modules/connectors/k8s/`(独立
//! crate,会消费 http-client capability)。
//!
//! cfg(target_arch = "wasm32") 守卫与 hello-world 同款 —— wit-bindgen 生成的 export
//! symbol 含 `:`/`@`,host 链接器不认。host 编译时此 crate 退化为空 crate。

#![allow(missing_docs)]

#[cfg(target_arch = "wasm32")]
mod bindings {
    wit_bindgen::generate!({
        world: "connector-world",
        path: "../../../specs/wit",
        generate_all,
    });
}

#[cfg(target_arch = "wasm32")]
use bindings::exports::sre::inspection::connector::{Fact, Guest, SyncError, SyncResult};

#[cfg(target_arch = "wasm32")]
struct K8sMini;

#[cfg(target_arch = "wasm32")]
#[derive(serde::Deserialize)]
struct Config {
    #[serde(default = "default_cluster")]
    cluster: String,
    #[serde(default = "default_namespaces")]
    namespaces: Vec<String>,
    /// 是否额外吐 Cluster / Node / Pod / Service 分层 Fact。
    /// 默认 false 保持向后兼容(现有 e2e 测试断言只有 Namespace Fact)。
    #[serde(default)]
    with_topology: bool,
}

#[cfg(target_arch = "wasm32")]
fn default_cluster() -> String {
    "local".to_string()
}

#[cfg(target_arch = "wasm32")]
fn default_namespaces() -> Vec<String> {
    vec!["default".to_string()]
}

#[cfg(target_arch = "wasm32")]
impl Default for Config {
    fn default() -> Self {
        Self {
            cluster: default_cluster(),
            namespaces: default_namespaces(),
            with_topology: false,
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl Guest for K8sMini {
    fn sync(config_json: String) -> Result<SyncResult, SyncError> {
        // 空 config / 解析失败 → 走默认值。
        // 显式 invalid JSON(传 "not-json")→ 返 SyncError::Config,让 host 看到
        // 配置错误。空字符串当成 "用默认配置" —— hello-world 也是这么处理的。
        let cfg: Config = if config_json.trim().is_empty() {
            Config::default()
        } else {
            serde_json::from_str(&config_json).map_err(|e| {
                SyncError::Config(format!("invalid config_json: {e}"))
            })?
        };

        let now_seconds = bindings::sre::inspection::clock::now_seconds();
        bindings::sre::inspection::logging::log(
            bindings::sre::inspection::logging::Level::Info,
            &format!(
                "k8s-mini sync: cluster={} namespaces={} with_topology={}",
                cfg.cluster,
                cfg.namespaces.len(),
                cfg.with_topology,
            ),
        );

        let mut facts: Vec<Fact> = Vec::new();

        // 拓扑 mode 开启时,先吐 Cluster + Nodes(在 Namespace 之前,顺序与
        // 父子关系一致,方便前端按到达顺序建图)
        if cfg.with_topology {
            // 1) Cluster —— 顶层节点,无 parent
            facts.push(Fact {
                id: format!("k8s-mini:cluster:{}:{now_seconds}", cfg.cluster),
                kind: "topology-node".to_string(),
                source: "k8s-mini".to_string(),
                resource_id: format!("cluster:{}", cfg.cluster),
                resource_type: "Cluster".to_string(),
                timestamp: now_seconds,
                attributes_json: format!(r#"{{"cluster":"{}"}}"#, cfg.cluster),
            });

            // 2) 2 个 Node(mock — control-plane + worker)
            for role in ["control-plane", "worker"] {
                facts.push(Fact {
                    id: format!(
                        "k8s-mini:node:{}:{}:{now_seconds}",
                        cfg.cluster, role
                    ),
                    kind: "topology-node".to_string(),
                    source: "k8s-mini".to_string(),
                    resource_id: format!("node:{}:{}", cfg.cluster, role),
                    resource_type: "Node".to_string(),
                    timestamp: now_seconds,
                    attributes_json: format!(
                        r#"{{"cluster":"{}","role":"{}","parent_resource_id":"cluster:{}"}}"#,
                        cfg.cluster, role, cfg.cluster
                    ),
                });
            }
        }

        // 3) Namespace —— 既有行为,无条件吐(向后兼容)
        for ns in &cfg.namespaces {
            // attributes 字段:
            // - 总是有 cluster + namespace
            // - with_topology 时加 parent_resource_id 指向 cluster
            let attrs = if cfg.with_topology {
                format!(
                    r#"{{"cluster":"{}","namespace":"{}","parent_resource_id":"cluster:{}"}}"#,
                    cfg.cluster, ns, cfg.cluster
                )
            } else {
                format!(
                    r#"{{"cluster":"{}","namespace":"{}"}}"#,
                    cfg.cluster, ns
                )
            };
            facts.push(Fact {
                // id 格式与 PRD-004 K8sConnector 对齐:`{source}:{resource_id}:{ts}`
                // 这样 host 端聚合时不会跟 hello-world 撞 ID。
                id: format!("k8s-mini:ns:{}:{}:{now_seconds}", cfg.cluster, ns),
                kind: "topology-node".to_string(),
                source: "k8s-mini".to_string(),
                // resource_id 命名空间 schema:`ns:<cluster>:<namespace>`
                // 与 doc/02-L1-L2-type-and-instance-model.md Namespace 一致。
                resource_id: format!("ns:{}:{}", cfg.cluster, ns),
                resource_type: "Namespace".to_string(),
                timestamp: now_seconds,
                attributes_json: attrs,
            });
        }

        // 4) Pod / Service —— 仅在 with_topology 时吐(防止破现有测试断言数量)
        if cfg.with_topology {
            let nodes = ["control-plane", "worker"];
            for (ns_idx, ns) in cfg.namespaces.iter().enumerate() {
                // 每个 namespace 2 个 Pod,平均分布到 2 个 node
                for pod_idx in 0..2 {
                    let pod_name = format!("app-{ns_idx}-{pod_idx}");
                    let node_role = nodes[(ns_idx + pod_idx) % nodes.len()];
                    facts.push(Fact {
                        id: format!(
                            "k8s-mini:pod:{}:{}:{}:{now_seconds}",
                            cfg.cluster, ns, pod_name
                        ),
                        kind: "topology-node".to_string(),
                        source: "k8s-mini".to_string(),
                        resource_id: format!(
                            "pod:{}:{}:{}",
                            cfg.cluster, ns, pod_name
                        ),
                        resource_type: "Pod".to_string(),
                        timestamp: now_seconds,
                        // parent_resource_id 指向 Namespace;node 字段记录
                        // 调度到哪个 Node(前端可选地建辅助 edge)
                        attributes_json: format!(
                            r#"{{"cluster":"{}","namespace":"{}","node":"node:{}:{}","parent_resource_id":"ns:{}:{}"}}"#,
                            cfg.cluster, ns, cfg.cluster, node_role, cfg.cluster, ns
                        ),
                    });
                }

                // 每个 namespace 1 个 Service(挂在 namespace 下)
                facts.push(Fact {
                    id: format!(
                        "k8s-mini:svc:{}:{}:web:{now_seconds}",
                        cfg.cluster, ns
                    ),
                    kind: "topology-node".to_string(),
                    source: "k8s-mini".to_string(),
                    resource_id: format!("service:{}:{}:web", cfg.cluster, ns),
                    resource_type: "Service".to_string(),
                    timestamp: now_seconds,
                    attributes_json: format!(
                        r#"{{"cluster":"{}","namespace":"{}","parent_resource_id":"ns:{}:{}"}}"#,
                        cfg.cluster, ns, cfg.cluster, ns
                    ),
                });
            }
        }

        Ok(SyncResult {
            facts,
            errors: vec![],
            duration_ms: 0,
        })
    }

    fn health_check() -> bool {
        // 静态 connector — 没有外部依赖,永远健康。
        // Phase 3 接 K8s API 后改成 `GET /healthz` 探活。
        true
    }
}

#[cfg(target_arch = "wasm32")]
bindings::export!(K8sMini with_types_in bindings);
