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
pub mod email_smtp;
pub mod scheduler;
pub mod sync_loop;

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
use commands::views::{
    access_link, alert_aggregation, config_impact, image_risk, list_resources_by_types, node_impact,
};
use commands::wasm::{list_connectors, sync_all_now};
use commands::connectors::{get_connectors_status, seed_connector_statuses, ConnectorStatus};
use commands::reports_scheduler::{
    create_subscription, delete_subscription, get_subscription, list_sent_emails,
    list_subscriptions, trigger_subscription_now, update_subscription,
};

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

/// 在 builder 之前同步加载 WasmRuntime,并从 manifest 播种 connector 状态注册表。
///
/// 失败 fallback 到 empty runtime,而不是 panic —— 用户首跑可能 modules 还没 build,
/// 让 UI 起来但 connector 列表空,前端提示 build 步骤。返 `(runtime, statuses)`:
/// manifest 解析成功即播种(即便 runtime 加载全失败,模块表仍反映 manifest + 全部 not-loaded)。
fn load_wasm_runtime() -> (WasmRuntime, Vec<ConnectorStatus>) {
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
            return (WasmRuntime::empty(modules_root), Vec::new());
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
            return (WasmRuntime::empty(modules_root), Vec::new());
        }
    };
    let rt = match tauri::async_runtime::block_on(WasmRuntime::from_manifest(&modules_root, &manifest)) {
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
    };
    // 播种状态表:从完整 manifest 建(禁用/失败模块也入表),runtime 决定 loaded/load_error。
    let statuses = seed_connector_statuses(&manifest, &rt);
    (rt, statuses)
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
    pub recovery_executions: tokio::sync::Mutex<engine_recovery::ExecutionRegistry>,
    /// Phase 3.6 - recovery chain 注册表。启动从 `recovery_chains` 表载入。
    pub recovery_chains: tokio::sync::Mutex<engine_recovery::ChainRegistry>,
    /// Phase 3.6 - change event 注册表。启动从 `change_events` 表载入;
    /// `record_change_event` 后 upsert。
    pub change_events: tokio::sync::Mutex<engine_changes::ChangeRegistry>,
    /// Phase 3.6 - alert 注册表(无 live 源,k8s-watch/webhook 延后;仅手动 record_alert)。
    pub alerts: tokio::sync::Mutex<engine_changes::AlertRegistry>,
    /// Phase 4.1 - 报告注册表(内存;SQLite 持久化留后续)。
    pub reports: tokio::sync::Mutex<engine_reports::ReportStore>,
    /// Phase 4.3 - 报告订阅注册表。启动从 `report_subscriptions` 表载入;
    /// create/update/delete + 调度触发后 upsert 回写。
    pub subscriptions: tokio::sync::Mutex<engine_reports::SubscriptionStore>,
    /// Phase 4.3 - 邮件发送器(SMTP_HOST 空 -> InMemory 回退)。
    pub email_sender: std::sync::Arc<dyn engine_reports::EmailSender>,
    /// Phase 4.3 - 调度循环 task handle。`std::sync::Mutex` 便于 `RunEvent::Exit`
    /// 同步回调 abort;command 端不持锁。
    pub scheduler_handle: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    /// Phase 4.3 后续 - 后台 sync 循环 task handle(同上,RunEvent::Exit abort)。
    pub sync_loop_handle: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    /// Phase 4.3 后续 - k8s 变更自动录:首次 sync 抑制(对齐 reference first_sync,
    /// 防重启录历史 burst + 时间戳误导)。首次 sync 后置 true,后续 sync 跑 detect_changes。
    pub first_sync_done: std::sync::atomic::AtomicBool,
    pub handler_executor: std::sync::Arc<dyn engine_recovery::HandlerExecutor>,
    /// Phase 6 connectors-ui - connector/handler 可观测状态注册表。启动从 manifest 播种
    /// (禁用/失败模块也入表),每次 sync(手动 + 后台 loop)在 run_sync 内刷新
    /// last_synced_at/fact_count/errors/duration。前端 ConnectorsPage 读此表。
    pub connector_statuses: tokio::sync::Mutex<Vec<ConnectorStatus>>,
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

    let (runtime, connector_statuses) = load_wasm_runtime();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(move |app| {
            let storage_path = resolve_storage_path(app.handle())?;
            let (storage, recovery_executions, recovery_chains, change_events, alerts, subscriptions, reports) =
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
                    // Phase 4.3 - 载入报告订阅(调度配置不能丢)
                    let subs = storage
                        .list_subscriptions(1000)
                        .await
                        .map_err(|e| format!("load report_subscriptions: {e}"))?;
                    // Phase 4.3 后续 - 载入报告历史(跨重启恢复)
                    let rpts = storage
                        .list_reports(1000)
                        .await
                        .map_err(|e| format!("load report_tasks: {e}"))?;
                    Ok::<_, String>((
                        storage,
                        engine_recovery::ExecutionRegistry::from_executions(execs),
                        engine_recovery::ChainRegistry::from_chains(chains),
                        engine_changes::ChangeRegistry::from_events(changes),
                        engine_changes::AlertRegistry::from_alerts(alerts),
                        engine_reports::SubscriptionStore::from_subscriptions(subs),
                        engine_reports::ReportStore::from_tasks(rpts),
                    ))
                })?;
            tracing::info!(path = %storage_path.display(), "sqlite storage ready");
            // Phase 4.3 - 邮件发送器(SMTP_HOST 空 -> InMemory 回退)
            let email_sender = email_smtp::get_email_sender();
            // Phase 3.9a-3b2 - handler_executor:engine_wasm::WasmHandlerExecutor if
            // k8s-handler 加载,else Mock fallback。real_mode / api_base 经 env 控制:
            //   SRE_GRAPH_HANDLER_MODE=real -> 真改集群(默认 mock 保安全,对齐 reference RECOVERY_HANDLER_MODE)
            //   SRE_GRAPH_K8S_API_BASE -> K8s API base(默认 desktop 托管 kubectl proxy 的 8001)
            let real_mode = std::env::var("SRE_GRAPH_HANDLER_MODE")
                .map(|v| v == "real")
                .unwrap_or(false);
            let api_base = std::env::var("SRE_GRAPH_K8S_API_BASE")
                .unwrap_or_else(|_| "http://127.0.0.1:8001".to_string());
            let scale_handler = runtime
                .handlers
                .iter()
                .find(|h| h.name == "k8s-handler")
                .map(|h| h.handler.clone());
            let handler_executor: std::sync::Arc<dyn engine_recovery::HandlerExecutor> =
                if let Some(h) = scale_handler {
                    tracing::info!(
                        real_mode,
                        api_base = %api_base,
                        "handler_executor = WasmHandlerExecutor (SRE_GRAPH_HANDLER_MODE)",
                    );
                    std::sync::Arc::new(engine_wasm::WasmHandlerExecutor::new(
                        h,
                        api_base,
                        real_mode,
                    ))
                } else {
                    tracing::info!("handler_executor = MockHandlerExecutor (k8s-handler not loaded)");
                    std::sync::Arc::new(engine_recovery::MockHandlerExecutor)
                };
            app.manage(AppState {
                runtime,
                storage,
                proxy: Mutex::new(None),
                recovery_executions: tokio::sync::Mutex::new(recovery_executions),
                recovery_chains: tokio::sync::Mutex::new(recovery_chains),
                change_events: tokio::sync::Mutex::new(change_events),
                alerts: tokio::sync::Mutex::new(alerts),
                reports: tokio::sync::Mutex::new(reports),
                subscriptions: tokio::sync::Mutex::new(subscriptions),
                email_sender,
                scheduler_handle: Mutex::new(None),
                sync_loop_handle: Mutex::new(None),
                first_sync_done: std::sync::atomic::AtomicBool::new(false),
                handler_executor,
                connector_statuses: tokio::sync::Mutex::new(connector_statuses),
            });
            // Phase 4.3 - 起调度循环(60s tick);存 handle 供 RunEvent::Exit abort
            let app_handle = app.handle().clone();
            let join = tauri::async_runtime::spawn(scheduler::scheduler_loop(app_handle));
            if let Some(state) = app.try_state::<AppState>() {
                if let Ok(mut guard) = state.scheduler_handle.lock() {
                    *guard = Some(join);
                }
            }
            // Phase 4.3 后续 - 起后台 sync 循环(默认 30s,env 可配/禁用);detect_changes 自动触发
            let app_handle2 = app.handle().clone();
            let join2 = tauri::async_runtime::spawn(sync_loop::sync_loop(app_handle2));
            if let Some(state) = app.try_state::<AppState>() {
                if let Ok(mut guard) = state.sync_loop_handle.lock() {
                    *guard = Some(join2);
                }
            }
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
            commands::reports::clear_reports,
            // Phase 4.3 - report subscriptions / scheduling (PRD-003)
            create_subscription,
            list_subscriptions,
            get_subscription,
            update_subscription,
            delete_subscription,
            trigger_subscription_now,
            list_sent_emails,
            // Phase 5 - inspection views (reference view2-5)
            node_impact,
            config_impact,
            access_link,
            image_risk,
            list_resources_by_types,
            alert_aggregation,
            // Phase 6 - connectors-ui (connector 运行时观测)
            get_connectors_status,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // 退出时杀掉托管的 kubectl proxy + abort 调度循环,不留孤儿。
            if let tauri::RunEvent::Exit = event {
                if let Some(state) = app.try_state::<AppState>() {
                    if let Ok(mut guard) = state.proxy.lock() {
                        if let Some(mut child) = guard.take() {
                            let _ = child.start_kill();
                        }
                    }
                    if let Ok(mut guard) = state.scheduler_handle.lock() {
                        if let Some(join) = guard.take() {
                            join.abort();
                        }
                    }
                    if let Ok(mut guard) = state.sync_loop_handle.lock() {
                        if let Some(join) = guard.take() {
                            join.abort();
                        }
                    }
                }
            }
        });
}
