//! jaeger — trace connector(对照 reference `jaeger_connector.py` + `trace_aggregator.py`)。
//!
//! 第一个 **trace 数据源** + 第三条 connector(k8s / prometheus / jaeger)。GET
//! `{jaeger_url}/api/services` 列服务,逐非内部服务 `GET {jaeger_url}/api/traces`,
//! 从跨服务 `CHILD_OF` span 引用聚合成 `CALLS` topology-edge Fact。**只产边,不产节点**
//! —— 节点由 k8s connector 建;边端点 = k8s ApplicationComponent 节点 id(经
//! `module_sdk::normalize_component_name` 派生)。
//!
//! ## 连 Jaeger(架构决策)
//!
//! otel-demo 的 Jaeger query API 挂在 `/jaeger/ui` 前缀(Helm quirk)。走 desktop 托管的
//! `kubectl proxy`(8001):`jaeger_url = http://127.0.0.1:8001/.../proxy/jaeger/ui`。
//! TLS + 认证留 proxy,本 connector 只发明文 GET,不碰凭据、契合 deny-by-default。
//!
//! ## config_json
//!
//! ```json
//! {
//!   "jaeger_url": "http://127.0.0.1:8001/.../proxy/jaeger/ui",
//!   "cluster": "vm-cluster",
//!   "namespace": "otel-demo",
//!   "release_prefix": "otel-demo",
//!   "lookback_seconds": 300,
//!   "call_count_threshold": 5,
//!   "limit_per_service": 100
//! }
//! ```
//! - `jaeger_url` 空 → 推一条 error note,整轮 0 fact(reference 同款)。
//! - threshold 默认 5(对照 reference);真集群稀疏可在 manifest 调低。
//!
//! ## 分层
//!
//! 逻辑核心在 [`mapper`](纯函数,host `cargo test` 直接测);本文件 wasm `Guest::sync`
//! 只做 HTTP GET + 调 mapper + `module_sdk::Fact` → WIT `Fact`。host 链接器不认
//! wit-bindgen export symbol 的 `:`/`@`,故 guest 逻辑全 `cfg(target_arch = "wasm32")`
//! 守卫;mapper 不守卫,host 端可测。

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
    use mapper::{ServicesResp, TracesResp};
    use serde::Deserialize;

    pub struct Jaeger;

    #[derive(Deserialize)]
    struct Config {
        #[serde(default)]
        jaeger_url: String,
        #[serde(default = "default_cluster")]
        cluster: String,
        #[serde(default = "default_namespace")]
        namespace: String,
        #[serde(default = "default_release_prefix")]
        release_prefix: String,
        #[serde(default = "default_lookback")]
        lookback_seconds: u64,
        #[serde(default = "default_threshold")]
        call_count_threshold: u64,
        #[serde(default = "default_limit")]
        limit_per_service: u64,
    }
    fn default_cluster() -> String {
        "local".to_string()
    }
    fn default_namespace() -> String {
        "default".to_string()
    }
    fn default_release_prefix() -> String {
        "otel-demo".to_string()
    }
    fn default_lookback() -> u64 {
        300
    }
    fn default_threshold() -> u64 {
        5
    }
    fn default_limit() -> u64 {
        100
    }

    impl Default for Config {
        fn default() -> Self {
            Self {
                jaeger_url: String::new(),
                cluster: default_cluster(),
                namespace: default_namespace(),
                release_prefix: default_release_prefix(),
                lookback_seconds: default_lookback(),
                call_count_threshold: default_threshold(),
                limit_per_service: default_limit(),
            }
        }
    }

    impl Guest for Jaeger {
        fn sync(config_json: String) -> Result<SyncResult, SyncError> {
            let cfg: Config = if config_json.trim().is_empty() {
                Config::default()
            } else {
                serde_json::from_str(&config_json)
                    .map_err(|e| SyncError::Config(format!("invalid config_json: {e}")))?
            };

            let now = bindings::sre::inspection::clock::now_seconds();
            let base = cfg.jaeger_url.trim_end_matches('/').to_string();

            bindings::sre::inspection::logging::log(
                bindings::sre::inspection::logging::Level::Info,
                &format!(
                    "jaeger sync: url={base} cluster={} ns={} lookback={}s threshold={}",
                    cfg.cluster, cfg.namespace, cfg.lookback_seconds, cfg.call_count_threshold
                ),
            );

            let mut errors: Vec<String> = Vec::new();
            if base.is_empty() {
                errors.push("jaeger_url is empty, skipping".to_string());
                return Ok(SyncResult {
                    facts: vec![],
                    errors,
                    duration_ms: 0,
                });
            }

            // 1) /api/services —— 拿不到服务列表整轮无意义(reference 同款早退)。
            let services: Vec<String> = match get_json::<ServicesResp>(&base, "/api/services", &mut errors) {
                Some(r) => r.data,
                None => {
                    return Ok(SyncResult {
                        facts: vec![],
                        errors,
                        duration_ms: 0,
                    });
                }
            };

            // 2) 逐非内部服务 /api/traces?service=&lookback=&limit=
            let mut all_traces: Vec<mapper::Trace> = Vec::new();
            for svc in &services {
                if !mapper::is_traceable_service(svc) {
                    continue;
                }
                let path = format!(
                    "/api/traces?service={}&lookback={}&limit={}",
                    encode_component(svc),
                    cfg.lookback_seconds,
                    cfg.limit_per_service
                );
                if let Some(r) = get_json::<TracesResp>(&base, &path, &mut errors) {
                    all_traces.extend(r.data);
                }
            }

            // 3) 聚合 → CALLS 边 Fact,翻译 module_sdk::Fact → WIT Fact。
            let call_cfg = mapper::CallCfg::new(
                &cfg.cluster,
                &cfg.namespace,
                &cfg.release_prefix,
                cfg.call_count_threshold,
                now,
            );
            let facts = mapper::map_traces(&all_traces, &call_cfg)
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

    /// GET 一个 Jaeger 端点,200 → 解析 JSON;失败推 error note 返 `None`(整轮不挂)。
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

    /// 最小 percent-encoding(service 名作 query param;对照 prometheus encode_component)。
    fn encode_component(s: &str) -> String {
        let mut out = String::with_capacity(s.len() * 3);
        for &b in s.as_bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(b as char)
                }
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    }
}

#[cfg(target_arch = "wasm32")]
use imp::Jaeger;

#[cfg(target_arch = "wasm32")]
bindings::export!(Jaeger with_types_in bindings);
