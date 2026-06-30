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
/// 三层契约 B 层。**Phase 2.5 起读 materialized topology**(`topology_nodes` /
/// `topology_edges`,由 `sync_all_now` 的 resolve→diff→apply 维护),经
/// `engine_identity::topology_to_graph` 成图 —— 不再每次从 raw facts 重算。
///
/// 注:首次升级到 2.5 的旧库 materialized 表为空,需先 `sync_all_now` 跑一次
/// resolve 才有数据(raw facts 仍在,sync 即回填)。
#[tauri::command]
pub async fn get_graph(state: State<'_, AppState>) -> Result<GraphResponse, String> {
    let topology = state
        .storage
        .materialized_topology()
        .await
        .map_err(|e| e.to_string())?;
    Ok(engine_identity::topology_to_graph(&topology))
}
