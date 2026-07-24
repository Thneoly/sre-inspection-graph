//! k8s-events — K8s Events connector(对照 reference `k8s_event_connector.py`)。
//!
//! 第四条 connector + 第一个 **ChangeEvent 生产者**。poll `GET {api_base}/api/v1/namespaces/{ns}/events`
//! (经 http-client,desktop 托管的 kubectl proxy),只挑 `INTERESTING_REASONS`
//! (`ScalingReplicaSet`/`SuccessfulRescale` → `deployment_rolled`),把 involvedObject 映成
//! target resource_id,产 `kind="change"` Fact(desktop run_sync 路由到 `record_change`)。
//! **不产节点/边/metric/alert**。
//!
//! ## 有状态(新 guest 模式)
//!
//! 首次 sync = baseline:把当前所有 event UID 种进 seen 集合,**不发**(防重启录历史)。
//! 之后每 sync 只发 UID 未见过的新 event。状态存 guest `thread_local`(WasmConnector 复用
//! 同一 instance,static 跨 sync 持久)。前 3 个 connector(k8s/prom/jaeger)无状态;本
//! connector + flagd 是首批有状态 guest。
//!
//! ## config_json
//!
//! ```json
//! { "api_base": "http://127.0.0.1:8001", "cluster": "vm-cluster", "namespace": "otel-demo" }
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
    use mapper::EventList;
    use serde::Deserialize;

    pub struct K8sEvents;

    #[derive(Deserialize)]
    struct Config {
        #[serde(default = "default_api_base")]
        api_base: String,
        #[serde(default = "default_cluster")]
        cluster: String,
        #[serde(default = "default_namespace")]
        namespace: String,
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

    // 跨 sync 持久的 guest 状态(单线程 WASM -> thread_local + RefCell,无锁/无原子依赖)。
    thread_local! {
        static SEEN: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
        static BASELINED: RefCell<bool> = const { RefCell::new(false) };
    }

    impl Guest for K8sEvents {
        fn sync(config_json: String) -> Result<SyncResult, SyncError> {
            let cfg: Config = if config_json.trim().is_empty() {
                Config {
                    api_base: default_api_base(),
                    cluster: default_cluster(),
                    namespace: default_namespace(),
                }
            } else {
                serde_json::from_str(&config_json)
                    .map_err(|e| SyncError::Config(format!("invalid config_json: {e}")))?
            };

            let now = bindings::sre::inspection::clock::now_seconds();
            let base = cfg.api_base.trim_end_matches('/').to_string();

            bindings::sre::inspection::logging::log(
                bindings::sre::inspection::logging::Level::Info,
                &format!("k8s-events sync: api_base={base} cluster={} ns={}", cfg.cluster, cfg.namespace),
            );

            let mut errors: Vec<String> = Vec::new();
            if base.is_empty() {
                errors.push("api_base is empty, skipping".to_string());
                return Ok(SyncResult { facts: vec![], errors, duration_ms: 0 });
            }

            let path = format!("/api/v1/namespaces/{}/events", cfg.namespace);
            let events: Vec<mapper::Event> = match get_json::<EventList>(&base, &path, &mut errors) {
                Some(list) => list.items,
                None => return Ok(SyncResult { facts: vec![], errors, duration_ms: 0 }),
            };

            let mcfg = mapper::Cfg::new(&cfg.cluster, &cfg.namespace, now);

            // 首次 sync = baseline:种 seen,不发(对齐 reference first_sync,防重启录历史 burst)。
            let baselined = BASELINED.with(|b| *b.borrow());
            let facts: Vec<Fact> = if baselined {
                let mut out = Vec::new();
                SEEN.with(|seen| {
                    let mut seen = seen.borrow_mut();
                    for ev in &events {
                        if seen.insert(ev_uid(ev)) {
                            if let Some(f) = mapper::event_to_change_fact(ev, &mcfg) {
                                out.push(f);
                            }
                        }
                    }
                });
                // 翻译 module_sdk::Fact → WIT Fact。
                out.into_iter()
                    .map(|f| Fact {
                        id: f.id,
                        kind: f.kind,
                        source: f.source,
                        resource_id: f.resource_id,
                        resource_type: f.resource_type,
                        timestamp: f.timestamp,
                        attributes_json: f.attributes_json,
                    })
                    .collect()
            } else {
                SEEN.with(|seen| {
                    let mut seen = seen.borrow_mut();
                    for ev in &events {
                        seen.insert(ev_uid(ev));
                    }
                });
                BASELINED.with(|b| *b.borrow_mut() = true);
                bindings::sre::inspection::logging::log(
                    bindings::sre::inspection::logging::Level::Info,
                    &format!("k8s-events baseline: seeded {} event uids, no facts emitted", events.len()),
                );
                vec![]
            };

            Ok(SyncResult { facts, errors, duration_ms: 0 })
        }

        fn health_check() -> bool {
            true
        }
    }

    /// 取 event UID(mapper::Event.metadata.uid 私有,经此 helper 读)。
    fn ev_uid(ev: &mapper::Event) -> String {
        mapper::event_uid(ev)
    }

    /// GET 一个 K8s 端点,200 → 解析;失败推 error note 返 None。
    fn get_json<T: serde::de::DeserializeOwned>(
        base: &str,
        path: &str,
        errors: &mut Vec<String>,
    ) -> Option<T> {
        let url = format!("{base}{path}");
        match http_client::get(&url, &[]) {
            Ok(resp) if resp.status == 200 => match serde_json::from_slice::<T>(&resp.body) {
                Ok(v) => Some(v),
                Err(e) => {
                    errors.push(format!("GET {path} parse failed: {e}"));
                    None
                }
            },
            Ok(resp) => {
                errors.push(format!("GET {path} HTTP {}", resp.status));
                None
            }
            Err(e) => {
                errors.push(format!("GET {path} failed: {}", http_err(e)));
                None
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
use imp::K8sEvents;

#[cfg(target_arch = "wasm32")]
bindings::export!(K8sEvents with_types_in bindings);
