//! mapper 纯函数单测(host target,CI-safe)。fixture 形态对齐 Alertmanager v2
//! `/api/v2/alerts` 真实响应(camelCase:startsAt/endsAt/fingerprint)。

use super::*;
use module_sdk::Fact;

fn alert(labels: serde_json::Value, state: &str) -> Alert {
    serde_json::from_value(serde_json::json!({
        "labels": labels,
        "annotations": { "summary": "High error rate" },
        "status": { "state": state },
        "startsAt": "2026-10-31T08:00:00.123456789Z",
        "endsAt": "0001-01-01T00:00:00Z",
        "fingerprint": "abc123"
    }))
    .unwrap()
}

fn attr_str(f: &Fact, key: &str) -> String {
    let m: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&f.attributes_json).unwrap();
    m.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

#[test]
fn active_alert_maps_to_fact_with_labels_mapped() {
    let a = alert(
        serde_json::json!({
            "alertname": "KubePodCrashLooping",
            "severity": "critical",
            "namespace": "otel-demo",
            "pod": "cart-abc123"
        }),
        "active",
    );
    let f = alert_to_fact(&a, 1_700_000_000, "vm", "monitoring").expect("fact");
    assert_eq!(f.kind, "alert");
    assert_eq!(f.source, "alertmanager");
    // id 不含时间戳(fingerprint 稳定)—— desktop alert_event_id 幂等,重启不重复
    assert_eq!(f.id, "alertmanager:alert:abc123");
    assert_eq!(f.resource_id, "alertmanager:abc123");
    assert_eq!(attr_str(&f, "alert_name"), "KubePodCrashLooping");
    assert_eq!(attr_str(&f, "resource_ref"), "pod:vm:otel-demo:cart-abc123");
    assert_eq!(attr_str(&f, "severity"), "critical");
    assert_eq!(attr_str(&f, "status"), "firing");
    assert_eq!(attr_str(&f, "rule_id"), "alertmanager");
    assert_eq!(attr_str(&f, "fired_at"), "2026-10-31T08:00:00.123456789Z");
    assert_eq!(attr_str(&f, "summary"), "High error rate");
}

#[test]
fn resolved_alert_skipped_v1() {
    let a = alert(serde_json::json!({ "alertname": "X" }), "resolved");
    assert!(alert_to_fact(&a, 1, "vm", "monitoring").is_none());
}

#[test]
fn resource_ref_priority_pod_over_deployment_over_service() {
    // pod 优先
    let both = alert(
        serde_json::json!({ "alertname": "A", "pod": "p1", "deployment": "d1", "service": "s1", "namespace": "ns" }),
        "active",
    );
    assert_eq!(resource_ref(&both, "c", "fallback"), "pod:c:ns:p1");
    // 无 pod → deployment
    let dep = alert(
        serde_json::json!({ "alertname": "A", "deployment": "d1", "service": "s1", "namespace": "ns" }),
        "active",
    );
    assert_eq!(resource_ref(&dep, "c", "fallback"), "deploy:c:ns:d1");
    // 无 pod/deploy → service
    let svc = alert(serde_json::json!({ "alertname": "A", "service": "s1", "namespace": "ns" }), "active");
    assert_eq!(resource_ref(&svc, "c", "fallback"), "svc:c:ns:s1");
    // node 无 ns 段
    let node = alert(serde_json::json!({ "alertname": "A", "node": "n1" }), "active");
    assert_eq!(resource_ref(&node, "c", "fallback"), "node:c:n1");
    // namespace 兜底(labels 无任何资源 label)
    let ns_only = alert(serde_json::json!({ "alertname": "A", "namespace": "ns" }), "active");
    assert_eq!(resource_ref(&ns_only, "c", "fallback"), "ns:c:ns");
    // labels 缺 namespace → config ns 兜底
    let no_ns = alert(serde_json::json!({ "alertname": "A", "pod": "p" }), "active");
    assert_eq!(resource_ref(&no_ns, "c", "monitoring"), "pod:c:monitoring:p");
}

#[test]
fn severity_mapping() {
    assert_eq!(map_severity("critical"), "critical");
    assert_eq!(map_severity("error"), "critical");
    assert_eq!(map_severity("warning"), "warning");
    assert_eq!(map_severity("info"), "info");
    assert_eq!(map_severity("totally-unknown"), "warning"); // 未知归 warning
}

#[test]
fn alert_key_prefers_fingerprint() {
    let a = alert(serde_json::json!({ "alertname": "A" }), "active");
    assert_eq!(alert_key(&a), "abc123");
    // fingerprint 缺省回退 labels 拼接
    let mut b = alert(serde_json::json!({ "alertname": "B", "namespace": "ns" }), "active");
    b.fingerprint = String::new();
    let k = alert_key(&b);
    assert!(k.starts_with("B|ns|"), "fallback key: {k}");
}

#[test]
fn summary_falls_back_to_description_then_alertname() {
    let mut a = alert(serde_json::json!({ "alertname": "MyAlert" }), "active");
    a.annotations.summary = String::new();
    a.annotations.description = "from description".into();
    let f = alert_to_fact(&a, 1, "c", "ns").unwrap();
    assert_eq!(attr_str(&f, "summary"), "from description");
    a.annotations.description = String::new();
    let f2 = alert_to_fact(&a, 1, "c", "ns").unwrap();
    assert_eq!(attr_str(&f2, "summary"), "MyAlert");
}

#[test]
fn batch_filters_active_only() {
    let list = AlertsList(vec![
        alert(serde_json::json!({ "alertname": "A" }), "active"),
        alert(serde_json::json!({ "alertname": "B" }), "suppressed"),
        alert(serde_json::json!({ "alertname": "C" }), "active"),
    ]);
    let facts = alerts_to_facts(&list, 1, "c", "ns");
    assert_eq!(facts.len(), 2);
}

#[test]
fn parses_top_level_array_response() {
    // Alertmanager v2 /api/v2/alerts 返回**顶层数组**(非 {"alerts":[...]} 包裹)
    // —— 首版 AlertsList 用对象包裹反序列化静默失败(0 facts),真数据抓出。
    let raw = serde_json::json!([
        { "labels": { "alertname": "A", "severity": "warning" }, "annotations": {},
          "status": { "state": "active" }, "startsAt": "2026-11-01T00:00:00Z",
          "fingerprint": "f1" }
    ]);
    let list: AlertsList = serde_json::from_value(raw).expect("top-level array parses");
    assert_eq!(list.0.len(), 1);
    assert_eq!(list.0[0].labels.alertname, "A");
    // 对象包裹形态应失败(防止再退回错误假设)
    assert!(serde_json::from_value::<AlertsList>(serde_json::json!({ "alerts": [] })).is_err());
}
