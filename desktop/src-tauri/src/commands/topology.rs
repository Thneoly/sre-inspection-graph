//! topology commands — read persisted topology facts from storage.

use engine_core::GraphResponse;
use tauri::State;

use crate::commands::wasm::FactDto;
use crate::AppState;

/// Return the latest persisted topology facts (raw `Fact` mirror).
///
/// 保留给需要原始 Fact 行的消费方(诊断 / 调试表)。前端拓扑渲染走
/// [`get_graph`] —— 拿已成图的 `GraphResponse`,不再 client 端解 JSON / 连边。
#[tauri::command]
pub async fn get_topology(state: State<'_, AppState>) -> Result<Vec<FactDto>, String> {
    let facts = state
        .storage
        .latest_topology_facts()
        .await
        .map_err(|e| e.to_string())?;
    Ok(facts.iter().map(FactDto::from).collect())
}

/// Return the latest persisted topology as a rendered [`GraphResponse`].
///
/// 三层契约 B 层:`storage.latest_topology_facts()` + `engine_core::facts_to_graph`
/// 的薄包装。去重 / parent_resource_id 连边 / 悬空过滤 / risk·health 统计全在
/// engine-core 完成,前端只把 `nodes`/`edges` 映射成 Cytoscape element。
#[tauri::command]
pub async fn get_graph(state: State<'_, AppState>) -> Result<GraphResponse, String> {
    let facts = state
        .storage
        .latest_topology_facts()
        .await
        .map_err(|e| e.to_string())?;
    Ok(engine_core::facts_to_graph(&facts))
}
