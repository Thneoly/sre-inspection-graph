//! Recovery Action Dry-Run Cascade - 影响范围计算(复刻 `reference/app/recovery/cascade.py`)。
//!
//! 输入:动作模板 + 目标资源 + 拓扑;输出:受影响资源列表 + 严重度 + 估算 SLA + 回滚参数。
//!
//! ## 与 reference 的差异
//!
//! - **I/O-free**:reference 读全局 DSS `store`;本模块把 [`engine_identity::Topology`]
//!   作入参传入(对齐 engine-identity 纯领域 + 薄持久化约定)。orchestration 层(Tauri/CLI)
//!   负责从 storage 取 materialized topology 喂进来。
//! - 算法逐字对齐:BFS `_walk`(forward/reverse + max_depth + target_type 筛选)、
//!   多规则命中取 max severity、关系/注释合并、排除自身、按 (-severity, resource_id) 排序。
//! - `rollback_input_params` 用 `serde_json::Value`(reference 用 dict)。

#![allow(missing_docs)]

use std::collections::{BTreeMap, HashMap, HashSet};

use engine_identity::{ResolvedNode, Topology};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::action_defs::{get_action, ActionDef, Direction, Impact, RiskLevel};

/// 一个受影响资源(merged 后)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AffectedResource {
    /// 资源 ID。
    pub resource_id: String,
    /// 资源类型。JSON key `type`(对齐 reference)。
    #[serde(rename = "type")]
    pub type_: String,
    /// 资源名(label)。
    pub name: String,
    /// 影响严重度。
    pub impact_severity: Impact,
    /// 命中此节点的关系类型(去重合并)。
    pub via_relations: Vec<String>,
    /// 影响说明(去重合并)。
    pub notes: Vec<String>,
}

/// dry-run 结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DryRunResult {
    /// 动作 ID。
    pub action_id: String,
    /// 动作名(动作未知时 None)。
    pub action_name: Option<String>,
    /// 目标资源 ID。
    pub target_resource_id: String,
    /// 目标资源类型(目标无效时 None)。
    pub target_resource_type: Option<String>,
    /// 目标资源名(目标无效时 None)。
    pub target_resource_name: Option<String>,
    /// 目标是否有效(动作存在 + 目标在拓扑 + 类型匹配)。
    pub target_valid: bool,
    /// 目标无效时的原因(target_valid=true 时 None)。
    pub validation_error: Option<String>,
    /// 受影响资源列表(按严重度倒序 + resource_id 升序)。
    pub affected_resources: Vec<AffectedResource>,
    /// 受影响资源数。
    pub affected_count: usize,
    /// 预估耗时(秒)。
    pub estimated_duration_seconds: u32,
    /// 预估 SLA 影响。
    pub estimated_sla_impact: String,
    /// 警告。
    pub warnings: Vec<String>,
    /// 回滚动作 ID(不可逆时 None)。
    pub rollback_action_id: Option<String>,
    /// 回滚参数(scale_deployment 反向 delta / rollback_deployment 空 {})。
    pub rollback_input_params: Option<Value>,
    /// 风险等级(动作未知时 None)。
    pub risk_level: Option<RiskLevel>,
    /// 是否需审批(动作未知时 None)。
    pub requires_approval: Option<bool>,
}

/// 对一个 (动作 + 目标) 计算影响范围。
///
/// 动作不存在 / 目标不在拓扑 / 类型不匹配 -> `target_valid=false` + `validation_error`
/// (不抛异常,前端可友好提示,对齐 reference)。
pub fn dry_run(
    action_id: &str,
    target_resource_id: &str,
    input_params: &Value,
    topology: &Topology,
) -> DryRunResult {
    let Some(action) = get_action(action_id) else {
        return invalid_result(action_id, target_resource_id, None, &format!("unknown action_id: {action_id}"));
    };

    // 节点索引:resource_id -> &ResolvedNode
    let node_index: HashMap<&str, &ResolvedNode> = topology
        .nodes
        .iter()
        .map(|n| (n.resource_id.as_str(), n))
        .collect();

    let Some(target_node) = node_index.get(target_resource_id) else {
        return invalid_result(
            action_id,
            target_resource_id,
            Some(action),
            &format!("target resource not found in topology: {target_resource_id}"),
        );
    };

    if target_node.resource_type != action.target_type {
        return invalid_result(
            action_id,
            target_resource_id,
            Some(action),
            &format!(
                "action targets {} but resource is {}",
                action.target_type, target_node.resource_type
            ),
        );
    }

    // 沿每条 propagation 规则 BFS,合并结果
    let mut affected: BTreeMap<String, AffectedResource> = BTreeMap::new();
    for rule in action.propagation {
        for hit in walk(target_resource_id, rule, topology, &node_index) {
            match affected.get_mut(&hit.resource_id) {
                Some(existing) => {
                    // 严重度取较大值
                    if hit.impact.rank() > existing.impact_severity.rank() {
                        existing.impact_severity = hit.impact;
                    }
                    // 关系 / 注释合并去重
                    if !existing.via_relations.contains(&hit.via_relation) {
                        existing.via_relations.push(hit.via_relation);
                    }
                    if !existing.notes.contains(&hit.note) {
                        existing.notes.push(hit.note);
                    }
                }
                None => {
                    affected.insert(
                        hit.resource_id.clone(),
                        AffectedResource {
                            resource_id: hit.resource_id,
                            type_: hit.type_,
                            name: hit.name,
                            impact_severity: hit.impact,
                            via_relations: vec![hit.via_relation],
                            notes: vec![hit.note],
                        },
                    );
                }
            }
        }
    }

    // 排除自身
    affected.remove(target_resource_id);

    // 按 (-severity, resource_id) 排序,稳定输出
    let mut affected_list: Vec<AffectedResource> = affected.into_values().collect();
    affected_list.sort_by(|a, b| {
        b.impact_severity
            .rank()
            .cmp(&a.impact_severity.rank())
            .then_with(|| a.resource_id.cmp(&b.resource_id))
    });

    let rollback_input_params = compute_rollback_params(action, input_params);

    DryRunResult {
        action_id: action_id.to_string(),
        action_name: Some(action.name.to_string()),
        target_resource_id: target_resource_id.to_string(),
        target_resource_type: Some(target_node.resource_type.clone()),
        target_resource_name: Some(target_node.label.clone()),
        target_valid: true,
        validation_error: None,
        affected_count: affected_list.len(),
        affected_resources: affected_list,
        estimated_duration_seconds: action.estimated_duration_seconds,
        estimated_sla_impact: action.sla_impact_estimate.to_string(),
        warnings: action.warnings.iter().map(|w| w.to_string()).collect(),
        rollback_action_id: action.rollback_action_id.map(|s| s.to_string()),
        rollback_input_params,
        risk_level: Some(action.risk_level),
        requires_approval: Some(action.requires_approval),
    }
}

/// BFS 一条 propagation 规则,返回所有命中节点(未合并)。
fn walk<'t>(
    start_id: &str,
    rule: &crate::action_defs::PropagationRule,
    topology: &'t Topology,
    node_index: &HashMap<&str, &'t ResolvedNode>,
) -> Vec<Hit> {
    let mut visited: HashSet<&str> = HashSet::new();
    visited.insert(start_id);
    let mut frontier: Vec<&str> = vec![start_id];
    let mut hits: Vec<Hit> = Vec::new();

    for _ in 0..rule.max_depth {
        let mut next_frontier: Vec<&str> = Vec::new();
        for node_id in &frontier {
            for edge in &topology.edges {
                if edge.edge_type != rule.edge {
                    continue;
                }
                // forward = source->target;reverse = target->source
                let next_id = match rule.direction {
                    Direction::Forward if edge.source == *node_id => edge.target.as_str(),
                    Direction::Reverse if edge.target == *node_id => edge.source.as_str(),
                    _ => continue,
                };
                if visited.contains(next_id) {
                    continue;
                }
                visited.insert(next_id);
                next_frontier.push(next_id);

                let Some(node) = node_index.get(next_id) else {
                    continue;
                };
                if let Some(tt) = rule.target_type {
                    if node.resource_type != tt {
                        continue;
                    }
                }
                hits.push(Hit {
                    resource_id: next_id.to_string(),
                    type_: node.resource_type.clone(),
                    name: node.label.clone(),
                    impact: rule.impact,
                    via_relation: rule.edge.to_string(),
                    note: rule.note.to_string(),
                });
            }
        }
        if next_frontier.is_empty() {
            break;
        }
        frontier = next_frontier;
    }
    hits
}

/// walk 的单条命中(合并前)。
struct Hit {
    resource_id: String,
    type_: String,
    name: String,
    impact: Impact,
    via_relation: String,
    note: String,
}

/// 计算回滚参数(对齐 reference `_compute_rollback_params`)。
///
/// - scale_deployment(replicas_delta=N) -> `{"replicas_delta": -N}`
/// - rollback_deployment -> `{}`(rollout undo 默认行为)
/// - 其它 -> None(不可逆或 self-rollback 无简单参数)
fn compute_rollback_params(action: &ActionDef, input_params: &Value) -> Option<Value> {
    let rollback_id = action.rollback_action_id?;
    if action.category == "scale" {
        if let Some(delta) = input_params.get("replicas_delta").and_then(Value::as_i64) {
            return Some(serde_json::json!({ "replicas_delta": -delta }));
        }
    }
    if rollback_id == "rollback_deployment" {
        return Some(serde_json::json!({}));
    }
    None
}

/// 构造 `target_valid=false` 的结果。
fn invalid_result(
    action_id: &str,
    target_resource_id: &str,
    action: Option<&ActionDef>,
    error: &str,
) -> DryRunResult {
    DryRunResult {
        action_id: action_id.to_string(),
        action_name: action.map(|a| a.name.to_string()),
        target_resource_id: target_resource_id.to_string(),
        target_resource_type: None,
        target_resource_name: None,
        target_valid: false,
        validation_error: Some(error.to_string()),
        affected_resources: vec![],
        affected_count: 0,
        estimated_duration_seconds: action.map(|a| a.estimated_duration_seconds).unwrap_or(0),
        estimated_sla_impact: action
            .map(|a| a.sla_impact_estimate.to_string())
            .unwrap_or_else(|| "n/a".to_string()),
        warnings: action
            .map(|a| a.warnings.iter().map(|w| w.to_string()).collect())
            .unwrap_or_default(),
        rollback_action_id: action.and_then(|a| a.rollback_action_id.map(|s| s.to_string())),
        rollback_input_params: None,
        risk_level: action.map(|a| a.risk_level),
        requires_approval: action.map(|a| a.requires_approval),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_identity::{ResolvedEdge, ResolvedNode, Topology};
    use serde_json::json;

    /// 构造 reference test_recovery.py 的 fixture 拓扑(9 节点 11 边)。
    ///
    /// ```text
    /// app:order
    ///   └ comp:order-api      (BELONGS_TO 反向 = comp 在 app 下)
    ///       └ deploy:order-api  (DEPLOYED_AS)
    ///           └ pod:order-api-1  (CONTAINS)
    ///           └ pod:order-api-2  (CONTAINS)
    ///               └ node:worker-1  (SCHEDULED_ON)
    /// svc:order-api  (ROUTES_TO pod:order-api-1, pod:order-api-2)
    /// secret:order-tls  (USES -> pod:order-api-1)
    /// mysql:order-db  (USES -> pod:order-api-1)
    /// ```
    fn fixture_topology() -> Topology {
        let nodes = vec![
            node("app:order", "Application", "订单应用"),
            node("comp:order-api", "ApplicationComponent", "订单API组件"),
            node("deploy:order-api", "Deployment", "order-api"),
            node("pod:order-api-1", "Pod", "order-api-1"),
            node("pod:order-api-2", "Pod", "order-api-2"),
            node("node:worker-1", "KubernetesNode", "worker-1"),
            node("svc:order-api", "Service", "order-api-svc"),
            node("secret:order-tls", "Secret", "order-tls"),
            node("mysql:order-db", "MySQL", "order-db"),
        ];
        let edges = vec![
            edge("e1", "app:order", "CONTAINS", "comp:order-api"),
            edge("e2", "comp:order-api", "DEPLOYED_AS", "deploy:order-api"),
            edge("e3", "deploy:order-api", "CONTAINS", "pod:order-api-1"),
            edge("e4", "deploy:order-api", "CONTAINS", "pod:order-api-2"),
            edge("e5", "pod:order-api-2", "SCHEDULED_ON", "node:worker-1"),
            edge("e6", "svc:order-api", "ROUTES_TO", "pod:order-api-1"),
            edge("e7", "svc:order-api", "ROUTES_TO", "pod:order-api-2"),
            edge("e8", "pod:order-api-1", "USES", "secret:order-tls"),
            edge("e9", "pod:order-api-1", "USES", "mysql:order-db"),
            edge("e10", "deploy:order-api", "BELONGS_TO", "comp:order-api"),
            edge("e11", "comp:order-api", "BELONGS_TO", "app:order"),
        ];
        Topology { nodes, edges }
    }

    fn node(rid: &str, rtype: &str, label: &str) -> ResolvedNode {
        ResolvedNode {
            resource_id: rid.into(),
            resource_type: rtype.into(),
            label: label.into(),
            attributes_json: "{}".into(),
        }
    }

    fn edge(id: &str, src: &str, etype: &str, tgt: &str) -> ResolvedEdge {
        ResolvedEdge {
            id: id.into(),
            source: src.into(),
            target: tgt.into(),
            edge_type: etype.into(),
        }
    }

    fn affected_ids(r: &DryRunResult) -> HashSet<String> {
        r.affected_resources
            .iter()
            .map(|a| a.resource_id.clone())
            .collect()
    }

    #[test]
    fn unknown_action_invalid() {
        let r = dry_run("nonexistent_action", "pod:x", &json!({}), &fixture_topology());
        assert!(!r.target_valid);
        assert!(r.validation_error.as_deref().unwrap().contains("unknown action_id"));
    }

    #[test]
    fn target_not_in_topology_invalid() {
        let r = dry_run("restart_pod", "pod:does-not-exist", &json!({}), &fixture_topology());
        assert!(!r.target_valid);
        assert!(r.validation_error.as_deref().unwrap().contains("not found"));
    }

    #[test]
    fn target_type_mismatch_invalid() {
        // restart_pod targets Pod, but deploy:order-api is Deployment
        let r = dry_run("restart_pod", "deploy:order-api", &json!({}), &fixture_topology());
        assert!(!r.target_valid);
        let err = r.validation_error.as_deref().unwrap();
        assert!(err.contains("Pod"), "err should mention action target_type Pod: {err}");
        assert!(err.contains("Deployment"), "err should mention resource type Deployment: {err}");
    }

    #[test]
    fn scale_deployment_propagation() {
        let r = dry_run(
            "scale_deployment",
            "deploy:order-api",
            &json!({ "replicas_delta": 2 }),
            &fixture_topology(),
        );
        assert!(r.target_valid);
        let ids = affected_ids(&r);
        // forward CONTAINS -> 2 个 Pod
        assert!(ids.contains("pod:order-api-1"));
        assert!(ids.contains("pod:order-api-2"));
        // BELONGS_TO 链向上 -> component / application
        assert!(ids.contains("comp:order-api") || ids.contains("app:order"));
        // 自身排除
        assert!(!ids.contains("deploy:order-api"));
        // 回滚参数:反向 delta
        assert_eq!(r.rollback_input_params, Some(json!({ "replicas_delta": -2 })));
    }

    #[test]
    fn restart_pod_propagation() {
        // reverse ROUTES_TO -> Service
        let r = dry_run("restart_pod", "pod:order-api-1", &json!({}), &fixture_topology());
        assert!(r.target_valid);
        let ids = affected_ids(&r);
        assert!(ids.contains("svc:order-api"), "reverse ROUTES_TO should hit Service");
    }

    #[test]
    fn drain_node_propagation() {
        // reverse SCHEDULED_ON -> Pod 调度在此节点
        let r = dry_run("drain_node", "node:worker-1", &json!({}), &fixture_topology());
        assert!(r.target_valid);
        let ids = affected_ids(&r);
        assert!(ids.contains("pod:order-api-2"), "pod:order-api-2 scheduled on worker-1");
    }

    #[test]
    fn refresh_secret_propagation() {
        // reverse USES -> 引用此 Secret 的 Pod
        let r = dry_run("refresh_secret", "secret:order-tls", &json!({}), &fixture_topology());
        assert!(r.target_valid);
        let ids = affected_ids(&r);
        assert!(ids.contains("pod:order-api-1"), "reverse USES should hit pod using secret");
    }

    #[test]
    fn severity_aggregation_takes_max() {
        // rollback_deployment 的 Pod 是 medium(forward CONTAINS)
        let r = dry_run("rollback_deployment", "deploy:order-api", &json!({}), &fixture_topology());
        assert!(r.target_valid);
        let pods: Vec<&AffectedResource> = r
            .affected_resources
            .iter()
            .filter(|a| a.type_ == "Pod")
            .collect();
        assert!(!pods.is_empty());
        for p in &pods {
            assert!(
                p.impact_severity == Impact::Medium || p.impact_severity == Impact::High,
                "Pod impact should be >= medium, got {:?}",
                p.impact_severity
            );
        }
    }

    #[test]
    fn rollback_deployment_rollback_params_empty_object() {
        let r = dry_run("rollback_deployment", "deploy:order-api", &json!({}), &fixture_topology());
        assert!(r.target_valid);
        assert_eq!(r.rollback_input_params, Some(json!({})));
    }

    #[test]
    fn non_reversible_action_has_no_rollback_params() {
        // restart_pod rollback_action_id=None -> None
        let r = dry_run("restart_pod", "pod:order-api-1", &json!({}), &fixture_topology());
        assert!(r.target_valid);
        assert_eq!(r.rollback_input_params, None);
    }

    #[test]
    fn affected_resources_sorted_by_severity_then_id() {
        // refresh_secret 命中 Pod(medium,reverse USES)+ Deployment(low,reverse USES)
        // + BELONGS_TO 链(low)。验排序:severity 倒序,同 severity 按 resource_id 升序。
        let r = dry_run("refresh_secret", "secret:order-tls", &json!({}), &fixture_topology());
        assert!(r.target_valid);
        let sev = r
            .affected_resources
            .iter()
            .map(|a| a.impact_severity.rank())
            .collect::<Vec<_>>();
        let mut sorted = sev.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(sev, sorted, "affected should be sorted by severity desc: {:?}", sev);
    }
}
