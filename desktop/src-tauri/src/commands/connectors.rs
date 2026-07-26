//! connectors commands —— PRD-004 connector 运行时观测(Phase 6 connectors-ui)。
//!
//! dogfood 浮现的痛点:connector 健康/sync 状态以前**只能看日志** ——
//! [`crate::commands::wasm::list_connectors`] 只列**已加载**的 connector(禁用 / 加载
//! 失败的不可见),后台 [`crate::sync_loop`] 每 30s 跑一次 `sync_all` 但其
//! per-connector 产出**被丢弃**,只有手动点「立即同步」(`sync_all_now`)才返
//! per-connector 明细。打开页面看 status 这件事根本做不到。
//!
//! 这里加一个**持久 connector 状态注册表**:`AppState.connector_statuses`。
//!
//! - **启动播种**:[`seed_connector_statuses`] 从**完整 manifest** 建(禁用 / 加载失败
//!   的模块也进表,带 `load_error`),而不是只看 runtime.entries。
//! - **每次 sync 刷新**:[`update_connector_statuses`] 在 [`run_sync`](`crate::commands::wasm::run_sync`)
//!   内调(手动 `sync_all_now` 与后台 `sync_loop` 共用同一管线 → 一处更新覆盖两路),
//!   写 `last_synced_at` / `last_fact_count` / `last_errors` / `last_duration_ms`。
//! - **前端拉**:[`get_connectors_status`] 命令 clone 出整表。
//!
//! 设计要点:
//! 1. **静态字段来自 manifest**(name/kind/enabled/capabilities/sync_interval/config/fs_roots),
//!    **运行时字段来自 sync** —— 一张表同时回答「配了啥」+「跑得咋样」。
//! 2. **`loaded` ≠ `enabled`**:`enabled` 是 manifest 开关,`loaded` 是真进了 runtime
//!    (enabled=true 但 wasm 缺失/实例化失败 → enabled=true,loaded=false,load_error=Some)。
//! 3. **handler 也入表**(kind="handler")—— 它们是 runtime 模块,recovery real-mode
//!    依赖 k8s-handler 加载,看见它是否 loaded 有诊断价值。handler 无 sync,运行时字段恒 None。
//! 4. **锁纪律**:`tokio::sync::Mutex`,update/get 持锁极短不跨 await。

use serde::Serialize;
use tauri::State;

use crate::AppState;

/// 单个模块(connector / handler)的可观测状态:静态 manifest 字段 + 最近一次 sync 的运行时字段。
#[derive(Debug, Clone, Serialize)]
pub struct ConnectorStatus {
    /// 模块名(manifest.name)。
    pub name: String,
    /// 模块类型:connector / handler / rule(本期 rule 未用)。
    pub kind: String,
    /// SemVer 版本。
    pub version: String,
    /// manifest `enabled` 开关(false → from_manifest 跳过加载)。
    pub enabled: bool,
    /// 是否真进了 runtime(`enabled=true` 且加载成功)。enabled≠loaded:失败模块 enabled 仍 true。
    pub loaded: bool,
    /// 申明的 capability。
    pub capabilities: Vec<String>,
    /// 周期同步间隔(秒)。
    pub sync_interval_seconds: u64,
    /// per-connector manifest config(inline table -> JSON object)。缺省 None。
    pub config: Option<serde_json::Value>,
    /// fs-read 允许的根(fs_roots;非 fs-read connector 为空)。
    pub fs_roots: Vec<String>,
    /// 加载失败原因(loaded=false 且 enabled=true 时有值)。
    pub load_error: Option<String>,
    /// 最近一次 sync 完成时间(ISO8601 UTC)。未 sync 过 = None(handler / 禁用 / 失败模块)。
    pub last_synced_at: Option<String>,
    /// 最近一次 sync 产出的 fact 数。
    pub last_fact_count: Option<usize>,
    /// 最近一次 sync 的 non-fatal 错误。
    pub last_errors: Vec<String>,
    /// 最近一次 sync guest 自报耗时(毫秒)。
    pub last_duration_ms: Option<u64>,
}

/// 启动播种:从完整 manifest + runtime 加载结果建初始状态表。
///
/// 遍历 **manifest.modules**(而非 runtime.entries),这样禁用 / 加载失败的模块也入表。
/// 运行时字段全 None/空 —— 等 `update_connector_statuses` 在首次 sync 后填。
pub fn seed_connector_statuses(
    manifest: &engine_wasm::ManifestFile,
    runtime: &engine_wasm::WasmRuntime,
) -> Vec<ConnectorStatus> {
    use std::collections::{HashMap, HashSet};
    let loaded_connectors: HashSet<&str> =
        runtime.entries.iter().map(|e| e.name.as_str()).collect();
    let loaded_handlers: HashSet<&str> =
        runtime.handlers.iter().map(|e| e.name.as_str()).collect();
    let load_errors: HashMap<&str, &str> = runtime
        .load_errors
        .iter()
        .map(|(n, e)| (n.as_str(), e.as_str()))
        .collect();

    manifest
        .modules
        .iter()
        .map(|m| {
            let loaded = match m.kind.as_str() {
                "connector" => loaded_connectors.contains(m.name.as_str()),
                "handler" => loaded_handlers.contains(m.name.as_str()),
                _ => false,
            };
            ConnectorStatus {
                name: m.name.clone(),
                kind: m.kind.clone(),
                version: m.version.clone(),
                enabled: m.enabled,
                loaded,
                capabilities: m.capabilities.clone(),
                sync_interval_seconds: m.sync_interval_seconds,
                config: m.config.clone(),
                fs_roots: m.fs_roots.clone().unwrap_or_default(),
                load_error: load_errors.get(m.name.as_str()).map(|s| (*s).to_string()),
                last_synced_at: None,
                last_fact_count: None,
                last_errors: Vec::new(),
                last_duration_ms: None,
            }
        })
        .collect()
}

/// 一次 sync 后刷新注册表:把 per-connector 产出回写到匹配模块的运行时字段。
///
/// 在 [`run_sync`](`crate::commands::wasm::run_sync`)内调(手动 + 后台两路共用)。
/// `now_iso` 由调用方传(同一次 sync 所有 connector 共享一个时间戳)。handler / 禁用 /
/// 失败模块不在 `per_connector` 里 → 运行时字段保持 None/空(正确:它们没 sync)。
pub async fn update_connector_statuses(
    statuses: &tokio::sync::Mutex<Vec<ConnectorStatus>>,
    per_connector: &[engine_wasm::ConnectorSyncStatus],
    now_iso: String,
) {
    let mut guard = statuses.lock().await;
    for pcs in per_connector {
        if let Some(s) = guard.iter_mut().find(|s| s.name == pcs.name) {
            s.last_synced_at = Some(now_iso.clone());
            s.last_fact_count = Some(pcs.fact_count);
            s.last_errors = pcs.errors.clone();
            s.last_duration_ms = Some(pcs.duration_ms);
        }
    }
}

/// 列出全部模块的可观测状态(前端 ConnectorsPage 数据源)。
#[tauri::command]
pub async fn get_connectors_status(
    state: State<'_, AppState>,
) -> Result<Vec<ConnectorStatus>, String> {
    let guard = state.connector_statuses.lock().await;
    Ok(guard.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use engine_wasm::{ManifestFile, ModuleManifest, WasiVersion};

    fn module(name: &str, kind: &str, enabled: bool) -> ModuleManifest {
        ModuleManifest {
            name: name.into(),
            kind: kind.into(),
            wasm_path: format!("target/{name}.wasm"),
            version: "0.1.0".into(),
            capabilities: vec!["logging".into(), "http-client".into()],
            wasi_version: WasiVersion::P2,
            sync_interval_seconds: 30,
            enabled,
            config: Some(serde_json::json!({ "cluster": "vm" })),
            fs_roots: None,
            sha256: String::new(),
        }
    }

    fn manifest(modules: Vec<ModuleManifest>) -> ManifestFile {
        ManifestFile {
            schema_version: "1".into(),
            modules,
        }
    }

    #[test]
    fn seed_reflects_disabled_loaded_and_failed() {
        // k8s: enabled+loaded; hello-world: enabled=false(禁用); broken: enabled but 缺 wasm
        // → 进 load_errors。用空 runtime(无 entries)+ 一条 load_errors 模拟 broken。
        let m = manifest(vec![
            module("k8s", "connector", true),
            module("hello-world", "connector", false),
            module("broken", "connector", true),
        ]);
        let runtime = engine_wasm::WasmRuntime::empty(std::path::PathBuf::new());
        // 模拟 k8s 已加载进 entries:空 runtime 没有,故 k8s loaded=false —— 为测 loaded=true
        // 分支,改用真 from_manifest 不现实(要 wasm)。直接断言 seed 逻辑:enabled=false
        // -> loaded=false 无 error;enabled=true 未进 entries -> loaded=false 无 error;
        // 失败进 load_errors -> load_error=Some。
        let statuses = seed_connector_statuses(&m, &runtime);
        assert_eq!(statuses.len(), 3);
        let by_name: HashMap<&str, &ConnectorStatus> =
            statuses.iter().map(|s| (s.name.as_str(), s)).collect();
        // k8s: enabled 但 runtime 空 -> loaded=false,无 load_error(没进 load_errors)
        assert!(by_name["k8s"].enabled);
        assert!(!by_name["k8s"].loaded);
        assert!(by_name["k8s"].load_error.is_none());
        // hello-world: enabled=false
        assert!(!by_name["hello-world"].enabled);
        assert!(!by_name["hello-world"].loaded);
        // 运行时字段初始全 None/空
        assert!(by_name["k8s"].last_synced_at.is_none());
        assert!(by_name["k8s"].last_errors.is_empty());
    }

    #[test]
    fn seed_records_load_error_from_runtime() {
        let m = manifest(vec![module("broken", "connector", true)]);
        let mut runtime = engine_wasm::WasmRuntime::empty(std::path::PathBuf::new());
        runtime.load_errors.push(("broken".into(), "wasm file not found".into()));
        let statuses = seed_connector_statuses(&m, &runtime);
        assert_eq!(statuses.len(), 1);
        assert!(statuses[0].enabled);
        assert!(!statuses[0].loaded);
        assert_eq!(
            statuses[0].load_error.as_deref(),
            Some("wasm file not found")
        );
    }

    #[tokio::test]
    async fn update_fills_runtime_fields_for_synced_connectors() {
        let m = manifest(vec![
            module("k8s", "connector", true),
            module("prometheus", "connector", true),
        ]);
        let runtime = engine_wasm::WasmRuntime::empty(std::path::PathBuf::new());
        let statuses = tokio::sync::Mutex::new(seed_connector_statuses(&m, &runtime));

        // 模拟一次 sync_all 的 per_connector:k8s 出 5 fact 0 错,prometheus 出 0 fact 1 错。
        let per = vec![
            engine_wasm::ConnectorSyncStatus {
                name: "k8s".into(),
                fact_count: 5,
                errors: vec![],
                duration_ms: 120,
            },
            engine_wasm::ConnectorSyncStatus {
                name: "prometheus".into(),
                fact_count: 0,
                errors: vec!["timeout".into()],
                duration_ms: 5000,
            },
        ];
        update_connector_statuses(&statuses, &per, "2026-07-26T01:00:00Z".into()).await;

        let guard = statuses.lock().await;
        let by_name: HashMap<&str, &ConnectorStatus> =
            guard.iter().map(|s| (s.name.as_str(), s)).collect();
        assert_eq!(by_name["k8s"].last_fact_count, Some(5));
        assert_eq!(by_name["k8s"].last_duration_ms, Some(120));
        assert_eq!(by_name["k8s"].last_synced_at.as_deref(), Some("2026-07-26T01:00:00Z"));
        assert!(by_name["k8s"].last_errors.is_empty());
        assert_eq!(by_name["prometheus"].last_fact_count, Some(0));
        assert_eq!(by_name["prometheus"].last_errors, vec!["timeout".to_string()]);
    }

    #[tokio::test]
    async fn update_leaves_unsynced_modules_untouched() {
        // handler / 禁用模块不在 per_connector 里 -> 运行时字段保持 None。
        let m = manifest(vec![
            module("k8s", "connector", true),
            module("k8s-handler", "handler", true),
        ]);
        let runtime = engine_wasm::WasmRuntime::empty(std::path::PathBuf::new());
        let statuses = tokio::sync::Mutex::new(seed_connector_statuses(&m, &runtime));
        let per = vec![engine_wasm::ConnectorSyncStatus {
            name: "k8s".into(),
            fact_count: 3,
            errors: vec![],
            duration_ms: 10,
        }];
        update_connector_statuses(&statuses, &per, "2026-07-26T01:00:00Z".into()).await;
        let guard = statuses.lock().await;
        let handler = guard.iter().find(|s| s.name == "k8s-handler").unwrap();
        assert!(handler.last_synced_at.is_none(), "handler never syncs");
        assert!(handler.last_fact_count.is_none());
    }
}
