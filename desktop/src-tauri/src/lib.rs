//! sre-graph-desktop — Tauri 2.x backend lib(可被 mobile target 复用)。
//!
//! Phase 1 仅注册 1 个示例 command。Phase 2 起按 doc/17 §4.2 拆分
//! `commands/{topology, recovery, change_events, reports, connectors,
//! fault_simulation, system}.rs` 7 个领域文件。

pub mod commands;

use commands::system::get_app_version;

/// Tauri app builder 入口。`main.rs` 调用,确保 macOS / iOS 共享同一 builder。
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![get_app_version])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
