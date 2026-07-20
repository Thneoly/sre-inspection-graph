//! K8s 变更检测(PRD-003 后续 / PRD-002 自动录 change event,Phase 4.3 后续)。
//!
//! `detect_changes(current, next) -> Vec<ChangeRequest>` 纯函数:对比两次 sync 的拓扑,
//! 对 ConfigMap/Secret/Deployment(对齐 reference k8s_watch_connector 只 watch 3 类)
//! 按信号字段判定变更,产 ChangeRequest 供 orchestration 调 `record_change`。
//!
//! **poll-diff 架构**(非 streaming watch):桌面 30s poll 周期,在 sync_all_now 的
//! diff 步骤后翻译 ChangeSet -> ChangeRequest。无需 WIT stream / WASIp3(真 watch 留长期)。
//!
//! **信号字段触发**(非整 attrs diff):只看 rollout/config 语义字段,避免 health_status/
//! risk_level 变化误报 deployment_rolled。用 `compute_yaml_diff(keys=Some(signal))` 聚焦。
//!
//! **偏差**:ConfigMap/Secret value-only 变更漏检(不存 data 值,data_keys 不变 -> diff 空);
//! image_pushed 跳过(reference 来自 Harbor webhook,非 watch);首次 sync 抑制在 orchestration 层。

#![allow(missing_docs)]

use engine_identity::Topology;
use serde_json::Value;

use crate::models::ChangeRequest;
use crate::yaml_diff::{compute_yaml_diff, summarize_diff};

/// ConfigMap/Secret/Deployment 的变更检测信号字段(只这些字段变化才触发 ChangeEvent)。
fn signal_keys(resource_type: &str) -> Option<&'static [&'static str]> {
    match resource_type {
        "ConfigMap" | "Secret" => Some(&["data_keys"]),
        "Deployment" => Some(&["current_revision", "images", "replicas_desired", "replicas_ready"]),
        _ => None,
    }
}

/// 资源类型 -> change_type(对齐 reference `_KIND_MAP`:CM->configmap_updated /
/// Secret->secret_rotated / Deployment->deployment_rolled)。
fn change_type_for(resource_type: &str) -> Option<&'static str> {
    match resource_type {
        "ConfigMap" => Some("configmap_updated"),
        "Secret" => Some("secret_rotated"),
        "Deployment" => Some("deployment_rolled"),
        _ => None,
    }
}

/// 解析 attributes_json;非法 -> Null(后续 .get 返 None,按缺失处理)。
fn parse_attrs(json_str: &str) -> Value {
    serde_json::from_str(json_str).unwrap_or(Value::Null)
}

/// 对比两次 sync 的拓扑,产出 ChangeRequest 列表(orchestration 调 record_change 录入)。
///
/// 仅 ConfigMap/Secret/Deployment 参与;新资源(current 无)跳过(对齐 reference ADDED
/// 不发,首次 sync 防炸历史);信号字段未变(compute_yaml_diff 空串)跳过(对齐 reference
/// 噪声过滤)。
pub fn detect_changes(current: &Topology, next: &Topology) -> Vec<ChangeRequest> {
    let mut reqs = Vec::new();
    for new_node in &next.nodes {
        let Some(change_type) = change_type_for(&new_node.resource_type) else {
            continue; // 非 CM/Secret/Deploy:拓扑变更归 ChangeSet,不发 ChangeEvent
        };
        // 仅对 current 里也存在的资源(MODIFIED 语义);新资源跳过
        let Some(old_node) = current.nodes.iter().find(|n| n.resource_id == new_node.resource_id)
        else {
            continue;
        };
        let keys = signal_keys(&new_node.resource_type).unwrap();
        let old_attrs = parse_attrs(&old_node.attributes_json);
        let new_attrs = parse_attrs(&new_node.attributes_json);
        let diff = compute_yaml_diff(&old_attrs, &new_attrs, Some(keys), &new_node.resource_id);
        if diff.is_empty() {
            continue; // 信号字段未变(噪声过滤后无差异)
        }
        let summary = summarize_diff(&diff);
        let cluster_id = new_attrs
            .get("cluster")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        reqs.push(ChangeRequest {
            change_type: change_type.to_string(),
            target_resource_id: new_node.resource_id.clone(),
            source: "k8s_api".to_string(),
            description: format!("k8s sync diff: {}", new_node.resource_id),
            diff_summary: serde_json::to_value(&summary).unwrap_or(Value::Null),
            yaml_diff: diff,
            cluster_id,
            ..Default::default()
        });
    }
    reqs
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_identity::{ResolvedNode, Topology};

    fn node(rid: &str, rtype: &str, attrs: &str) -> ResolvedNode {
        ResolvedNode {
            resource_id: rid.into(),
            resource_type: rtype.into(),
            label: rid.into(),
            attributes_json: attrs.into(),
        }
    }

    fn topo(nodes: Vec<ResolvedNode>) -> Topology {
        Topology { nodes, edges: vec![] }
    }

    // 移植 reference test_change_events_phase2.py::TestK8sWatchConnector 语义

    #[test]
    fn deployment_revision_change_produces_deployment_rolled() {
        let current = topo(vec![node(
            "deploy:c:ns:frontend",
            "Deployment",
            r#"{"cluster":"c","current_revision":"1","images":"img:v1","replicas_desired":2,"replicas_ready":2,"health_status":"normal"}"#,
        )]);
        let next = topo(vec![node(
            "deploy:c:ns:frontend",
            "Deployment",
            r#"{"cluster":"c","current_revision":"2","images":"img:v2","replicas_desired":2,"replicas_ready":1,"health_status":"normal"}"#,
        )]);
        let reqs = detect_changes(&current, &next);
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].change_type, "deployment_rolled");
        assert_eq!(reqs[0].target_resource_id, "deploy:c:ns:frontend");
        assert_eq!(reqs[0].source, "k8s_api");
        assert_eq!(reqs[0].cluster_id, "c");
        assert!(!reqs[0].yaml_diff.is_empty()); // revision/images 变更 -> diff 非空
    }

    #[test]
    fn configmap_data_keys_change_produces_configmap_updated() {
        let current = topo(vec![node(
            "cm:c:ns:cfg",
            "ConfigMap",
            r#"{"cluster":"c","data_keys":"a,b"}"#,
        )]);
        let next = topo(vec![node(
            "cm:c:ns:cfg",
            "ConfigMap",
            r#"{"cluster":"c","data_keys":"a,b,c"}"#,
        )]);
        let reqs = detect_changes(&current, &next);
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].change_type, "configmap_updated");
    }

    #[test]
    fn secret_data_keys_change_produces_secret_rotated() {
        let current = topo(vec![node(
            "secret:c:ns:db",
            "Secret",
            r#"{"cluster":"c","data_keys":"password"}"#,
        )]);
        let next = topo(vec![node(
            "secret:c:ns:db",
            "Secret",
            r#"{"cluster":"c","data_keys":"password,token"}"#,
        )]);
        let reqs = detect_changes(&current, &next);
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].change_type, "secret_rotated");
    }

    #[test]
    fn new_resource_skipped() {
        // next 有但 current 无 -> 新资源,不发(对齐 reference first_sync ADDED 不发)
        let current = topo(vec![]);
        let next = topo(vec![node(
            "deploy:c:ns:new",
            "Deployment",
            r#"{"cluster":"c","current_revision":"1","images":"img","replicas_desired":1,"replicas_ready":1}"#,
        )]);
        let reqs = detect_changes(&current, &next);
        assert!(reqs.is_empty());
    }

    #[test]
    fn signal_unchanged_skips_even_if_health_changed() {
        // health_status 变了但信号字段(revision/images/replicas)没变 -> 不发(防误报 roll)
        let current = topo(vec![node(
            "deploy:c:ns:frontend",
            "Deployment",
            r#"{"cluster":"c","current_revision":"1","images":"img:v1","replicas_desired":2,"replicas_ready":2,"health_status":"normal"}"#,
        )]);
        let next = topo(vec![node(
            "deploy:c:ns:frontend",
            "Deployment",
            r#"{"cluster":"c","current_revision":"1","images":"img:v1","replicas_desired":2,"replicas_ready":2,"health_status":"critical"}"#,
        )]);
        let reqs = detect_changes(&current, &next);
        assert!(reqs.is_empty()); // 信号字段未变 -> diff 空 -> 跳过
    }

    #[test]
    fn non_watched_resource_type_skipped() {
        // Pod/Service/Node 等不产 ChangeEvent(拓扑变更归 ChangeSet)
        let current = topo(vec![node("pod:c:ns:p", "Pod", r#"{"cluster":"c","phase":"Running"}"#)]);
        let next = topo(vec![node("pod:c:ns:p", "Pod", r#"{"cluster":"c","phase":"Pending"}"#)]);
        let reqs = detect_changes(&current, &next);
        assert!(reqs.is_empty());
    }

    #[test]
    fn multiple_changes_in_one_sync() {
        let current = topo(vec![
            node("deploy:c:ns:a", "Deployment", r#"{"cluster":"c","current_revision":"1","images":"img","replicas_desired":1,"replicas_ready":1}"#),
            node("cm:c:ns:b", "ConfigMap", r#"{"cluster":"c","data_keys":"x"}"#),
        ]);
        let next = topo(vec![
            node("deploy:c:ns:a", "Deployment", r#"{"cluster":"c","current_revision":"2","images":"img","replicas_desired":1,"replicas_ready":1}"#),
            node("cm:c:ns:b", "ConfigMap", r#"{"cluster":"c","data_keys":"x,y"}"#),
        ]);
        let reqs = detect_changes(&current, &next);
        assert_eq!(reqs.len(), 2);
        let types: Vec<&str> = reqs.iter().map(|r| r.change_type.as_str()).collect();
        assert!(types.contains(&"deployment_rolled"));
        assert!(types.contains(&"configmap_updated"));
    }
}
