//! inspection view commands — Phase 5(复刻 reference view2-5 图遍历视图)。
//!
//! 4 个视图共用 `engine_identity::views::subgraph` BFS 子图原语,各配不同 edge 白名单、
//! 方向、默认 depth。读 materialized topology,跑 subgraph,再 `topology_to_graph` 出
//! `GraphResponse`(前端复用 `TopologyView` 渲染)。
//!
//! - node-impact:KubernetesNode 起点,Reverse,depth 4
//! - config-impact:Secret/ConfigMap 起点,Reverse,depth 4
//! - access-link:Application 起点,Both,depth 5
//! - image-risk:ContainerImage 起点,Reverse,depth 4(真集群当前无 image 节点 -> 空图)
//!
//! `alert-aggregation` 结构不同(AlertRegistry join + FIRED_ON 边),延后。

use engine_core::GraphResponse;
use engine_identity::{subgraph, topology_to_graph};
use serde::Serialize;
use tauri::State;

use crate::AppState;

/// 选择器数据源:前端下拉用,避免整图下发。
#[derive(Debug, Clone, Serialize)]
pub struct ResourceOption {
    /// 节点 canonical 身份键。
    pub resource_id: String,
    /// 展示标签。
    pub label: String,
    /// 资源类型。
    pub resource_type: String,
}

/// 读 materialized topology,跑 node-impact(爆炸半径)子图。
#[tauri::command]
pub async fn node_impact(
    state: State<'_, AppState>,
    node_id: String,
    depth: Option<usize>,
) -> Result<GraphResponse, String> {
    let topo = state
        .storage
        .materialized_topology()
        .await
        .map_err(|e| e.to_string())?;
    let sub = subgraph(
        &topo,
        &node_id,
        depth.unwrap_or(4),
        engine_identity::NODE_IMPACT_EDGES,
        engine_identity::TraversalDir::Reverse,
    );
    Ok(topology_to_graph(&sub))
}

/// 读 materialized topology,跑 config-impact(配置传播)子图。
#[tauri::command]
pub async fn config_impact(
    state: State<'_, AppState>,
    resource_id: String,
    depth: Option<usize>,
) -> Result<GraphResponse, String> {
    let topo = state
        .storage
        .materialized_topology()
        .await
        .map_err(|e| e.to_string())?;
    let sub = subgraph(
        &topo,
        &resource_id,
        depth.unwrap_or(4),
        engine_identity::CONFIG_IMPACT_EDGES,
        engine_identity::TraversalDir::Reverse,
    );
    Ok(topology_to_graph(&sub))
}

/// 读 materialized topology,跑 access-link(访问链)子图。无向,起点默认 Application。
#[tauri::command]
pub async fn access_link(
    state: State<'_, AppState>,
    resource_id: String,
    depth: Option<usize>,
) -> Result<GraphResponse, String> {
    let topo = state
        .storage
        .materialized_topology()
        .await
        .map_err(|e| e.to_string())?;
    let sub = subgraph(
        &topo,
        &resource_id,
        depth.unwrap_or(5),
        engine_identity::ACCESS_LINK_EDGES,
        engine_identity::TraversalDir::Both,
    );
    Ok(topology_to_graph(&sub))
}

/// 读 materialized topology,跑 image-risk(镜像风险)子图。
///
/// 真集群当前 k8s connector 不产 ContainerImage 节点 -> 真集群上返空图(预期)。
#[tauri::command]
pub async fn image_risk(
    state: State<'_, AppState>,
    resource_id: String,
    depth: Option<usize>,
) -> Result<GraphResponse, String> {
    let topo = state
        .storage
        .materialized_topology()
        .await
        .map_err(|e| e.to_string())?;
    let sub = subgraph(
        &topo,
        &resource_id,
        depth.unwrap_or(4),
        engine_identity::IMAGE_RISK_EDGES,
        engine_identity::TraversalDir::Reverse,
    );
    Ok(topology_to_graph(&sub))
}

/// 列出 `resource_types` 命中任一类型的节点(前端视图起点选择器数据源)。
#[tauri::command]
pub async fn list_resources_by_types(
    state: State<'_, AppState>,
    resource_types: Vec<String>,
) -> Result<Vec<ResourceOption>, String> {
    let topo = state
        .storage
        .materialized_topology()
        .await
        .map_err(|e| e.to_string())?;
    let opts = topo
        .nodes
        .iter()
        .filter(|n| resource_types.iter().any(|t| t == &n.resource_type))
        .map(|n| ResourceOption {
            resource_id: n.resource_id.clone(),
            label: n.label.clone(),
            resource_type: n.resource_type.clone(),
        })
        .collect();
    Ok(opts)
}

/// alert-aggregation 视图(复刻 reference view6):聚合所有 firing 告警 + 其 resource 邻域。
///
/// 读 AlertRegistry(firing + 可选 severity 过滤)+ materialized topology,经
/// `engine_changes::alert_aggregation_graph` 合成 alert 节点 + FIRED_ON 边 + resource 邻域。
/// 与其余 4 视图不同:无起点 Select,展示全部 firing 告警的聚合图。
#[tauri::command]
pub async fn alert_aggregation(
    state: State<'_, AppState>,
    severity: Option<engine_changes::AlertSeverity>,
    depth: Option<usize>,
) -> Result<GraphResponse, String> {
    let topology = state
        .storage
        .materialized_topology()
        .await
        .map_err(|e| e.to_string())?;
    // clone registry 出锁,避免持锁期间做图构建(纯函数,无 await)
    let alerts = state.alerts.lock().await.clone();
    Ok(engine_changes::alert_aggregation_graph(
        &alerts,
        &topology,
        severity,
        depth.unwrap_or(engine_changes::DEFAULT_ALERT_AGGREGATION_DEPTH),
    ))
}
