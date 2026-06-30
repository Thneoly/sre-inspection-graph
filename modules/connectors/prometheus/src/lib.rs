//! prometheus — 第三条 connector,**首个消费 `http-client` capability 的 guest**。
//!
//! 对照 reference `app/datasource/connectors/prometheus_connector.py`:跑若干
//! PromQL `GET {url}/api/v1/query?query=...`,解析 Prom JSON,把每个 sample 反查
//! 成资源 `resource_id`,产 `kind="metric"` 的 Fact 喂回 host。
//!
//! ## 与 reference 的差异(v0)
//!
//! - reference 直接写 DSS `MetricSnapshot` 并就地更新 node.properties.health;
//!   我们走 Fact 总线 —— 产 metric Fact,host 落 raw facts 表。**不**就地改
//!   topology-node 的 health(那会触发 Identity Resolver v0 的 newest-wins 把
//!   k8s connector 建的 topology 节点覆盖掉 —— field-ownership 合并是 PRD-005
//!   完整版的事,见 doc/11 §4.3)。所以本期 metric Fact 进 raw store,拓扑视图
//!   暂不渲染 metric(在 sync summary 的 Facts 表可见)。
//! - TLS / DNS / 连接池全由 host 的 `http-client` capability 实现(reqwest);
//!   guest 只发 URL 拿 bytes,deny-by-default —— manifest 必须申明 `http-client`。
//!
//! ## config_json
//!
//! ```json
//! {
//!   "prometheus_url": "http://localhost:9090",
//!   "cluster": "local",
//!   "namespace": "otel-demo",
//!   "queries": [
//!     {"name":"span_p99_ms","promql":"...","unit":"ms","target":"service"}
//!   ]
//! }
//! ```
//! - `prometheus_url` 空 → 不发请求,返一条 error note(整轮仍 success)。
//! - `cluster` 缺省 `"local"`;`namespace` 缺省 `"default"`。
//! - `queries` 缺省内置 3 条(P99 / error-rate / request-rate,对照 reference
//!   `prometheus_queries.py`)。`target` ∈ {`service`,`pod`} 决定 label 反查规则。
//!
//! host 链接器不认 wit-bindgen export symbol 里的 `:`/`@`,故全部逻辑用
//! `cfg(target_arch = "wasm32")` 守卫;host 编译时此 crate 退化为空 crate
//! (与 hello-world / k8s-mini 同款)。

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
mod imp {
    use super::bindings;
    use bindings::exports::sre::inspection::connector::{Fact, Guest, SyncError, SyncResult};
    use bindings::sre::inspection::http_client::{self, Error as HttpError};
    use serde::Deserialize;
    use serde_json::{Map, Value};

    pub struct Prometheus;

    #[derive(Deserialize)]
    struct Config {
        #[serde(default)]
        prometheus_url: String,
        #[serde(default = "default_cluster")]
        cluster: String,
        #[serde(default = "default_namespace")]
        namespace: String,
        #[serde(default = "default_queries")]
        queries: Vec<QueryDef>,
    }

    #[derive(Deserialize, Clone)]
    struct QueryDef {
        name: String,
        promql: String,
        #[serde(default)]
        unit: String,
        /// "service" | "pod" —— 决定 label → resource_id 反查规则。
        #[serde(default = "default_target")]
        target: String,
    }

    fn default_cluster() -> String {
        "local".to_string()
    }
    fn default_namespace() -> String {
        "default".to_string()
    }
    fn default_target() -> String {
        "service".to_string()
    }

    /// 内置 3 条 PromQL(对照 reference prometheus_queries.py)。
    fn default_queries() -> Vec<QueryDef> {
        vec![
            QueryDef {
                name: "span_p99_ms".to_string(),
                promql: "histogram_quantile(0.99, sum by (service_name, le) (rate(duration_milliseconds_bucket{span_kind=\"SPAN_KIND_SERVER\"}[5m])))".to_string(),
                unit: "ms".to_string(),
                target: "service".to_string(),
            },
            QueryDef {
                name: "span_error_rate_pct".to_string(),
                promql: "100 * sum by (service_name) (rate(calls_total{status_code=\"STATUS_CODE_ERROR\", span_kind=\"SPAN_KIND_SERVER\"}[5m])) / clamp_min(sum by (service_name) (rate(calls_total{span_kind=\"SPAN_KIND_SERVER\"}[5m])), 0.001)".to_string(),
                unit: "percent".to_string(),
                target: "service".to_string(),
            },
            QueryDef {
                name: "span_request_rate".to_string(),
                promql: "sum by (service_name) (rate(calls_total{span_kind=\"SPAN_KIND_SERVER\"}[5m]))".to_string(),
                unit: "req/s".to_string(),
                target: "service".to_string(),
            },
        ]
    }

    impl Default for Config {
        fn default() -> Self {
            Self {
                prometheus_url: String::new(),
                cluster: default_cluster(),
                namespace: default_namespace(),
                queries: default_queries(),
            }
        }
    }

    /// 一条 PromQL 返回的解析结果。
    #[derive(Deserialize)]
    struct PromResponse {
        #[serde(default)]
        status: String,
        #[serde(default)]
        data: PromData,
        #[serde(default)]
        error: Option<String>,
    }
    #[derive(Deserialize, Default)]
    struct PromData {
        #[serde(default)]
        result: Vec<PromSample>,
    }
    #[derive(Deserialize)]
    struct PromSample {
        #[serde(default)]
        metric: Map<String, Value>,
        /// Prom instant vector value: `[<unix_ts: number>, "<value: string>"]`。
        #[serde(default)]
        value: Vec<Value>,
    }

    impl Guest for Prometheus {
        fn sync(config_json: String) -> Result<SyncResult, SyncError> {
            let cfg: Config = if config_json.trim().is_empty() {
                Config::default()
            } else {
                serde_json::from_str(&config_json)
                    .map_err(|e| SyncError::Config(format!("invalid config_json: {e}")))?
            };

            let now = bindings::sre::inspection::clock::now_seconds();
            let base = cfg.prometheus_url.trim_end_matches('/').to_string();

            bindings::sre::inspection::logging::log(
                bindings::sre::inspection::logging::Level::Info,
                &format!(
                    "prometheus sync: url={} cluster={} ns={} queries={}",
                    base,
                    cfg.cluster,
                    cfg.namespace,
                    cfg.queries.len()
                ),
            );

            let mut facts: Vec<Fact> = Vec::new();
            let mut errors: Vec<String> = Vec::new();

            if base.is_empty() {
                errors.push("prometheus_url is empty, skipping".to_string());
                return Ok(SyncResult {
                    facts,
                    errors,
                    duration_ms: 0,
                });
            }

            for q in &cfg.queries {
                let url = format!("{base}/api/v1/query?query={}", encode_component(&q.promql));
                let body = match http_client::get(&url, &[]) {
                    Ok(resp) => {
                        if resp.status != 200 {
                            errors.push(format!("query {} HTTP {}", q.name, resp.status));
                            continue;
                        }
                        resp.body
                    }
                    Err(e) => {
                        errors.push(format!("query {} failed: {}", q.name, http_err(e)));
                        continue;
                    }
                };

                let parsed: PromResponse = match serde_json::from_slice(&body) {
                    Ok(p) => p,
                    Err(e) => {
                        errors.push(format!("query {} parse failed: {e}", q.name));
                        continue;
                    }
                };
                if parsed.status != "success" {
                    let msg = parsed.error.unwrap_or_else(|| "unknown".to_string());
                    errors.push(format!("query {} prom error: {msg}", q.name));
                    continue;
                }

                for sample in &parsed.data.result {
                    let Some(resource_id) = resolve_target(&q.target, &sample.metric, &cfg) else {
                        continue;
                    };
                    let Some(value) = sample_value(&sample.value) else {
                        continue;
                    };
                    let resource_type = if q.target == "pod" { "Pod" } else { "Service" };
                    facts.push(Fact {
                        id: format!("prometheus:{}:{resource_id}:{now}", q.name),
                        kind: "metric".to_string(),
                        source: "prometheus".to_string(),
                        resource_id,
                        resource_type: resource_type.to_string(),
                        timestamp: now,
                        attributes_json: metric_attrs(&q.name, value, &q.unit, &sample.metric),
                    });
                }
            }

            Ok(SyncResult {
                facts,
                errors,
                duration_ms: 0,
            })
        }

        fn health_check() -> bool {
            // 静态返 true —— 无 config(health_check 不带参),真探活需 url。
            // Phase 3 可加 health-check 带 config 的 WIT 变体走 GET /-/healthy。
            true
        }
    }

    /// label → resource_id 反查(对照 reference `_resolve_target_id`)。
    fn resolve_target(target: &str, labels: &Map<String, Value>, cfg: &Config) -> Option<String> {
        match target {
            "service" => {
                let svc = labels.get("service_name").and_then(Value::as_str)?;
                if svc.is_empty() {
                    return None;
                }
                Some(format!("service:{}:{}:{svc}", cfg.cluster, cfg.namespace))
            }
            "pod" => {
                let pod = labels.get("pod").and_then(Value::as_str)?;
                if pod.is_empty() {
                    return None;
                }
                let ns = labels
                    .get("namespace")
                    .and_then(Value::as_str)
                    .unwrap_or(&cfg.namespace);
                Some(format!("pod:{}:{ns}:{pod}", cfg.cluster))
            }
            _ => None,
        }
    }

    /// Prom instant value `[ts, "floatstr"]` → f64。
    fn sample_value(value: &[Value]) -> Option<f64> {
        let s = value.get(1)?.as_str()?;
        let v: f64 = s.parse().ok()?;
        if v.is_nan() {
            return None;
        }
        Some(v)
    }

    /// 组 metric Fact 的 attributes_json。labels 原样带上,便于诊断。
    fn metric_attrs(metric: &str, value: f64, unit: &str, labels: &Map<String, Value>) -> String {
        let mut m = Map::new();
        m.insert("metric".to_string(), Value::String(metric.to_string()));
        m.insert(
            "value".to_string(),
            serde_json::Number::from_f64(value)
                .map(Value::Number)
                .unwrap_or(Value::Null),
        );
        m.insert("unit".to_string(), Value::String(unit.to_string()));
        m.insert("labels".to_string(), Value::Object(labels.clone()));
        Value::Object(m).to_string()
    }

    /// http-client error → 人读字符串。
    fn http_err(e: HttpError) -> String {
        match e {
            HttpError::Unauthorized => "unauthorized (capability denied?)".to_string(),
            HttpError::NotFound => "not found".to_string(),
            HttpError::Network(m) => format!("network: {m}"),
            HttpError::Timeout => "timeout".to_string(),
        }
    }

    /// 最小 percent-encoding(RFC 3986 unreserved 不转义,其余 %XX)。
    /// 不引 url crate 以保持 wasm 体积小。
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
use imp::Prometheus;

#[cfg(target_arch = "wasm32")]
bindings::export!(Prometheus with_types_in bindings);
