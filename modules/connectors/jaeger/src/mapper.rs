//! jaeger trace → CALLS edge Fact 的纯映射(host target 可单测)。
//!
//! 对照 reference `trace_aggregator.py`:从 Jaeger `/api/traces` 返回的 spans 里
//! 数 **跨服务 `CHILD_OF` 引用**,产出 `{caller} -CALLS-> {callee}` 的 topology-edge
//! Fact。边端点是 k8s connector 已建的 **ApplicationComponent** 节点 id(经
//! [`module_sdk::normalize_component_name`] 派生,与 k8s mapper 同源)。**只产边,
//! 不产节点** —— reference 行为,节点由 k8s connector 负责;若某服务在 k8s 无对应
//! comp 节点,边会悬空,被 host `facts_to_graph` 过滤(reference 同样悬空)。
//!
//! `FOLLOWS_FROM` 引用不数;同服务自调用(parent==child)不数;count < threshold 的
//! 服务对丢弃(对照 reference)。

use std::collections::{BTreeMap, HashMap};

use module_sdk::{component_id, normalize_component_name, Fact};
use serde::{de, Deserialize, Deserializer};

/// source 标识(写进 Fact.source + 边 discovery_method)。
pub const SOURCE: &str = "jaeger";
const KIND: &str = "topology-edge";

/// Jaeger `/api/services` 返回。
#[derive(Deserialize, Default)]
pub struct ServicesResp {
    #[serde(default)]
    pub data: Vec<String>,
}

/// Jaeger `/api/traces` 返回的最外层。
#[derive(Deserialize, Default)]
pub struct TracesResp {
    #[serde(default)]
    pub data: Vec<Trace>,
}

/// 单条 Jaeger trace。
#[derive(Deserialize, Default)]
pub struct Trace {
    #[serde(default)]
    pub spans: Vec<Span>,
    #[serde(default)]
    pub processes: HashMap<String, Process>,
}

/// Jaeger span(只取聚合需要的字段)。
#[derive(Deserialize)]
pub struct Span {
    #[serde(rename = "spanID")]
    pub span_id: String,
    #[serde(rename = "processID")]
    pub process_id: String,
    /// Jaeger 对无引用的根 span 常发 `"references": null`,用 null-safe 反序列化。
    #[serde(default, deserialize_with = "null_default")]
    pub references: Vec<SpanRef>,
}

#[derive(Deserialize)]
pub struct SpanRef {
    #[serde(rename = "refType")]
    pub ref_type: String,
    #[serde(rename = "spanID")]
    pub span_id: String,
}

#[derive(Deserialize, Default)]
pub struct Process {
    #[serde(rename = "serviceName")]
    pub service_name: String,
}

/// `null` → `T::default()`(对照 Jaeger 根 span 的 `"references": null`)。
fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + de::Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// 聚合参数。
#[derive(Clone)]
pub struct CallCfg {
    /// 集群 id(拼进 comp resource_id)。
    pub cluster: String,
    /// namespace(拼进 comp resource_id)。
    pub namespace: String,
    /// release prefix(otel-demo)。
    pub release_prefix: String,
    /// (caller,callee) 计数低于此值丢弃(对照 reference threshold,默认 5)。
    pub threshold: u64,
    /// 写进 Fact.timestamp + id 的当前秒。
    pub now: u64,
}

impl CallCfg {
    pub fn new(
        cluster: &str,
        namespace: &str,
        release_prefix: &str,
        threshold: u64,
        now: u64,
    ) -> Self {
        Self {
            cluster: cluster.to_string(),
            namespace: namespace.to_string(),
            release_prefix: release_prefix.to_string(),
            threshold,
            now,
        }
    }
}

/// OTel `service.name` → ApplicationComponent `resource_id`
/// (与 k8s connector 同一套 normalize:`{release_prefix}-{svc}` → `comp:{c}:{ns}:{short}`)。
pub fn service_to_component_id(service: &str, cfg: &CallCfg) -> String {
    if service.is_empty() {
        return String::new();
    }
    let deploy_like = format!("{}-{service}", cfg.release_prefix);
    let short = normalize_component_name(&deploy_like, &cfg.release_prefix);
    component_id(&cfg.cluster, &cfg.namespace, &short)
}

/// 数 `traces` 里跨服务 `CHILD_OF` 引用,产 CALLS 边 Fact(对照 reference
/// `aggregate_calls`)。返回的边按 `(caller, callee)` BTreeMap 字典序。
pub fn map_traces(traces: &[Trace], cfg: &CallCfg) -> Vec<Fact> {
    // (caller_service, callee_service) -> 次数
    let mut counter: BTreeMap<(String, String), u64> = BTreeMap::new();
    for trace in traces {
        // spanID -> serviceName(本 trace 内)
        let mut span_svc: HashMap<&str, &str> = HashMap::new();
        for s in &trace.spans {
            if let Some(p) = trace.processes.get(&s.process_id) {
                span_svc.insert(s.span_id.as_str(), p.service_name.as_str());
            }
        }
        for s in &trace.spans {
            let Some(&child_svc) = span_svc.get(s.span_id.as_str()) else {
                continue;
            };
            for r in &s.references {
                if r.ref_type != "CHILD_OF" {
                    continue;
                }
                let Some(&parent_svc) = span_svc.get(r.span_id.as_str()) else {
                    continue;
                };
                if parent_svc.is_empty() || child_svc.is_empty() || parent_svc == child_svc {
                    continue;
                }
                *counter
                    .entry((parent_svc.to_string(), child_svc.to_string()))
                    .or_insert(0) += 1;
            }
        }
    }

    let mut facts = Vec::new();
    for ((caller, callee), count) in counter {
        if count < cfg.threshold {
            continue;
        }
        let caller_rid = service_to_component_id(&caller, cfg);
        let callee_rid = service_to_component_id(&callee, cfg);
        if caller_rid.is_empty() || callee_rid.is_empty() {
            continue;
        }
        facts.push(calls_edge_fact(cfg.now, &caller_rid, &callee_rid, count));
    }
    facts
}

/// 构造一条 CALLS topology-edge Fact(id 含 ts 防跨轮撞;resource_id 不含 ts 作去重键,
/// 与 k8s `edge_fact` 同款)。attributes 带 `call_count_5m` + `discovery_method` provenance。
fn calls_edge_fact(now: u64, caller: &str, callee: &str, count: u64) -> Fact {
    let resource_id = format!("edge:CALLS:{caller}->{callee}");
    Fact {
        id: format!("{SOURCE}:edge:CALLS:{caller}->{callee}:{now}"),
        kind: KIND.to_string(),
        source: SOURCE.to_string(),
        resource_id,
        resource_type: "Edge".to_string(),
        timestamp: now,
        attributes_json: serde_json::json!({
            "source": caller,
            "target": callee,
            "edge_type": "CALLS",
            "call_count_5m": count,
            // provenance tag(对照 reference;不再用于 delete-on-disappear,纯溯源)。
            "discovery_method": "jaeger_connector",
        })
        .to_string(),
    }
}

/// 跳过 Jaeger 内部 / 负载生成服务(对照 reference `_is_otel_demo_service`)。
pub fn is_traceable_service(svc: &str) -> bool {
    !matches!(
        svc,
        "jaeger-all-in-one"
            | "jaeger-query"
            | "jaeger-collector"
            | "loadgenerator"
            | "load-generator"
            | "load_generator"
    )
}

#[cfg(test)]
mod tests;
