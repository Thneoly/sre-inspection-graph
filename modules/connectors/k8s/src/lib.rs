//! k8s — 真实 Kubernetes connector(Phase 2.6b)。
//!
//! 对照 reference `app/datasource/connectors/k8s_connector.py`:列 namespace 内
//! Deployment / ReplicaSet / Pod / Service + 集群 Node,映射成 topology Fact。
//!
//! ## 怎么连 API server(关键架构决策)
//!
//! WASM 只有 `http-client`(纯 GET + headers)capability。直连 API server 要处理
//! 自签 CA + bearer token,需扩 capability/host。**改走本地 `kubectl proxy`**:它在
//! `127.0.0.1:8001` 把 K8s API 以明文 HTTP 暴露,TLS + 认证全留在 proxy/kubeconfig。
//! 本 connector 只发 `GET {api_base}/...` —— 不碰凭据、不加新 capability、契合
//! deny-by-default。(desktop 后续可由 Rust 端托管 proxy 生命周期。)
//!
//! ## config_json
//!
//! ```json
//! { "api_base": "http://127.0.0.1:8001", "cluster": "vm-cluster", "namespace": "otel-demo" }
//! ```
//!
//! ## 分层
//!
//! 逻辑核心在 [`mapper`](纯函数,host `cargo test` 直接测);本文件的 wasm
//! `Guest::sync` 只做 HTTP GET + 调 mapper + 把 `module_sdk::Fact` 转 WIT `Fact`。
//! host 链接器不认 wit-bindgen export symbol 的 `:`/`@`,故 guest 逻辑全
//! `cfg(target_arch = "wasm32")` 守卫;mapper 不守卫,host 端可测。

#![allow(missing_docs)]

pub mod mapper;

#[cfg(target_arch = "wasm32")]
mod bindings {
    wit_bindgen::generate!({
        world: "connector-world",
        path: "../../../specs/wit",
        generate_all,
    });
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::{bindings, mapper};
    use bindings::exports::sre::inspection::connector::{Fact, Guest, SyncError, SyncResult};
    use bindings::sre::inspection::http_client::{self, Error as HttpError};
    use serde::Deserialize;
    use serde_json::Value;

    pub struct K8s;

    #[derive(Deserialize)]
    struct Config {
        #[serde(default = "default_api_base")]
        api_base: String,
        #[serde(default = "default_cluster")]
        cluster: String,
        #[serde(default = "default_namespace")]
        namespace: String,
        #[serde(default = "default_release_prefix")]
        release_prefix: String,
    }
    fn default_api_base() -> String {
        "http://127.0.0.1:8001".to_string()
    }
    fn default_cluster() -> String {
        "local".to_string()
    }
    fn default_namespace() -> String {
        "default".to_string()
    }
    fn default_release_prefix() -> String {
        mapper::DEFAULT_RELEASE_PREFIX.to_string()
    }

    impl Guest for K8s {
        fn sync(config_json: String) -> Result<SyncResult, SyncError> {
            let cfg: Config = if config_json.trim().is_empty() {
                Config {
                    api_base: default_api_base(),
                    cluster: default_cluster(),
                    namespace: default_namespace(),
                    release_prefix: default_release_prefix(),
                }
            } else {
                serde_json::from_str(&config_json)
                    .map_err(|e| SyncError::Config(format!("invalid config_json: {e}")))?
            };

            let now = bindings::sre::inspection::clock::now_seconds();
            let base = cfg.api_base.trim_end_matches('/').to_string();
            let ns = cfg.namespace.clone();

            bindings::sre::inspection::logging::log(
                bindings::sre::inspection::logging::Level::Info,
                &format!(
                    "k8s sync: api_base={base} cluster={} ns={ns}",
                    cfg.cluster
                ),
            );

            let mut errors: Vec<String> = Vec::new();
            if base.is_empty() {
                errors.push("api_base is empty, skipping".to_string());
                return Ok(SyncResult {
                    facts: vec![],
                    errors,
                    duration_ms: 0,
                });
            }

            let nodes = get_json(&base, "/api/v1/nodes", &mut errors);
            let deployments = get_json(
                &base,
                &format!("/apis/apps/v1/namespaces/{ns}/deployments"),
                &mut errors,
            );
            let replicasets = get_json(
                &base,
                &format!("/apis/apps/v1/namespaces/{ns}/replicasets"),
                &mut errors,
            );
            let pods = get_json(
                &base,
                &format!("/api/v1/namespaces/{ns}/pods"),
                &mut errors,
            );
            let services = get_json(
                &base,
                &format!("/api/v1/namespaces/{ns}/services"),
                &mut errors,
            );
            let configmaps = get_json(
                &base,
                &format!("/api/v1/namespaces/{ns}/configmaps"),
                &mut errors,
            );
            let secrets = get_json(
                &base,
                &format!("/api/v1/namespaces/{ns}/secrets"),
                &mut errors,
            );

            let input = mapper::ClusterInput {
                cluster: cfg.cluster,
                namespace: ns,
                now,
                nodes,
                deployments,
                replicasets,
                pods,
                services,
                configmaps,
                secrets,
                release_prefix: cfg.release_prefix,
            };
            let facts = mapper::map_cluster(&input)
                .into_iter()
                .map(|f| Fact {
                    id: f.id,
                    kind: f.kind,
                    source: f.source,
                    resource_id: f.resource_id,
                    resource_type: f.resource_type,
                    timestamp: f.timestamp,
                    attributes_json: f.attributes_json,
                })
                .collect();

            Ok(SyncResult {
                facts,
                errors,
                duration_ms: 0,
            })
        }

        fn health_check() -> bool {
            true
        }
    }

    /// GET 一个 K8s list 端点,200 → 解析 JSON;非 200 / 网络错 / 解析错 → 推
    /// error note 并返 `Value::Null`(mapper 把 Null 当空 items,整轮不挂)。
    fn get_json(base: &str, path: &str, errors: &mut Vec<String>) -> Value {
        let url = format!("{base}{path}");
        match http_client::get(&url, &[]) {
            Ok(resp) if resp.status == 200 => match serde_json::from_slice(&resp.body) {
                Ok(v) => v,
                Err(e) => {
                    errors.push(format!("GET {path} parse failed: {e}"));
                    Value::Null
                }
            },
            Ok(resp) => {
                errors.push(format!("GET {path} HTTP {}", resp.status));
                Value::Null
            }
            Err(e) => {
                errors.push(format!("GET {path} failed: {}", http_err(e)));
                Value::Null
            }
        }
    }

    fn http_err(e: HttpError) -> String {
        match e {
            HttpError::Unauthorized => "unauthorized (capability denied?)".to_string(),
            HttpError::NotFound => "not found".to_string(),
            HttpError::Network(m) => format!("network: {m}"),
            HttpError::Timeout => "timeout".to_string(),
        }
    }
}

#[cfg(target_arch = "wasm32")]
use imp::K8s;

#[cfg(target_arch = "wasm32")]
bindings::export!(K8s with_types_in bindings);
