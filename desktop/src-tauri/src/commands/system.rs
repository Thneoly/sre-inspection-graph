//! system commands —— 应用元信息。

/// 返回应用版本号(tauri.conf.json 的 `version`,即安装包/关于对话框展示的版本)。
/// engine-core 版本与桌面应用版本独立演进,不再混用 —— 之前返回 engine-core
/// crate 版本,导致 UI 自报版本与发布叙事(v0.4.0)失配。
#[tauri::command]
pub fn get_app_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

/// 返回 engine-core crate 版本(诊断用,区分引擎与应用的演进节奏)。
#[tauri::command]
pub fn get_engine_version() -> String {
    engine_core::version().to_string()
}

/// recovery handler 模式(mock / real)。real = 恢复动作真改集群 —— 前端据此显示
/// REAL 徽标;此前该状态只存在于 env 解析,UI 全程不可见。
#[tauri::command]
pub fn get_handler_mode(state: tauri::State<'_, crate::AppState>) -> HandlerModeDto {
    HandlerModeDto {
        real_mode: state.handler_real_mode,
        api_base: state.handler_api_base.clone(),
    }
}

/// handler 模式 DTO(`get_handler_mode` 返回)。
#[derive(Debug, Clone, serde::Serialize)]
pub struct HandlerModeDto {
    /// true = SRE_GRAPH_HANDLER_MODE=real(真改集群)。
    pub real_mode: bool,
    /// handler 走的 K8s API base(kubectl proxy)。
    pub api_base: String,
}
