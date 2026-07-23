//! mapper 纯函数单测 —— 移植 reference `TestTraceAggregator`(7 例)+ 边形状 + service 过滤。
//! 全部用合成 Jaeger trace(host target,CI-safe),不碰网络。

use super::*;
use module_sdk::Fact;

fn cfg(threshold: u64) -> CallCfg {
    CallCfg::new("vm-cluster", "otel-demo", "otel-demo", threshold, 1_700_000_000)
}

/// 对照 reference `_trace(edges_per_pair)`:每对 `(parent_svc, child_svc, count)`
/// → 1 个 parent span(无 ref)+ `count` 个 child span(各带一条 CHILD_OF→parent)。
/// 不同对用独立 process id + 唯一 span id。
fn build_trace(pairs: &[(&str, &str, u64)]) -> Trace {
    let mut spans: Vec<Span> = Vec::new();
    let mut processes: HashMap<String, Process> = HashMap::new();
    let mut pid = 0u32;
    let mut sid = 0u32;
    for &(parent_svc, child_svc, count) in pairs {
        let parent_pid = format!("p{pid}");
        pid += 1;
        let child_pid = format!("p{pid}");
        pid += 1;
        processes.insert(
            parent_pid.clone(),
            Process {
                service_name: parent_svc.to_string(),
            },
        );
        processes.insert(
            child_pid.clone(),
            Process {
                service_name: child_svc.to_string(),
            },
        );
        let parent_sid = format!("s{sid}");
        sid += 1;
        spans.push(Span {
            span_id: parent_sid.clone(),
            process_id: parent_pid,
            references: Vec::new(),
        });
        for _ in 0..count {
            let child_sid = format!("s{sid}");
            sid += 1;
            spans.push(Span {
                span_id: child_sid,
                process_id: child_pid.clone(),
                references: vec![SpanRef {
                    ref_type: "CHILD_OF".to_string(),
                    span_id: parent_sid.clone(),
                }],
            });
        }
    }
    Trace { spans, processes }
}

/// 一条 FOLLOWS_FROM 的 trace(对照 reference test_non_child_of_reference_ignored)。
fn follows_from_trace(parent_svc: &str, child_svc: &str, count: u64) -> Trace {
    let mut t = build_trace(&[(parent_svc, child_svc, count)]);
    for s in &mut t.spans {
        for r in &mut s.references {
            r.ref_type = "FOLLOWS_FROM".to_string();
        }
    }
    t
}

/// 读 Fact.attributes_json 的某 key。
fn attr(f: &Fact, key: &str) -> serde_json::Value {
    let m: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&f.attributes_json).unwrap();
    m.get(key).cloned().unwrap_or(serde_json::Value::Null)
}

#[test]
fn above_threshold_creates_edge() {
    let traces = vec![build_trace(&[("frontend", "cartservice", 6)])];
    let facts = map_traces(&traces, &cfg(5));
    assert_eq!(facts.len(), 1);
    let f = &facts[0];
    assert_eq!(f.kind, "topology-edge");
    assert_eq!(f.source, "jaeger");
    assert_eq!(f.resource_type, "Edge");
    assert_eq!(attr(f, "edge_type"), serde_json::json!("CALLS"));
    assert_eq!(attr(f, "source"), serde_json::json!("comp:vm-cluster:otel-demo:frontend"));
    assert_eq!(attr(f, "target"), serde_json::json!("comp:vm-cluster:otel-demo:cart"));
    assert_eq!(attr(f, "call_count_5m"), serde_json::json!(6));
    assert_eq!(f.resource_id, "edge:CALLS:comp:vm-cluster:otel-demo:frontend->comp:vm-cluster:otel-demo:cart");
}

#[test]
fn below_threshold_filtered() {
    let traces = vec![build_trace(&[("frontend", "cartservice", 3)])];
    let facts = map_traces(&traces, &cfg(5));
    assert!(facts.is_empty());
}

#[test]
fn self_call_excluded() {
    let traces = vec![build_trace(&[("cartservice", "cartservice", 10)])];
    let facts = map_traces(&traces, &cfg(5));
    assert!(facts.is_empty());
}

#[test]
fn multiple_pairs_aggregated_across_traces() {
    // 两条 trace,frontend→cart 计数跨 trace 相加(3+4=7);checkout→payment 单 trace 6 次。
    let traces = vec![
        build_trace(&[("frontend", "cartservice", 3), ("checkoutservice", "paymentservice", 6)]),
        build_trace(&[("frontend", "cartservice", 4)]),
    ];
    let facts = map_traces(&traces, &cfg(5));
    assert_eq!(facts.len(), 2);
    let by_target: std::collections::HashMap<String, &Fact> =
        facts.iter().map(|f| (attr(f, "target").as_str().unwrap().to_string(), f)).collect();
    let cart = by_target.get("comp:vm-cluster:otel-demo:cart").expect("cart edge");
    assert_eq!(attr(cart, "call_count_5m"), serde_json::json!(7));
    assert_eq!(attr(cart, "source"), serde_json::json!("comp:vm-cluster:otel-demo:frontend"));
    let payment = by_target.get("comp:vm-cluster:otel-demo:payment").expect("payment edge");
    assert_eq!(attr(payment, "call_count_5m"), serde_json::json!(6));
    assert_eq!(attr(payment, "source"), serde_json::json!("comp:vm-cluster:otel-demo:checkout"));
}

#[test]
fn service_to_component_id_normalization() {
    let c = cfg(1);
    assert_eq!(service_to_component_id("cartservice", &c), "comp:vm-cluster:otel-demo:cart");
    assert_eq!(service_to_component_id("frontend", &c), "comp:vm-cluster:otel-demo:frontend");
    assert_eq!(service_to_component_id("adservice", &c), "comp:vm-cluster:otel-demo:ad");
    assert_eq!(service_to_component_id("", &c), "");
}

#[test]
fn empty_traces() {
    let facts = map_traces(&[], &cfg(5));
    assert!(facts.is_empty());
}

#[test]
fn follows_from_ignored() {
    let traces = vec![follows_from_trace("frontend", "cartservice", 10)];
    // threshold 1:只要有计数就会出边 —— FOLLOWS_FROM 不计数,故 0 边。
    let facts = map_traces(&traces, &cfg(1));
    assert!(facts.is_empty());
}

#[test]
fn null_references_does_not_panic() {
    // Jaeger 根 span 常发 "references": null —— 反序列化须容错。
    let json = r#"{"spans":[{"spanID":"a","processID":"p1","references":null}],"processes":{"p1":{"serviceName":"frontend"}}}"#;
    let trace: Trace = serde_json::from_str(json).unwrap();
    assert_eq!(trace.spans.len(), 1);
    assert!(trace.spans[0].references.is_empty());
    // 空 references 的单服务 trace → 无边。
    assert!(map_traces(&[trace], &cfg(1)).is_empty());
}

#[test]
fn is_traceable_service_filters_internals() {
    assert!(!is_traceable_service("jaeger-query"));
    assert!(!is_traceable_service("jaeger-all-in-one"));
    assert!(!is_traceable_service("loadgenerator"));
    assert!(is_traceable_service("cartservice"));
    assert!(is_traceable_service("frontend"));
}

#[test]
fn edge_id_carries_timestamp_dedup_key_does_not() {
    // id 含 ts(跨轮不撞);resource_id 不含 ts(去重键,与 k8s edge_fact 同款)。
    let traces = vec![build_trace(&[("frontend", "cartservice", 6)])];
    let f = &map_traces(&traces, &cfg(5))[0];
    assert!(f.id.ends_with(":1700000000"));
    assert!(!f.resource_id.contains("1700000000"));
}
