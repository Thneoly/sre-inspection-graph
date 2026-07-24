//! flagd flag 快照 → `kind="change"` Fact 的纯映射(host target 可单测)。
//!
//! 对照 reference `flagd_connector.py` + `otel_demo_scenarios.py`:POST `ResolveAll`
//! 拿 `{flag_name: state}` 快照,diff 上一轮(`changed`/`added`/`removed`),每条 delta
//! 产一个 `configmap_updated` ChangeEvent 载荷(写进 `kind="change"` Fact 的 attributes_json;
//! desktop run_sync 路由到 `record_change`),target = flagd ConfigMap 节点。翻转的 flag 若
//! 命中 **OTel Demo 8 故障 scenario**,把 `scenario.{recommended_action,target_component,…}`
//! 塞进 `diff_summary` + description 后缀。**不产节点/边/metric/alert**(reference 行为)。
//! 首次 sync baseline + 跨轮 diff 是有状态逻辑,在 `lib.rs`(guest static)。

use std::collections::BTreeMap;

use module_sdk::Fact;
use serde::Deserialize;
use serde_json::Value;

/// source 标识(Fact.source = connector 名)。
pub const SOURCE: &str = "flagd";
/// ChangeRequest.source(record_change 校验 -> Source enum;对照 reference `flagd`)。
pub const CHANGE_SOURCE: &str = "flagd";
const KIND: &str = "change";

/// flagd `ResolveAll` 响应:`{flags: {name: state}}`。
#[derive(Deserialize, Default)]
pub struct ResolveAllResp {
    #[serde(default)]
    pub flags: Snapshot,
}

/// 一份 flag 快照:`flag_name -> state`(state: `{variant, *Value}`)。
pub type Snapshot = BTreeMap<String, Value>;

// ─────────────────────────────────────────────────────────────
//  OTel Demo 8 故障 scenario 表(对照 reference otel_demo_scenarios.py,逐字)
// ─────────────────────────────────────────────────────────────

/// 一个 OTel Demo fault scenario 的元数据(对照 reference `FaultScenario`)。
#[derive(Debug, Clone)]
pub struct FaultScenario {
    pub name: &'static str,
    pub flag_name: &'static str,
    pub target_component: &'static str,
    pub expected_metric: &'static str,
    pub finding_rule: &'static str,
    pub finding_severity: &'static str, // warning | critical
    pub recommended_action: &'static str, // PRD-001 action_id
    pub description: &'static str,
}

pub const SCENARIOS: &[FaultScenario] = &[
    FaultScenario {
        name: "product_catalog_failure",
        flag_name: "productCatalogFailure",
        target_component: "product-catalog",
        expected_metric: "span_error_rate_pct",
        finding_rule: "HTTP_5XX_HIGH",
        finding_severity: "critical",
        recommended_action: "restart_pod",
        description: "product-catalog 返回 5xx,推荐重启 Pod 复位状态",
    },
    FaultScenario {
        name: "recommendation_cache_failure",
        flag_name: "recommendationServiceCacheFailure",
        target_component: "recommendation",
        expected_metric: "span_p99_ms",
        finding_rule: "MEM_HIGH",
        finding_severity: "warning",
        recommended_action: "restart_pod",
        description: "recommendation 内存泄漏,缓存不停增长。重启清理",
    },
    FaultScenario {
        name: "ad_manual_gc",
        flag_name: "adServiceManualGc",
        target_component: "ad",
        expected_metric: "span_p99_ms",
        finding_rule: "P99_HIGH",
        finding_severity: "warning",
        recommended_action: "rollback_deployment",
        description: "ad 手动触发 GC,P99 周期性飙升。回滚到无 GC 版本",
    },
    FaultScenario {
        name: "ad_high_cpu",
        flag_name: "adServiceHighCpu",
        target_component: "ad",
        expected_metric: "span_p99_ms",
        finding_rule: "CPU_HIGH",
        finding_severity: "critical",
        recommended_action: "scale_deployment",
        description: "ad CPU 飙满,扩容应对",
    },
    FaultScenario {
        name: "cart_failure",
        flag_name: "cartServiceFailure",
        target_component: "cart",
        expected_metric: "span_error_rate_pct",
        finding_rule: "HTTP_5XX_HIGH",
        finding_severity: "critical",
        recommended_action: "clear_cache",
        description: "cart 写入失败,清 Valkey 缓存复位",
    },
    FaultScenario {
        name: "payment_failure",
        flag_name: "paymentServiceFailure",
        target_component: "payment",
        expected_metric: "span_error_rate_pct",
        finding_rule: "HTTP_5XX_HIGH",
        finding_severity: "critical",
        recommended_action: "restart_service",
        description: "payment 服务超时,重启 Service 端点恢复路由",
    },
    FaultScenario {
        name: "payment_unreachable",
        flag_name: "paymentServiceUnreachable",
        target_component: "payment",
        expected_metric: "span_error_rate_pct",
        finding_rule: "UNREACHABLE",
        finding_severity: "critical",
        recommended_action: "restart_pod",
        description: "payment 完全不可达。重启 Pod 让 Service 重选 endpoint",
    },
    FaultScenario {
        name: "kafka_queue_problems",
        flag_name: "kafkaQueueProblems",
        target_component: "kafka",
        expected_metric: "span_request_rate",
        finding_rule: "QUEUE_LAG",
        finding_severity: "warning",
        recommended_action: "scale_deployment",
        description: "kafka lag 飙升。扩容 Deployment",
    },
];

/// 8 个合法 PRD-001 action_id(对照 reference test 断言)。
pub const VALID_RECOMMENDED_ACTIONS: &[&str] = &[
    "restart_pod",
    "restart_service",
    "scale_deployment",
    "rollback_deployment",
    "clear_cache",
    "kill_query",
    "refresh_secret",
    "drain_node",
];

/// flag 名 -> scenario(对照 `scenario_for_flag`)。
pub fn scenario_for_flag(flag_name: &str) -> Option<&'static FaultScenario> {
    SCENARIOS.iter().find(|s| s.flag_name == flag_name)
}

/// scenario 名 -> scenario(对照 `scenario_for_name`)。
pub fn scenario_for_name(name: &str) -> Option<&'static FaultScenario> {
    SCENARIOS.iter().find(|s| s.name == name)
}

// ─────────────────────────────────────────────────────────────
//  state diff(对照 _extract_value / _state_differs)
// ─────────────────────────────────────────────────────────────

/// 从 state 拿真实值(对照 `_extract_value`):boolValue/doubleValue/stringValue/intValue/objectValue,回落 variant。
pub fn extract_value(state: &Value) -> Value {
    let Some(obj) = state.as_object() else {
        return state.clone();
    };
    for k in ["boolValue", "doubleValue", "stringValue", "intValue", "objectValue"] {
        if let Some(v) = obj.get(k) {
            return v.clone();
        }
    }
    obj.get("variant").cloned().unwrap_or(Value::Null)
}

/// variant 或具体值变了(对照 `_state_differs`)。
pub fn state_differs(old: &Value, new: &Value) -> bool {
    if old.get("variant") != new.get("variant") {
        return true;
    }
    extract_value(old) != extract_value(new)
}

// ─────────────────────────────────────────────────────────────
//  snapshot diff -> deltas(对照 sync_once 的 changed/added/removed)
// ─────────────────────────────────────────────────────────────

/// 一条 flag delta(`old` None=新增,`new` None=删除)。
#[derive(Debug, Clone)]
pub struct FlagDelta {
    pub flag_name: String,
    pub old: Option<Value>,
    pub new: Option<Value>,
    /// 基础 description(不含 scenario 后缀;enrich 时追加)。
    pub description: String,
}

/// diff 旧/新快照 -> deltas(对照 reference sync_once:changed + added + removed)。
pub fn diff_snapshots(old: &Snapshot, new: &Snapshot) -> Vec<FlagDelta> {
    let mut out = Vec::new();
    for (name, new_state) in new {
        match old.get(name) {
            None => out.push(FlagDelta {
                flag_name: name.clone(),
                old: None,
                new: Some(extract_value(new_state)),
                description: format!("flag added: {name}"),
            }),
            Some(old_state) if state_differs(old_state, new_state) => out.push(FlagDelta {
                flag_name: name.clone(),
                old: Some(extract_value(old_state)),
                new: Some(extract_value(new_state)),
                description: format!(
                    "flag {name}: variant={} → {}",
                    old_state
                        .get("variant")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                    new_state.get("variant").and_then(Value::as_str).unwrap_or("")
                ),
            }),
            _ => {}
        }
    }
    for (name, old_state) in old {
        if !new.contains_key(name) {
            out.push(FlagDelta {
                flag_name: name.clone(),
                old: Some(extract_value(old_state)),
                new: None,
                description: format!("flag removed: {name}"),
            });
        }
    }
    out
}

/// 聚合参数。
#[derive(Clone)]
pub struct Cfg {
    pub cluster: String,
    pub namespace: String,
    pub flagd_configmap_name: String,
    pub now: u64,
}

impl Cfg {
    pub fn new(cluster: &str, namespace: &str, flagd_configmap_name: &str, now: u64) -> Self {
        Self {
            cluster: cluster.to_string(),
            namespace: namespace.to_string(),
            flagd_configmap_name: flagd_configmap_name.to_string(),
            now,
        }
    }
    fn target(&self) -> String {
        format!("configmap:{}:{}:{}", self.cluster, self.namespace, self.flagd_configmap_name)
    }
}

/// 一条 delta -> change Fact(对照 reference `_try_record`,含 scenario enrichment)。
pub fn delta_to_change_fact(delta: &FlagDelta, cfg: &Cfg) -> Fact {
    let target = cfg.target();
    let mut diff_summary = serde_json::Map::new();
    diff_summary.insert(
        delta.flag_name.clone(),
        serde_json::json!({
            "old": delta.old.clone().unwrap_or(Value::Null),
            "new": delta.new.clone().unwrap_or(Value::Null),
        }),
    );
    let mut description = delta.description.clone();
    if let Some(s) = scenario_for_flag(&delta.flag_name) {
        diff_summary.insert(
            "scenario".to_string(),
            serde_json::json!({
                "name": s.name,
                "target_component": s.target_component,
                "recommended_action": s.recommended_action,
                "finding_severity": s.finding_severity,
                "expected_metric": s.expected_metric,
            }),
        );
        description.push_str(&format!(
            " [scenario={} 推荐动作={} 目标组件={}]",
            s.name, s.recommended_action, s.target_component
        ));
    }
    change_fact(cfg.now, &delta.flag_name, &target, description, Value::Object(diff_summary))
}

/// 构造一条 `kind="change"` Fact。attributes_json 载 ChangeRequest 子集。
fn change_fact(now: u64, flag_name: &str, target: &str, description: String, diff_summary: Value) -> Fact {
    Fact {
        id: format!("{SOURCE}:change:{flag_name}:{now}"),
        kind: KIND.to_string(),
        source: SOURCE.to_string(),
        resource_id: target.to_string(),
        resource_type: "ChangeEvent".to_string(),
        timestamp: now,
        attributes_json: serde_json::json!({
            "change_type": "configmap_updated",
            "target_resource_id": target,
            "source": CHANGE_SOURCE,
            "changed_by": "flagd",
            "description": description,
            "diff_summary": diff_summary,
            "cluster_id": "",
        })
        .to_string(),
    }
}

#[cfg(test)]
mod tests;
