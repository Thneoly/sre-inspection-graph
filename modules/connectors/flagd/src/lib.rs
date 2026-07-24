//! flagd — feature-flag connector(对照 reference `flagd_connector.py`)。
//!
//! 第二个 ChangeEvent 生产者。POST `{flagd_url}/flagd.evaluation.v1.Service/ResolveAll`
//! (Connect-RPC transcoded,JSON body `{}`,**经 http-client 的 `write` + `http-write` capability** ——
//! Connect-RPC unary 走 POST,故用 write 而非 get)拿 `{flags:{name:state}}` 快照,diff 上一轮,
//! 每条翻转 `configmap_updated` ChangeEvent(target = flagd ConfigMap 节点),命中 OTel Demo 8 故障
//! scenario 时富化 `diff_summary.scenario` + description。**不产节点/边/metric/alert**。
//!
//! ## 有状态
//!
//! 首次 sync = baseline:存快照,**不发**。之后每 sync diff → 发 delta。状态存 guest
//! `thread_local`(WasmConnector 复用同一 instance,static 跨 sync 持久)。
//!
//! ## 真集群可用性
//!
//! **本集群未部署 flagd**(无 flagd svc/deploy/ConfigMap)→ 本 connector 真集群不可达,
//! 仅 e2e/contract 验证(同 prometheus OOM)。真值需部署 flagd。
//!
//! ## config_json
//!
//! ```json
//! { "flagd_url": "http://otel-demo-flagd:8013", "cluster": "vm-cluster",
//!   "namespace": "otel-demo", "flagd_configmap_name": "otel-demo-flagd-config" }
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

    use super::{bindings, mapper};
    use bindings::exports::sre::inspection::connector::{Fact, Guest, SyncError, SyncResult};
    use bindings::sre::inspection::http_client::{self, Error as HttpError, WriteRequest};
    use mapper::{ResolveAllResp, Snapshot};

    pub struct Flagd;

    #[derive(serde::Deserialize)]
    struct Config {
        #[serde(default)]
        flagd_url: String,
        #[serde(default = "default_cluster")]
        cluster: String,
        #[serde(default = "default_namespace")]
        namespace: String,
        #[serde(default = "default_configmap_name")]
        flagd_configmap_name: String,
    }
    fn default_cluster() -> String {
        "local".to_string()
    }
    fn default_namespace() -> String {
        "default".to_string()
    }
    fn default_configmap_name() -> String {
        "otel-demo-flagd-config".to_string()
    }

    // 跨 sync 持久的 guest 状态:上一轮 flag 快照(None=未 baseline)。
    thread_local! {
        static LAST: RefCell<Option<Snapshot>> = const { RefCell::new(None) };
    }

    impl Guest for Flagd {
        fn sync(config_json: String) -> Result<SyncResult, SyncError> {
            let cfg: Config = if config_json.trim().is_empty() {
                Config {
                    flagd_url: String::new(),
                    cluster: default_cluster(),
                    namespace: default_namespace(),
                    flagd_configmap_name: default_configmap_name(),
                }
            } else {
                serde_json::from_str(&config_json)
                    .map_err(|e| SyncError::Config(format!("invalid config_json: {e}")))?
            };

            let now = bindings::sre::inspection::clock::now_seconds();
            let base = cfg.flagd_url.trim_end_matches('/').to_string();

            bindings::sre::inspection::logging::log(
                bindings::sre::inspection::logging::Level::Info,
                &format!("flagd sync: flagd_url={base} cluster={} ns={}", cfg.cluster, cfg.namespace),
            );

            let mut errors: Vec<String> = Vec::new();
            if base.is_empty() {
                errors.push("flagd_url is empty, skipping".to_string());
                return Ok(SyncResult { facts: vec![], errors, duration_ms: 0 });
            }

            // POST ResolveAll(Connect-RPC transcoded)。经 http-write capability。
            let req = WriteRequest {
                method: "POST".to_string(),
                url: format!("{base}/flagd.evaluation.v1.Service/ResolveAll"),
                headers: vec![("Content-Type".to_string(), "application/json".to_string())],
                body: Some(b"{}".to_vec()),
            };
            let body = match http_client::write(&req) {
                Ok(resp) if resp.status == 200 => resp.body,
                Ok(resp) => {
                    errors.push(format!("ResolveAll HTTP {}", resp.status));
                    return Ok(SyncResult { facts: vec![], errors, duration_ms: 0 });
                }
                Err(e) => {
                    errors.push(format!("ResolveAll failed: {}", http_err(e)));
                    return Ok(SyncResult { facts: vec![], errors, duration_ms: 0 });
                }
            };
            let current: Snapshot = match serde_json::from_slice::<ResolveAllResp>(&body) {
                Ok(r) => r.flags,
                Err(e) => {
                    errors.push(format!("ResolveAll parse failed: {e}"));
                    return Ok(SyncResult { facts: vec![], errors, duration_ms: 0 });
                }
            };

            let mcfg = mapper::Cfg::new(&cfg.cluster, &cfg.namespace, &cfg.flagd_configmap_name, now);

            // 首次 sync = baseline:存快照,不发。之后 diff。
            let facts: Vec<Fact> = LAST.with(|last| {
                let mut last = last.borrow_mut();
                match (*last).take() {
                    None => {
                        bindings::sre::inspection::logging::log(
                            bindings::sre::inspection::logging::Level::Info,
                            &format!("flagd baseline: {} flags, no facts emitted", current.len()),
                        );
                        *last = Some(current);
                        Vec::new()
                    }
                    Some(prev) => {
                        let deltas = mapper::diff_snapshots(&prev, &current);
                        *last = Some(current);
                        deltas
                            .iter()
                            .map(|d| mapper::delta_to_change_fact(d, &mcfg))
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
                    }
                }
            });

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
use imp::Flagd;

#[cfg(target_arch = "wasm32")]
bindings::export!(Flagd with_types_in bindings);
