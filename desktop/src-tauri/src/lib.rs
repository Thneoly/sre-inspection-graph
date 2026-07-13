//! sre-graph-desktop — Tauri 2.x backend lib(可被 mobile target 复用)。
//!
//! 启动序列(F — Tauri ↔ engine-wasm 串通):
//!
//! 1. tracing-subscriber 初始化日志(env-filter,默认 info)
//! 2. 解析 modules_root —— env `MODULES_ROOT` 优先,否则取 `CARGO_MANIFEST_DIR`
//!    上溯两层(repo/desktop/src-tauri → repo/modules)
//! 3. 读 manifest.toml + `tauri::async_runtime::block_on(WasmRuntime::from_manifest)`
//!    —— Tauri 2.x 内置 tokio runtime,在 builder 之前同步等待加载
//! 4. **失败不阻塞 UI** —— wasm 没 build / manifest 缺失 → 用空 WasmRuntime
//!    fallback + log warn,前端列表为空时引导用户运行 `cd modules && cargo wasi-build`
//! 5. 初始化 SQLite storage 并 `.manage(AppState)` 注入 Tauri state
//!
//! Phase 2 起按 doc/17 §4.2 拆分 `commands/{topology, recovery, change_events,
//! reports, connectors, fault_simulation}.rs`。

pub mod commands;

use std::path::PathBuf;
use std::sync::Mutex;

use engine_storage::SqliteStorage;
use engine_wasm::{ManifestFile, WasmRuntime};
use tauri::Manager;

use commands::alerts::{
    correlate_changes_for_alert, get_alert, list_alerts, record_alert, resolve_alert,
};
use commands::change_events::{
    change_event_alerts, change_event_impact, change_event_recovery_suggestion, correlated_changes,
    frequent_changes, get_change_event, list_change_events, record_change_event,
};
use commands::proxy::{proxy_status, start_kubectl_proxy, stop_kubectl_proxy};
use commands::recovery::{
    abort_chain, cancel_chain, cancel_recovery_execution, confirm_chain,
    confirm_recovery_execution, dry_run_recovery, execute_chain, execute_recovery,
    get_recovery_action, get_recovery_chain, get_recovery_execution, get_chain_template,
    list_chain_templates, list_recovery_actions, list_recovery_chains, list_recovery_executions,
    recovery_suggestions_for_rule, reverify_recovery_execution, rollback_recovery_execution,
};
use commands::system::get_app_version;
use commands::topology::{get_graph, get_topology};
use commands::wasm::{list_connectors, sync_all_now};

/// 解析 modules/ 根目录。
///
/// 优先级:
/// 1. `MODULES_ROOT` env(测试 / CI / 用户自定义)
/// 2. `CARGO_MANIFEST_DIR/../../modules` —— dev build 时 manifest 是
///    `desktop/src-tauri/Cargo.toml`,上溯两层正好到 repo/modules
///
/// Phase 2 上 bundle 后,modules wasm 会随 resources 打包,这里再加 bundle-resource
/// 兜底。本期 dev 路径就够。
fn resolve_modules_root() -> PathBuf {
    if let Ok(v) = std::env::var("MODULES_ROOT") {
        return PathBuf::from(v);
    }
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir).join("../../modules")
}

fn resolve_storage_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    if let Ok(v) = std::env::var("SRE_GRAPH_DB_PATH") {
        return Ok(PathBuf::from(v));
    }
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("resolve app data dir: {e}"))?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("create app data dir {}: {e}", dir.display()))?;
    Ok(dir.join("sre-graph.sqlite"))
}

/// 在 builder 之前同步加载 WasmRuntime。
///
/// 失败 fallback 到 empty runtime,而不是 panic —— 用户首跑可能 modules 还没 build,
/// 让 UI 起来但 connector 列表空,前端提示 build 步骤。
fn load_wasm_runtime() -> WasmRuntime {
    let modules_root = resolve_modules_root();
    let manifest_path = modules_root.join("manifest.toml");
    let toml_text = match std::fs::read_to_string(&manifest_path) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(
                path = %manifest_path.display(),
                error = %e,
                "manifest.toml not found — desktop will start with empty wasm runtime; \
                 run `cd modules && cargo wasi-build` first"
            );
            return WasmRuntime::empty(modules_root);
        }
    };
    let manifest = match ManifestFile::from_toml_str(&toml_text) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(
                path = %manifest_path.display(),
                error = %e,
                "manifest.toml parse failed — desktop will start with empty wasm runtime"
            );
            return WasmRuntime::empty(modules_root);
        }
    };
    match tauri::async_runtime::block_on(WasmRuntime::from_manifest(&modules_root, &manifest)) {
        Ok(rt) => {
            tracing::info!(
                modules_root = %modules_root.display(),
                connectors = rt.connector_count(),
                load_errors = rt.load_errors.len(),
                names = ?rt.connector_names(),
                "wasm runtime ready"
            );
            for (name, err) in &rt.load_errors {
                tracing::warn!(connector = %name, error = %err, "connector load failed");
            }
            rt
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                "WasmRuntime::from_manifest failed — fallback to empty runtime"
            );
            WasmRuntime::empty(modules_root)
        }
    }
}

/// Tauri shared state.
pub struct AppState {
    /// WASM connector runtime.
    pub runtime: WasmRuntime,
    /// Local SQLite storage.
    pub storage: SqliteStorage,
    /// desktop 托管的 kubectl proxy 子进程(Phase 2.7)。`std::sync::Mutex` 便于在
    /// `RunEvent::Exit` 同步回调里 kill;command 端持锁时间极短(不跨 await)。
    pub proxy: Mutex<Option<tokio::process::Child>>,
    /// Phase 3.6 - recovery 执行注册表(单机确认门 + mock handler twin)。启动从
    /// `recovery_executions` 表载入;每次 execute/confirm/rollback 后 upsert 回写。
    pub recovery_executions: Mutex<engine_recovery::ExecutionRegistry>,
    /// Phase 3.6 - recovery chain 注册表。启动从 `recovery_chains` 表载入。
    pub recovery_chains: Mutex<engine_recovery::ChainRegistry>,
    /// Phase 3.6 - change event 注册表。启动从 `change_events` 表载入;
    /// `record_change_event` 后 upsert。
    pub change_events: Mutex<engine_changes::ChangeRegistry>,
    /// Phase 3.6 - alert 注册表(无 live 源,k8s-watch/webhook 延后;仅手动 record_alert)。
    pub alerts: Mutex<engine_changes::AlertRegistry>,
    /// Phase 4.1 - 报告注册表(内存;SQLite 持久化留后续)。
    pub reports: Mutex<engine_reports::ReportStore>,
}

/// Tauri app builder 入口。`main.rs` 调用,确保 macOS / iOS 共享同一 builder。
pub fn run() {
    // tracing-subscriber 失败(已被初始化)不致命,可忽略
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    let runtime = load_wasm_runtime();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(move |app| {
            let storage_path = resolve_storage_path(app.handle())?;
            let (storage, recovery_executions, recovery_chains, change_events, alerts) =
                tauri::async_runtime::block_on(async {
                    let storage = SqliteStorage::connect(&storage_path)
                        .await
                        .map_err(|e| format!("connect sqlite {}: {e}", storage_path.display()))?;
                    storage
                        .migrate()
                        .await
                        .map_err(|e| format!("migrate sqlite {}: {e}", storage_path.display()))?;
                    // Phase 3.6 - 启动从 storage 载入 4 个 registry(重启恢复)
                    let execs = storage
                        .list_recovery_executions(1000)
                        .await
                        .map_err(|e| format!("load recovery_executions: {e}"))?;
                    let chains = storage
                        .list_recovery_chains(1000)
                        .await
                        .map_err(|e| format!("load recovery_chains: {e}"))?;
                    let changes = storage
                        .list_change_events(1000)
                        .await
                        .map_err(|e| format!("load change_events: {e}"))?;
                    let alerts = storage
                        .list_alert_events(1000)
                        .await
                        .map_err(|e| format!("load alert_events: {e}"))?;
                    Ok::<_, String>((
                        storage,
                        engine_recovery::ExecutionRegistry::from_executions(execs),
                        engine_recovery::ChainRegistry::from_chains(chains),
                        engine_changes::ChangeRegistry::from_events(changes),
                        engine_changes::AlertRegistry::from_alerts(alerts),
                    ))
                })?;
            tracing::info!(path = %storage_path.display(), "sqlite storage ready");
            app.manage(AppState {
                runtime,
                storage,
                proxy: Mutex::new(None),
                recovery_executions: Mutex::new(recovery_executions),
                recovery_chains: Mutex::new(recovery_chains),
                change_events: Mutex::new(change_events),
                alerts: Mutex::new(alerts),
                reports: Mutex::new(engine_reports::ReportStore::new()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_app_version,
            list_connectors,
            sync_all_now,
            get_topology,
            get_graph,
            start_kubectl_proxy,
            stop_kubectl_proxy,
            proxy_status,
            // Phase 3.6 - recovery (PRD-001)
            list_recovery_actions,
            get_recovery_action,
            dry_run_recovery,
            recovery_suggestions_for_rule,
            execute_recovery,
            list_recovery_executions,
            get_recovery_execution,
            confirm_recovery_execution,
            cancel_recovery_execution,
            rollback_recovery_execution,
            reverify_recovery_execution,
            list_chain_templates,
            get_chain_template,
            execute_chain,
            confirm_chain,
            cancel_chain,
            abort_chain,
            list_recovery_chains,
            get_recovery_chain,
            // Phase 3.6 - change_events (PRD-002)
            record_change_event,
            list_change_events,
            get_change_event,
            correlated_changes,
            frequent_changes,
            change_event_impact,
            change_event_recovery_suggestion,
            change_event_alerts,
            // Phase 3.6 - alerts
            record_alert,
            list_alerts,
            get_alert,
            resolve_alert,
            correlate_changes_for_alert,
            // Phase 4.1 - reports (PRD-003)
            commands::reports::generate_report_cmd,
            commands::reports::list_reports,
            commands::reports::get_report,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // 退出时杀掉托管的 kubectl proxy,不留孤儿进程。
            if let tauri::RunEvent::Exit = event {
                if let Some(state) = app.try_state::<AppState>() {
                    if let Ok(mut guard) = state.proxy.lock() {
                        if let Some(mut child) = guard.take() {
                            let _ = child.start_kill();
                        }
                    }
                }
            }
        });
}
