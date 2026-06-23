//! system commands —— 应用元信息。
//!
//! Phase 1 仅 `get_app_version` 用于验证 invoke 链路。

/// 返回 engine-core 版本号(从 Cargo 元信息嵌入)。
#[tauri::command]
pub fn get_app_version() -> String {
    engine_core::version().to_string()
}
