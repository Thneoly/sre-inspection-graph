//! 后台 sync 循环(Phase 4.3 后续,为 k8s-watch detect_changes 自动触发)。
//!
//! `tauri::async_runtime::spawn` 起 interval loop,每 tick 调 `run_sync`(与
//! `sync_all_now` command 同管线)。首次 tick 跳过(前端启动已 sync 一次作 baseline),
//! 之后每 `SRE_GRAPH_SYNC_INTERVAL_SECS` 秒(默认 30,0=禁用)sync 一次 ->
//! detect_changes 在非首次 sync 时检测 real-cluster 变更自动录 ChangeEvent。
//! `RunEvent::Exit` abort 防孤儿。

#![allow(missing_docs)]

use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::commands::wasm::run_sync;
use crate::AppState;

/// 后台 sync 循环。spawn 后长期运行,RunEvent::Exit 时 abort。
pub async fn sync_loop(app: AppHandle) {
    let interval_secs: u64 = std::env::var("SRE_GRAPH_SYNC_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    if interval_secs == 0 {
        tracing::info!("background sync loop disabled (SRE_GRAPH_SYNC_INTERVAL_SECS=0)");
        return;
    }
    tracing::info!("background sync loop started (interval={interval_secs}s)");
    let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
    // 跳过首次 immediate tick(前端启动已 sync 一次作 baseline;首次 sync 抑制 detect)
    ticker.tick().await;
    loop {
        ticker.tick().await;
        let state = app.state::<AppState>();
        match run_sync(&state, "{}").await {
            Ok(s) => tracing::debug!(
                "background sync: {} facts, {} errors",
                s.facts.len(),
                s.total_errors
            ),
            Err(e) => tracing::warn!("background sync error: {e}"),
        }
    }
}
