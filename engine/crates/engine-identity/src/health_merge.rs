//! Metric -> topology health 合并(doc/11 §4.3 field-ownership,v0)。
//!
//! Prometheus connector 产 `kind="metric"` Fact(`attributes_json = {metric, value,
//! unit, labels}`),其 `resource_id` 与 topology 节点对齐(`service:{c}:{ns}:{name}`
//! / `pod:...`)。本模块把 metric value 按**阈值**推成 `health_status`,再按
//! field-ownership 把它合进 topology 节点的 `health` 字段 -- 让 prometheus 采集
//! 到的高错误率 / 高延迟反映到拓扑节点的 health 配色上。
//!
//! ## v0 仲裁规则(简化版,非 doc/11 §5 完整版)
//!
//! doc/11 §4.3:`health: source=[prometheus, cloud_api]  # 多源,新覆盖旧`。
//! 完整版用 confidence + timestamp 仲裁(doc/11 §5 Identity Resolver 算法)。
//! v0 取**最严重胜出**(critical > warning > normal > unknown):
//!
//! - 巡检工具偏向暴露问题 -- 一个 CrashLoopBackOff(k8s critical)或一个高错误率
//!   (prometheus critical)都是真实信号,取更严重者避免陈旧 normal 掩盖故障。
//! - 同一资源多条 metric(错误率 + P99)取最严重;同严重度取更新 timestamp。
//! - k8s phase health 与 prometheus metric health 取更严重者。
//!
//! PRD-005 完整版(doc/11 §5)将替换为 confidence + observed_at 仲裁 + 冲突人工队列。
//!
//! ## 纯领域逻辑
//!
//! 本模块 I/O-free:吃 `&Topology` + `&[Fact]`(metric),产新 `Topology`。
//! 可单测;orchestration 层(Tauri `sync_all_now`)调 [`merge_metric_health`]。

use std::collections::BTreeMap;

use engine_core::Fact;
use serde_json::{Map, Value};

use crate::topology::Topology;

/// 单个 metric 的健康阈值。
///
/// `value >= critical` -> critical;`value >= warning` -> warning;否则 normal。
/// 阈值语义为「越差越大」(错误率 % / 延迟 ms),与内置 prometheus metric 一致。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HealthThreshold {
    /// 达到此值(含)判 warning。
    pub warning: f64,
    /// 达到此值(含)判 critical(应 > warning)。
    pub critical: f64,
}

/// metric name -> 阈值表。缺省 [`HealthRules::default_prometheus`] 三条内置 metric。
#[derive(Debug, Clone)]
pub struct HealthThresholds(pub BTreeMap<String, HealthThreshold>);

impl HealthThresholds {
    /// 内置 prometheus connector 的 3 条 metric 阈值(对照 prometheus connector
    /// `default_queries`)。`span_request_rate` 为信息性指标,不推 health,故不入表。
    ///
    /// - `span_error_rate_pct`(百分比):>=5 critical,>=1 warning
    /// - `span_p99_ms`(毫秒):>=1000 critical,>=300 warning
    pub fn default_prometheus() -> Self {
        let mut m = BTreeMap::new();
        m.insert(
            "span_error_rate_pct".to_string(),
            HealthThreshold {
                warning: 1.0,
                critical: 5.0,
            },
        );
        m.insert(
            "span_p99_ms".to_string(),
            HealthThreshold {
                warning: 300.0,
                critical: 1000.0,
            },
        );
        Self(m)
    }

    /// 查 metric name 的阈值。
    pub fn get(&self, metric: &str) -> Option<&HealthThreshold> {
        self.0.get(metric)
    }
}

impl Default for HealthThresholds {
    fn default() -> Self {
        Self::default_prometheus()
    }
}

/// 一条 metric Fact 推导出的 health 信号(尚未合进 topology)。
#[derive(Debug, Clone, PartialEq)]
pub struct DerivedHealth {
    /// `normal` / `warning` / `critical`。
    pub health_status: String,
    /// 由 health 映射:`critical->high` / `warning->medium` / `normal->low`。
    pub risk_level: String,
    /// metric Fact 的 timestamp(仲裁同严重度时取新)。
    pub timestamp: u64,
}

/// 把一条 metric Fact 推成 [`DerivedHealth`]。
///
/// 非 metric fact / `attributes_json` 非 object / 缺 `metric` 或 `value` /
/// metric 不在阈值表 -> 返 `None`(该 fact 不贡献 health)。
pub fn derive_metric_health(fact: &Fact, thresholds: &HealthThresholds) -> Option<DerivedHealth> {
    if fact.kind != "metric" {
        return None;
    }
    let attrs: Value = serde_json::from_str(&fact.attributes_json).ok()?;
    let metric = attrs.get("metric").and_then(Value::as_str)?;
    let value = attrs.get("value").and_then(Value::as_f64)?;
    if value.is_nan() {
        return None;
    }
    let thr = thresholds.get(metric)?;
    let health = if value >= thr.critical {
        "critical"
    } else if value >= thr.warning {
        "warning"
    } else {
        "normal"
    };
    Some(DerivedHealth {
        health_status: health.to_string(),
        risk_level: risk_for_health(health).to_string(),
        timestamp: fact.timestamp,
    })
}

/// 把 metric-derived health 合进 topology 节点的 `health_status` / `risk_level`。
///
/// v0 仲裁 = **最严重胜出**(见模块顶部说明):
/// - 同一 `resource_id` 多条 metric -> 取最严重;同严重度取更新 timestamp。
/// - 节点既有 health(k8s phase)与 metric-derived health -> 取更严重者。
/// - 节点无对应 metric fact -> 原样保留。
///
/// 返回的新 `Topology` 节点 `attributes_json` 仍为 canonical(sorted-key)字符串,
/// 与 [`crate::resolve`] 产出一致,便于 [`crate::diff`] 按字符串相等判定变化。
pub fn merge_metric_health(
    topology: &Topology,
    metric_facts: &[Fact],
    thresholds: &HealthThresholds,
) -> Topology {
    // 1. resource_id -> worst DerivedHealth
    let mut by_resource: BTreeMap<String, DerivedHealth> = BTreeMap::new();
    for f in metric_facts {
        let Some(d) = derive_metric_health(f, thresholds) else {
            continue;
        };
        match by_resource.get(&f.resource_id) {
            Some(prev) => {
                let prev_rank = severity_rank(&prev.health_status);
                let new_rank = severity_rank(&d.health_status);
                // 更严重 -> 取;同严重度且更新 -> 取
                let take = new_rank > prev_rank
                    || (new_rank == prev_rank && d.timestamp > prev.timestamp);
                if take {
                    by_resource.insert(f.resource_id.clone(), d);
                }
            }
            None => {
                by_resource.insert(f.resource_id.clone(), d);
            }
        }
    }

    // 2. 叠加到每个 topology 节点(worst-wins vs 既有 health)
    let nodes = topology
        .nodes
        .iter()
        .map(|n| {
            let Some(d) = by_resource.get(&n.resource_id) else {
                return n.clone();
            };
            let mut attrs: Map<String, Value> =
                serde_json::from_str(&n.attributes_json).unwrap_or_else(|_| Map::new());
            let existing = attrs
                .get("health_status")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            // worst-wins:metric-derived 与节点既有 health 取更严重者。
            let (final_health, final_risk): (String, String) =
                if severity_rank(&d.health_status) >= severity_rank(&existing) {
                    (d.health_status.clone(), d.risk_level.clone())
                } else {
                    (
                        existing,
                        attrs
                            .get("risk_level")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .to_string(),
                    )
                };
            attrs.insert(
                "health_status".to_string(),
                Value::String(final_health),
            );
            attrs.insert("risk_level".to_string(), Value::String(final_risk));
            let mut cloned = n.clone();
            // serde_json Map = BTreeMap(无 preserve_order feature)-> 序列化 key 有序,
            // 与 resolve 的 attributes_json 形态一致。
            cloned.attributes_json = Value::Object(attrs).to_string();
            cloned
        })
        .collect();

    Topology {
        nodes,
        edges: topology.edges.clone(),
    }
}

/// health 字符串 -> 严重度序(越大越严重)。意外值视作 0(不覆盖既有)。
fn severity_rank(s: &str) -> u8 {
    match s {
        "critical" => 3,
        "warning" => 2,
        "normal" => 1,
        _ => 0,
    }
}

/// health -> risk 映射(与 k8s mapper `risk_from_health` 一致)。
fn risk_for_health(health: &str) -> &'static str {
    match health {
        "critical" => "high",
        "warning" => "medium",
        "normal" => "low",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topology::{ResolvedNode, Topology};
    use serde_json::json;

    fn metric_fact(resource_id: &str, metric: &str, value: f64, ts: u64) -> Fact {
        Fact::new(
            format!("prometheus:{metric}:{resource_id}:{ts}"),
            "metric",
            "prometheus",
            resource_id,
            "Service",
            ts,
            json!({ "metric": metric, "value": value, "unit": "x", "labels": {} }).to_string(),
        )
    }

    fn node(resource_id: &str, health: &str, risk: &str) -> ResolvedNode {
        ResolvedNode {
            resource_id: resource_id.into(),
            resource_type: "Service".into(),
            label: resource_id.into(),
            attributes_json: json!({ "health_status": health, "risk_level": risk }).to_string(),
        }
    }

    fn topology(nodes: Vec<ResolvedNode>) -> Topology {
        Topology {
            nodes,
            edges: vec![],
        }
    }

    #[test]
    fn derive_threshold_boundaries() {
        let t = HealthThresholds::default_prometheus();
        // error_rate: >=5 critical, >=1 warning
        assert_eq!(
            derive_metric_health(
                &metric_fact("svc:a", "span_error_rate_pct", 0.5, 1),
                &t
            )
            .unwrap()
            .health_status,
            "normal"
        );
        assert_eq!(
            derive_metric_health(
                &metric_fact("svc:a", "span_error_rate_pct", 1.0, 1),
                &t
            )
            .unwrap()
            .health_status,
            "warning"
        );
        assert_eq!(
            derive_metric_health(
                &metric_fact("svc:a", "span_error_rate_pct", 5.0, 1),
                &t
            )
            .unwrap()
            .health_status,
            "critical"
        );
        // p99: >=1000 critical, >=300 warning
        assert_eq!(
            derive_metric_health(&metric_fact("svc:a", "span_p99_ms", 300.0, 1), &t)
                .unwrap()
                .health_status,
            "warning"
        );
        assert_eq!(
            derive_metric_health(&metric_fact("svc:a", "span_p99_ms", 999.0, 1), &t)
                .unwrap()
                .health_status,
            "warning"
        );
        assert_eq!(
            derive_metric_health(&metric_fact("svc:a", "span_p99_ms", 1000.0, 1), &t)
                .unwrap()
                .health_status,
            "critical"
        );
    }

    #[test]
    fn derive_returns_none_for_unmapped_or_malformed() {
        let t = HealthThresholds::default_prometheus();
        // span_request_rate 不在阈值表 -> None
        assert!(derive_metric_health(
            &metric_fact("svc:a", "span_request_rate", 42.0, 1),
            &t
        )
        .is_none());
        // 非 metric kind -> None
        let mut f = metric_fact("svc:a", "span_p99_ms", 300.0, 1);
        f.kind = "topology-node".into();
        assert!(derive_metric_health(&f, &t).is_none());
        // 缺 value -> None
        let bad = Fact::new(
            "bad",
            "metric",
            "prometheus",
            "svc:a",
            "Service",
            1,
            json!({ "metric": "span_p99_ms" }).to_string(),
        );
        assert!(derive_metric_health(&bad, &t).is_none());
    }

    #[test]
    fn merge_prom_warning_overlays_k8s_normal() {
        let topo = topology(vec![node("svc:a", "normal", "low")]);
        let metrics = vec![metric_fact("svc:a", "span_error_rate_pct", 2.0, 100)];
        let merged = merge_metric_health(&topo, &metrics, &HealthThresholds::default());
        assert_eq!(merged.nodes[0].attributes_json, r#"{"health_status":"warning","risk_level":"medium"}"#);
    }

    #[test]
    fn merge_does_not_downgrade_k8s_critical() {
        // k8s says critical(crashloop),prom says normal(无流量无错误)-> 仍 critical(worst-wins)
        let topo = topology(vec![node("svc:a", "critical", "high")]);
        let metrics = vec![metric_fact("svc:a", "span_error_rate_pct", 0.1, 100)];
        let merged = merge_metric_health(&topo, &metrics, &HealthThresholds::default());
        assert_eq!(merged.nodes[0].attributes_json, r#"{"health_status":"critical","risk_level":"high"}"#);
    }

    #[test]
    fn merge_takes_worst_across_multiple_metrics() {
        // 同资源:error_rate normal + p99 critical -> critical
        let topo = topology(vec![node("svc:a", "normal", "low")]);
        let metrics = vec![
            metric_fact("svc:a", "span_error_rate_pct", 0.1, 100),
            metric_fact("svc:a", "span_p99_ms", 1200.0, 100),
        ];
        let merged = merge_metric_health(&topo, &metrics, &HealthThresholds::default());
        assert_eq!(merged.nodes[0].attributes_json, r#"{"health_status":"critical","risk_level":"high"}"#);
    }

    #[test]
    fn merge_no_metric_leaves_node_unchanged() {
        let topo = topology(vec![node("svc:a", "normal", "low")]);
        let merged = merge_metric_health(&topo, &[], &HealthThresholds::default());
        assert_eq!(merged, topo);
    }

    #[test]
    fn merge_canonicalizes_attributes_json_sorted_keys() {
        // 节点带乱序额外字段 -> 合并后 key 仍有序
        let n = ResolvedNode {
            resource_id: "svc:a".into(),
            resource_type: "Service".into(),
            label: "a".into(),
            attributes_json: json!({ "zone": "z1", "health_status": "normal", "risk_level": "low" })
                .to_string(),
        };
        let topo = Topology {
            nodes: vec![n],
            edges: vec![],
        };
        let metrics = vec![metric_fact("svc:a", "span_p99_ms", 400.0, 100)];
        let merged = merge_metric_health(&topo, &metrics, &HealthThresholds::default());
        // key 字典序:health_status < risk_level < zone
        assert_eq!(
            merged.nodes[0].attributes_json,
            r#"{"health_status":"warning","risk_level":"medium","zone":"z1"}"#
        );
    }

    #[test]
    fn merge_only_affects_nodes_with_matching_metric() {
        let topo = topology(vec![
            node("svc:a", "normal", "low"),
            node("svc:b", "normal", "low"),
        ]);
        let metrics = vec![metric_fact("svc:a", "span_p99_ms", 1200.0, 100)];
        let merged = merge_metric_health(&topo, &metrics, &HealthThresholds::default());
        assert_eq!(
            merged.nodes[0].attributes_json,
            r#"{"health_status":"critical","risk_level":"high"}"#
        );
        // svc:b 无 metric -> 原样
        assert_eq!(
            merged.nodes[1].attributes_json,
            r#"{"health_status":"normal","risk_level":"low"}"#
        );
    }
}
