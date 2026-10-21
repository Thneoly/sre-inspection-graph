//! 后台 sync 循环(Phase 4.3 后续,为 k8s-watch detect_changes 自动触发)。
//!
//! `tauri::async_runtime::spawn` 起 loop,每轮调 `run_sync`(与 `sync_all_now`
//! command 同管线)。首次轮跳过(前端启动已 sync 一次作 baseline),之后每
//! `SRE_GRAPH_SYNC_INTERVAL_SECS` 秒(默认 30,0=禁用)sync 一次 ->
//! detect_changes 在非首次 sync 时检测 real-cluster 变更自动录 ChangeEvent。
//! `RunEvent::Exit` abort 防孤儿。
//!
//! ## 退避(Phase D)
//!
//! 集群 / kubectl proxy 不可达时,4 个 http connector 串行吃满各自 30s 超时,
//! 无退避的固定 interval 会近乎背靠背地重试空转。故:
//! - 有错误(Err 或 total_errors>0)时下一轮间隔指数拉长(×2,封顶
//!   `BACKOFF_CAP_MULT`×base,默认 10min),全绿后复位;
//! - 用 `sleep` 动态间隔而非 `interval`(tokio interval 的默认 Burst 行为会在
//!   长 tick 后立即补发,正是要避免的)。

#![allow(missing_docs)]

use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::commands::wasm::run_sync;
use crate::AppState;

/// 退避倍数封顶(base 30s × 20 = 10min)。
const BACKOFF_CAP_MULT: u32 = 20;

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
    let base = Duration::from_secs(interval_secs);
    tracing::info!("background sync loop started (interval={interval_secs}s, backoff cap {}s)",
        base.as_secs() * BACKOFF_CAP_MULT as u64);
    let mut mult: u32 = 1;
    loop {
        tokio::time::sleep(base * mult).await;
        let state = app.state::<AppState>();
        let healthy = match run_sync(&state, "{}").await {
            Ok(s) => {
                tracing::debug!(
                    "background sync: {} facts, {} errors (backoff x{mult})",
                    s.facts.len(),
                    s.total_errors
                );
                s.total_errors == 0
            }
            Err(e) => {
                tracing::warn!("background sync error (backoff x{mult}): {e}");
                false
            }
        };
        mult = if healthy {
            1
        } else {
            (mult.saturating_mul(2)).min(BACKOFF_CAP_MULT)
        };
    }
}
