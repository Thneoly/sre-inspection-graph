//! Alertmanager v2 alerts → `kind="alert"` Fact 的纯映射(host target 可单测)。
//!
//! `GET /api/v2/alerts` 的每条 alert(labels/annotations/status/startsAt/fingerprint)
//! → AlertEvent 载荷(desktop run_sync 路由进 AlertRegistry,与 k8s-events 的
//! Warning 事件产线同汇)。resource_ref 从 K8s 常见 labels(pod/deployment/service/
//! namespace/node)映射 canonical rid,无则 labels 原样拼接兜底。
//!
//! **v1 语义**:只发 `status.state == "active"` 的新 fingerprint(baseline 防重启
//! burst,与 k8s-events 同款);resolved 转好(fingerprint 从 active 变 resolved)
//! 升级为 AlertEvent status=resolved 留后 —— 当前去重按 fingerprint 一生一次。

use module_sdk::Fact;
use serde::Deserialize;

/// source 标识(Fact.source = connector 名)。
pub const SOURCE: &str = "alertmanager";
const KIND: &str = "alert";

/// Alertmanager `/api/v2/alerts` 响应(list of alert;字段取映射所需子集)。
#[derive(Deserialize, Default)]
pub struct AlertsList {
    #[serde(default)]
    pub alerts: Vec<Alert>,
}

/// 单条 alert(camelCase 字段:startsAt/endsAt;labels/annotations/fingerprint)。
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Alert {
    pub labels: Labels,
    pub annotations: Annotations,
    pub status: Status,
    pub starts_at: String,
    #[serde(default)]
    pub fingerprint: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Labels {
    pub alertname: String,
    pub severity: String,
    pub namespace: String,
    pub pod: String,
    pub deployment: String,
    pub service: String,
    pub node: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Annotations {
    pub summary: String,
    pub description: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Status {
    pub state: String,
}

/// alert 的去重键(fingerprint;缺省回退 labels 拼接)。
pub fn alert_key(a: &Alert) -> String {
    if !a.fingerprint.is_empty() {
        return a.fingerprint.clone();
    }
    format!(
        "{}|{}|{}|{}|{}",
        a.labels.alertname, a.labels.namespace, a.labels.pod, a.labels.deployment, a.starts_at
    )
}

/// labels → canonical resource_ref(K8s 常见 label 优先级:pod > deployment >
/// service > node;均无时 namespace 兜底;全空用 alertname 原样)。
///
/// **注意**:namespace label 只有名字,cluster 未知 —— rid 的 cluster 段用 config
/// 的 cluster(单集群桌面语义;alertmanager 不会跨集群)。pod/deployment label
/// 缺 namespace 时同样用 config namespace 兜底(Prometheus 规则通常都带)。
pub fn resource_ref(a: &Alert, cluster: &str, default_ns: &str) -> String {
    let ns = if a.labels.namespace.is_empty() { default_ns } else { &a.labels.namespace };
    let c = cluster;
    if !a.labels.pod.is_empty() {
        format!("pod:{c}:{}:{}", ns, a.labels.pod)
    } else if !a.labels.deployment.is_empty() {
        format!("deploy:{c}:{}:{}", ns, a.labels.deployment)
    } else if !a.labels.service.is_empty() {
        format!("svc:{c}:{}:{}", ns, a.labels.service)
    } else if !a.labels.node.is_empty() {
        format!("node:{c}:{}", a.labels.node)
    } else if !ns.is_empty() {
        format!("ns:{c}:{ns}")
    } else {
        a.labels.alertname.clone()
    }
}

/// Alertmanager severity label → AlertEvent severity(critical/warning/info;
/// 未知值归 warning —— 宁可多关注不可漏)。
pub fn map_severity(s: &str) -> &'static str {
    match s.to_ascii_lowercase().as_str() {
        "critical" | "error" | "page" => "critical",
        "info" | "none" | "debug" | "silence" => "info",
        _ => "warning",
    }
}

/// 单条 alert → alert Fact。**只发 active**(resolved 留后,见模块头)。
/// startsAt 形如 `2026-10-31T08:00:00.123456789Z`(RFC3339Nano)原样透传,
/// desktop 侧时间窗解析容忍。
pub fn alert_to_fact(a: &Alert, now: u64, cluster: &str, default_ns: &str) -> Option<Fact> {
    if a.status.state != "active" {
        return None;
    }
    let summary = if !a.annotations.summary.is_empty() {
        a.annotations.summary.chars().take(200).collect::<String>()
    } else if !a.annotations.description.is_empty() {
        a.annotations.description.chars().take(200).collect::<String>()
    } else {
        a.labels.alertname.clone()
    };
    Some(Fact {
        id: format!("{SOURCE}:alert:{}:{now}", alert_key(a)),
        kind: KIND.to_string(),
        source: SOURCE.to_string(),
        resource_id: format!("alertmanager:{}", alert_key(a)),
        resource_type: "AlertEvent".to_string(),
        timestamp: now,
        attributes_json: serde_json::json!({
            "alert_name": a.labels.alertname,
            "resource_ref": resource_ref(a, cluster, default_ns),
            "severity": map_severity(&a.labels.severity),
            "status": "firing",
            "fired_at": a.starts_at,
            "rule_id": "alertmanager",
            "summary": summary,
            "cluster_id": cluster,
        })
        .to_string(),
    })
}

/// 便捷:整批 alerts(仅 active)→ facts。
pub fn alerts_to_facts(list: &AlertsList, now: u64, cluster: &str, default_ns: &str) -> Vec<Fact> {
    list.alerts
        .iter()
        .filter_map(|a| alert_to_fact(a, now, cluster, default_ns))
        .collect()
}

#[cfg(test)]
mod tests;
