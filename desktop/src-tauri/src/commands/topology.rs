//! topology commands — read persisted topology facts from storage.

use tauri::State;

use crate::commands::wasm::FactDto;
use crate::AppState;

/// Return the latest persisted topology facts.
#[tauri::command]
pub async fn get_topology(state: State<'_, AppState>) -> Result<Vec<FactDto>, String> {
    let facts = state
        .storage
        .latest_topology_facts()
        .await
        .map_err(|e| e.to_string())?;
    Ok(facts.iter().map(FactDto::from).collect())
}
