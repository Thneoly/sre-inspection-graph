//! 报告调度后台循环(Phase 4.3,对齐 reference `_run_subscription_safely` + APScheduler)。
//!
//! `tauri::async_runtime::spawn` 起 60s tick 循环,每 tick 遍历 enabled 订阅,调
//! `check_fire` 判定:Fire -> `run_subscription` 生成 + 发邮件 + 回写 last_*;MissedAdvance
//! -> 推进 last_run_at(消费 stale fire);NotDue -> 跳过。`RunEvent::Exit` abort 防孤儿。
//! 只在 app 开着时跑;关机期间漏发 >5min 不补跑(对齐 reference misfire_grace_time=300)。

#![allow(missing_docs)]

use std::time::Duration;

use chrono::Utc;
use tauri::{AppHandle, Manager};

use engine_reports::{
    check_fire, default_grace, parse_cron, run_subscription, FireDecision, ReportSubscription,
    SubscriptionStatus,
};

use crate::AppState;

/// 调度循环(60s tick)。spawn 后长期运行,RunEvent::Exit 时 abort。
pub async fn scheduler_loop(app: AppHandle) {
    let mut ticker = tokio::time::interval(Duration::from_secs(60));
    loop {
        ticker.tick().await;
        if let Err(e) = run_tick(&app).await {
            tracing::warn!("scheduler tick error: {e}");
        }
    }
}

/// 单次 tick:遍历订阅,按 FireDecision 跑/推进/跳过。
async fn run_tick(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let now = Utc::now();
    let now_iso = now.to_rfc3339();
    let grace = default_grace();

    // 1. 收集 enabled 订阅 + 各自 FireDecision(持 subscriptions 锁,释放后跑,避免跨锁)
    let actions: Vec<(ReportSubscription, FireDecision)> = {
        let subs = state.subscriptions.lock().await;
        subs.list(None)
            .into_iter()
            .filter_map(|s| {
                if !s.enabled {
                    return None;
                }
                let schedule = parse_cron(&s.cron).ok()?;
                let last_run = s.last_run_dt().unwrap_or(now);
                let decision = check_fire(&schedule, last_run, now, grace);
                Some((s.clone(), decision))
            })
            .collect()
    };

    // 2. 逐个处理(锁纪律:subscriptions 锁释放后再取 changes/executions 锁)
    for (sub, decision) in actions {
        match decision {
            FireDecision::NotDue => {}
            FireDecision::MissedAdvance(next_fire) => {
                advance_last_run(app, &sub.subscription_id, next_fire.to_rfc3339()).await?;
            }
            FireDecision::Fire => {
                if let Err(e) = fire_subscription(app, &sub, &now_iso).await {
                    // 回写 failed(不阻断其它订阅)
                    let _ = mark_failed(app, &sub.subscription_id, &now_iso, &e.to_string()).await;
                    tracing::warn!("scheduler sub {} failed: {e}", sub.subscription_id);
                }
            }
        }
    }
    Ok(())
}

/// Fire:生成报告 + 发邮件 + 回写 last_status=Ok。
async fn fire_subscription(app: &AppHandle, sub: &ReportSubscription, now_iso: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let topo = state
        .storage
        .materialized_topology()
        .await
        .map_err(|e| e.to_string())?;
    let result = {
        let changes = state.change_events.lock().await;
        let execs = state.recovery_executions.lock().await;
        run_subscription(sub, &topo, &changes, &execs, &*state.email_sender, now_iso).await
    };
    let r = result.map_err(|e| e.to_string())?;

    // 存报告(ReportStore 内存 + SQLite 持久化)
    state.reports.lock().await.add(r.task.clone());
    if let Err(e) = state.storage.upsert_report(&r.task).await {
        tracing::warn!("scheduler upsert_report {}: {e}", r.report_id);
    }

    // 回写订阅 last_*
    let snap = {
        let mut subs = state.subscriptions.lock().await;
        if let Some(s) = subs.get_mut(&sub.subscription_id) {
            s.last_run_at = now_iso.to_string();
            s.last_status = SubscriptionStatus::Ok;
            s.last_error.clear();
            s.last_report_id = r.report_id.clone();
            Some(s.clone())
        } else {
            None
        }
    };
    if let Some(snap) = snap {
        state
            .storage
            .upsert_subscription(&snap)
            .await
            .map_err(|e| e.to_string())?;
    }
    tracing::info!(
        "scheduler fired sub {} -> report {}",
        sub.subscription_id,
        r.report_id
    );
    Ok(())
}

/// MissedAdvance:推进 last_run_at 到 next_fire(消费 stale fire),不跑。
async fn advance_last_run(app: &AppHandle, sub_id: &str, next_fire_iso: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    let snap = {
        let mut subs = state.subscriptions.lock().await;
        if let Some(s) = subs.get_mut(sub_id) {
            s.last_run_at = next_fire_iso;
            Some(s.clone())
        } else {
            None
        }
    };
    if let Some(snap) = snap {
        state
            .storage
            .upsert_subscription(&snap)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 失败回写:last_status=Failed + last_error。
async fn mark_failed(app: &AppHandle, sub_id: &str, now_iso: &str, err: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let snap = {
        let mut subs = state.subscriptions.lock().await;
        if let Some(s) = subs.get_mut(sub_id) {
            s.last_run_at = now_iso.to_string();
            s.last_status = SubscriptionStatus::Failed;
            s.last_error = err.to_string();
            Some(s.clone())
        } else {
            None
        }
    };
    if let Some(snap) = snap {
        state
            .storage
            .upsert_subscription(&snap)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
