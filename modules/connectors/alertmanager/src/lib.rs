//! alertmanager — Alertmanager v2 alerts connector(live 告警源之二)。
//!
//! 与 k8s-events 的 Warning 事件产线互补:那是「集群自报异常」,这是「完整告警
//! 规则引擎」(Prometheus rules → Alertmanager)。poll `GET {alertmanager_url}/api/v2/alerts`
//! (经 kubectl proxy 子路径,同 jaeger 模式:TLS/凭据留 proxy),`status=active`
//! 的新 fingerprint → `kind="alert"` Fact(desktop run_sync 路由进 AlertRegistry)。
//!
//! **有状态**(与 k8s-events 同款):首次 sync baseline 种 fingerprints 不发(防
//! 重启录历史 burst);之后只发新 fingerprint。v1 不跟踪 active→resolved 转好
//! (fingerprint 一生一次;升级见 mapper.rs 模块头)。
//!
//! ## config_json
//!
//! ```json
//! { "alertmanager_url": "http://127.0.0.1:8001/api/v1/namespaces/monitoring/services/prom-kube-prometheus-stack-alertmanager:9093/proxy",
//!   "cluster": "vm-cluster", "namespace": "monitoring" }
//! ```

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
    use std::cell::RefCell;
    use std::collections::HashSet;

    use super::{bindings, mapper};
    use bindings::exports::sre::inspection::connector::{Fact, Guest, SyncError, SyncResult};
    use bindings::sre::inspection::http_client::{self, Error as HttpError};
    use serde::Deserialize;

    pub struct Alertmanager;

    #[derive(Deserialize)]
    struct Config {
        #[serde(default = "default_url")]
        alertmanager_url: String,
        #[serde(default = "default_cluster")]
        cluster: String,
        /// labels 无 namespace 时的兜底(alertmanager 自身的 ns,如 monitoring)。
        #[serde(default = "default_namespace")]
        namespace: String,
    }
    fn default_url() -> String {
        "http://127.0.0.1:8001".to_string()
    }
    fn default_cluster() -> String {
        "local".to_string()
    }
    fn default_namespace() -> String {
        "default".to_string()
    }

    thread_local! {
        static SEEN: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
        static BASELINED: RefCell<bool> = const { RefCell::new(false) };
    }

    impl Guest for Alertmanager {
        fn sync(config_json: String) -> Result<SyncResult, SyncError> {
            let cfg: Config = if config_json.trim().is_empty() {
                Config {
                    alertmanager_url: default_url(),
                    cluster: default_cluster(),
                    namespace: default_namespace(),
                }
            } else {
                serde_json::from_str(&config_json)
                    .map_err(|e| SyncError::Config(format!("invalid config_json: {e}")))?
            };

            let now = bindings::sre::inspection::clock::now_seconds();
            let base = cfg.alertmanager_url.trim_end_matches('/').to_string();

            bindings::sre::inspection::logging::log(
                bindings::sre::inspection::logging::Level::Info,
                &format!(
                    "alertmanager sync: url={base} cluster={} ns={}",
                    cfg.cluster, cfg.namespace
                ),
            );

            let mut errors: Vec<String> = Vec::new();
            if base.is_empty() {
                errors.push("alertmanager_url is empty, skipping".to_string());
                return Ok(SyncResult { facts: vec![], errors, duration_ms: 0 });
            }

            let url = format!("{base}/api/v2/alerts");
            let list: Option<mapper::AlertsList> = match http_client::get(&url, &[]) {
                Ok(resp) if resp.status == 200 => serde_json::from_slice(&resp.body).map_err(|e| {
                    errors.push(format!("GET /api/v2/alerts parse failed: {e}"));
                    e
                })
                .ok(),
                Ok(resp) => {
                    errors.push(format!("GET /api/v2/alerts HTTP {}", resp.status));
                    None
                }
                Err(e) => {
                    errors.push(format!("GET /api/v2/alerts failed: {}", http_err(e)));
                    None
                }
            };
            let Some(list) = list else {
                return Ok(SyncResult { facts: vec![], errors, duration_ms: 0 });
            };

            // baseline / 增量(与 k8s-events 同款;去重键 = fingerprint)
            let baselined = BASELINED.with(|b| *b.borrow());
            let facts: Vec<Fact> = if baselined {
                let mut out = Vec::new();
                SEEN.with(|seen| {
                    let mut seen = seen.borrow_mut();
                    for a in &list.alerts {
                        let key = mapper::alert_key(a);
                        if seen.insert(key) {
                            if let Some(f) =
                                mapper::alert_to_fact(a, now, &cfg.cluster, &cfg.namespace)
                            {
                                out.push(Fact {
                                    id: f.id,
                                    kind: f.kind,
                                    source: f.source,
                                    resource_id: f.resource_id,
                                    resource_type: f.resource_type,
                                    timestamp: f.timestamp,
                                    attributes_json: f.attributes_json,
                                });
                            }
                        }
                    }
                });
                out
            } else {
                SEEN.with(|seen| {
                    let mut seen = seen.borrow_mut();
                    for a in &list.alerts {
                        seen.insert(mapper::alert_key(a));
                    }
                });
                BASELINED.with(|b| *b.borrow_mut() = true);
                bindings::sre::inspection::logging::log(
                    bindings::sre::inspection::logging::Level::Info,
                    &format!(
                        "alertmanager baseline: seeded {} fingerprints, no facts emitted",
                        list.alerts.len()
                    ),
                );
                vec![]
            };

            Ok(SyncResult { facts, errors, duration_ms: 0 })
        }

        fn health_check() -> bool {
            true
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
use imp::Alertmanager;

#[cfg(target_arch = "wasm32")]
bindings::export!(Alertmanager with_types_in bindings);
