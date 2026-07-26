//! wasm commands —— 把 `engine-wasm::WasmRuntime` 暴露给前端。
//!
//! 两条入口:
//! - [`list_connectors`] —— 列当前加载的 connector(name / version / kind /
//!   sync_interval),前端启动时拉一次,以及 sync 失败时 fallback 显示
//! - [`sync_all_now`] —— 立即触发一次 sync_all,返聚合 Fact 列表 +
//!   per-connector 状态。前端「立即同步」按钮调
//!
//! 设计要点:
//!
//! 1. **`WasmRuntime` 持 `Mutex<WasmConnector>`**(wasmtime Store !Sync),所以
//!    可以直接 `.manage(runtime)` 到 Tauri state,`&self` 路径走 `sync_all` /
//!    `entries` 读取都安全
//! 2. **DTO 与 engine_core::Fact 解耦** —— Fact 是 host 内部规范型,前端要的是
//!    serde 友好的 camel/snake 字段。这里逐字段平移,避免对 engine_core 加
//!    `#[derive(Serialize)]` 把它的 ABI 锚定到 JSON
//! 3. **错误透出为 String** —— Tauri command 默认 `Result<T, E: Serialize>`,
//!    `anyhow::Error` 不 Serialize,所以全部 `.map_err(|e| e.to_string())`

use serde::Serialize;
use tauri::State;

use crate::AppState;

/// 单个 connector 的静态元信息(给前端列表展示用)。
#[derive(Debug, Clone, Serialize)]
pub struct ConnectorInfo {
    /// connector 名,manifest.toml [[modules]].name。
    pub name: String,
    /// SemVer 版本。
    pub version: String,
    /// 模块类型(本期固定 "connector",Phase 3 起会有 rule / handler)。
    pub kind: String,
    /// 周期同步间隔(秒)—— Phase 3 tick_loop 时使用。
    pub sync_interval_seconds: u64,
    /// 申明的 capability(logging/clock/http-client...)。
    pub capabilities: Vec<String>,
}

/// 单条 Fact —— engine_core::Fact 的 serde 镜像。
///
/// 字段顺序与 fact_schema() Arrow Schema 对齐(7 列),方便前端拼表头。
#[derive(Debug, Clone, Serialize)]
pub struct FactDto {
    pub id: String,
    pub kind: String,
    pub source: String,
    pub resource_id: String,
    pub resource_type: String,
    pub timestamp: u64,
    pub attributes_json: String,
}

impl From<&engine_core::Fact> for FactDto {
    fn from(f: &engine_core::Fact) -> Self {
        Self {
            id: f.id.clone(),
            kind: f.kind.clone(),
            source: f.source.clone(),
            resource_id: f.resource_id.clone(),
            resource_type: f.resource_type.clone(),
            timestamp: f.timestamp,
            attributes_json: f.attributes_json.clone(),
        }
    }
}

/// 单 connector 在一次 sync_all 内的产出 + 错误。
#[derive(Debug, Clone, Serialize)]
pub struct ConnectorStatusDto {
    pub name: String,
    pub fact_count: usize,
    pub errors: Vec<String>,
    /// 本次 guest 自报耗时(毫秒)。Phase 6 connectors-ui。
    pub duration_ms: u64,
}

/// `sync_all_now` 的返回。
#[derive(Debug, Clone, Serialize)]
pub struct SyncSummaryDto {
    /// 全 connector 的 Fact 聚合(Arrow batch flatten 成 JSON list)。
    pub facts: Vec<FactDto>,
    /// 每个 connector 的明细。
    pub per_connector: Vec<ConnectorStatusDto>,
    /// 总 non-fatal 错误数。
    pub total_errors: u64,
    /// guest 自报的总耗时(毫秒)。
    pub total_duration_ms: u64,
    /// 本次 resolve→diff 相对上次 materialized 拓扑的增量(Phase 2.5)。
    pub changes: ChangeSummaryDto,
}

/// materialized 拓扑增量计数 —— `engine_identity::ChangeSummary` 的 serde 镜像。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ChangeSummaryDto {
    /// upsert 的节点数(新增 + 属性变化)。
    pub nodes_upserted: usize,
    /// 删除的节点数。
    pub nodes_removed: usize,
    /// upsert 的边数。
    pub edges_upserted: usize,
    /// 删除的边数。
    pub edges_removed: usize,
}

impl From<engine_identity::ChangeSummary> for ChangeSummaryDto {
    fn from(s: engine_identity::ChangeSummary) -> Self {
        Self {
            nodes_upserted: s.nodes_upserted,
            nodes_removed: s.nodes_removed,
            edges_upserted: s.edges_upserted,
            edges_removed: s.edges_removed,
        }
    }
}

/// 列出当前 runtime 加载的 connector。
///
/// 不调 wasm,只读 state.entries 元信息,可以走 sync command(无 await)。
#[tauri::command]
pub fn list_connectors(state: State<'_, AppState>) -> Vec<ConnectorInfo> {
    state
        .runtime
        .entries
        .iter()
        .map(|e| ConnectorInfo {
            name: e.name.clone(),
            version: e.manifest.version.clone(),
            kind: e.manifest.kind.clone(),
            sync_interval_seconds: e.manifest.sync_interval_seconds,
            capabilities: e.manifest.capabilities.clone(),
        })
        .collect()
}

/// 立即触发一次 sync_all,返聚合 Fact + per-connector 状态。
///
/// `config_json` 给所有 connector 同样的 config —— 本期前端默认传 `"{}"`,
/// 每 connector 各自的 config 注入留 Phase 3(manifest 加 `[modules.config]`)。
#[tauri::command]
pub async fn sync_all_now(
    state: State<'_, AppState>,
    config_json: Option<String>,
) -> Result<SyncSummaryDto, String> {
    let cfg = config_json.as_deref().unwrap_or("{}");
    run_sync(&state, cfg).await
}

/// sync 管线主体(sync -> upsert facts -> resolve+merge+diff -> detect_changes 自动录
/// -> apply_change_set -> 返回增量)。`sync_all_now` command 与后台 `sync_loop` 共用。
pub async fn run_sync(
    state: &AppState,
    config_json: &str,
) -> Result<SyncSummaryDto, String> {
    let summary = state.runtime.sync_all(config_json).await;

    // Phase 6 connectors-ui - 刷新 connector 状态注册表(此处更新覆盖手动 sync_all_now
    // 与后台 sync_loop 两路 —— run_sync 是共用管线)。last_synced_at/fact_count/
    // errors/duration 回写;handler/禁用/失败模块不在 per_connector 里 → 保持 None。
    // 借用 summary.per_connector(下方仍要 move 用),锁内不跨 await。
    crate::commands::connectors::update_connector_statuses(
        &state.connector_statuses,
        &summary.per_connector,
        engine_changes::iso::now_iso(),
    )
    .await;

    // 1. raw facts 落 append-only 真相源
    state
        .storage
        .upsert_facts(summary.batch.as_slice())
        .await
        .map_err(|e| e.to_string())?;

    // 2. Identity Resolver v0:resolve(最新 topology facts)→ 合入 metric-derived
    //    health(doc/11 §4.3)→ diff(当前 materialized)→ apply。materialized 表是
    //    get_graph 的读源。
    let facts = state
        .storage
        .latest_topology_facts()
        .await
        .map_err(|e| e.to_string())?;
    let mut next = engine_identity::resolve(&facts);
    // Phase 2.7 - 把 prometheus metric Fact 按阈值推成 health,合进 topology 节点
    // (worst-severity v0 仲裁,doc/11 §4.3)。真 Prom 不可用时 metric_facts 为空
    // -> no-op,k8s phase health 照常。
    let metric_facts = state
        .storage
        .latest_metric_facts()
        .await
        .map_err(|e| e.to_string())?;
    if !metric_facts.is_empty() {
        next = engine_identity::merge_metric_health(
            &next,
            &metric_facts,
            &engine_identity::HealthThresholds::default(),
        );
    }
    let current = state
        .storage
        .materialized_topology()
        .await
        .map_err(|e| e.to_string())?;
    let change_set = engine_identity::diff(&current, &next);
    // Phase 4.3 后续 - k8s 变更自动录(poll-diff):非首次 sync 时,detect_changes(current,next)
    // -> record_change。首次 sync 抑制(对齐 reference first_sync,防重启录历史 burst)。
    // 必须在 apply_change_set 之前读 current 旧 attrs(之后 materialized 表被覆盖)。
    if state
        .first_sync_done
        .swap(true, std::sync::atomic::Ordering::Relaxed)
    {
        let reqs = engine_changes::detect_changes(&current, &next);
        if !reqs.is_empty() {
            let events: Vec<engine_changes::ChangeEvent> = {
                let mut reg = state.change_events.lock().await;
                reqs.iter()
                    .filter_map(|req| {
                        engine_changes::record_change(&mut reg, &next, req)
                            .map_err(|e| {
                                tracing::warn!(
                                    "record_change {} failed: {e}",
                                    req.target_resource_id
                                );
                                e
                            })
                            .ok()
                    })
                    .collect()
            };
            for event in events {
                if let Err(e) = state.storage.upsert_change_event(&event).await {
                    tracing::warn!("upsert_change_event {}: {e}", event.change_event_id);
                }
            }
        }
    }

    // kind="change" facts(flagd / k8s-events connector 产)-> 解码 ChangeRequest -> record_change。
    // 与上面 detect_changes 同款 change-recording;guest 自管 baseline/dedup,故不经 first_sync gate。
    let change_facts: Vec<&engine_core::Fact> = summary
        .batch
        .as_slice()
        .iter()
        .filter(|f| f.kind == "change")
        .collect();
    if !change_facts.is_empty() {
        let events: Vec<engine_changes::ChangeEvent> = {
            let mut reg = state.change_events.lock().await;
            change_facts
                .iter()
                .filter_map(|f| match engine_changes::record_change(
                    &mut reg,
                    &next,
                    &decode_change_fact(f),
                ) {
                    Ok(ev) => Some(ev),
                    Err(e) => {
                        tracing::warn!("record_change(change-fact {}) failed: {e}", f.id);
                        None
                    }
                })
                .collect()
        };
        for event in events {
            if let Err(e) = state.storage.upsert_change_event(&event).await {
                tracing::warn!("upsert_change_event {}: {e}", event.change_event_id);
            }
        }
    }

    state
        .storage
        .apply_change_set(&change_set)
        .await
        .map_err(|e| e.to_string())?;

    let facts: Vec<FactDto> = summary.batch.as_slice().iter().map(FactDto::from).collect();
    let per_connector = summary
        .per_connector
        .into_iter()
        .map(|s| ConnectorStatusDto {
            name: s.name,
            fact_count: s.fact_count,
            errors: s.errors,
            duration_ms: s.duration_ms,
        })
        .collect();
    Ok(SyncSummaryDto {
        facts,
        per_connector,
        total_errors: summary.total_errors,
        total_duration_ms: summary.total_duration_ms,
        changes: change_set.summary().into(),
    })
}

/// 把 `kind="change"` Fact 的 attributes_json 解码成 [`engine_changes::ChangeRequest`]
/// (flagd / k8s-events connector 产的 ChangeEvent 载荷)。字段缺失 -> Default;
/// `record_change` 再校验 `change_type` / `source`。
fn decode_change_fact(f: &engine_core::Fact) -> engine_changes::ChangeRequest {
    let v: serde_json::Value = serde_json::from_str(&f.attributes_json).unwrap_or_default();
    let obj = v.as_object();
    let s = |k: &str| {
        obj.and_then(|o| o.get(k))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    engine_changes::ChangeRequest {
        change_type: s("change_type"),
        target_resource_id: s("target_resource_id"),
        changed_by: s("changed_by"),
        source: s("source"),
        description: s("description"),
        diff_summary: obj
            .and_then(|o| o.get("diff_summary"))
            .cloned()
            .unwrap_or_else(|| serde_json::json!({})),
        cluster_id: s("cluster_id"),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fact_dto_mirrors_engine_core_fact() {
        let f = engine_core::Fact {
            id: "x".into(),
            kind: "topology-node".into(),
            source: "hello".into(),
            resource_id: "rid".into(),
            resource_type: "Namespace".into(),
            timestamp: 1_700_000_000,
            attributes_json: r#"{"a":1}"#.into(),
        };
        let dto = FactDto::from(&f);
        assert_eq!(dto.id, "x");
        assert_eq!(dto.timestamp, 1_700_000_000);
        assert_eq!(dto.attributes_json, r#"{"a":1}"#);

        // 全字段 JSON round-trip — 防 serde rename 漏字段
        let j = serde_json::to_value(&dto).expect("serialize");
        let obj = j.as_object().expect("object");
        assert_eq!(obj.len(), 7, "FactDto must serialize exactly 7 fields");
        for k in [
            "id",
            "kind",
            "source",
            "resource_id",
            "resource_type",
            "timestamp",
            "attributes_json",
        ] {
            assert!(obj.contains_key(k), "missing field {k} in serialized DTO");
        }
    }

    #[test]
    fn sync_summary_dto_serializes_with_expected_keys() {
        let dto = SyncSummaryDto {
            facts: vec![],
            per_connector: vec![ConnectorStatusDto {
                name: "k8s-mini".into(),
                fact_count: 3,
                errors: vec!["nope".into()],
                duration_ms: 42,
            }],
            total_errors: 1,
            total_duration_ms: 42,
            changes: ChangeSummaryDto {
                nodes_upserted: 2,
                nodes_removed: 1,
                edges_upserted: 1,
                edges_removed: 0,
            },
        };
        let j = serde_json::to_value(&dto).expect("serialize");
        assert_eq!(j["total_errors"], 1);
        assert_eq!(j["total_duration_ms"], 42);
        assert_eq!(j["per_connector"][0]["name"], "k8s-mini");
        assert_eq!(j["per_connector"][0]["fact_count"], 3);
        assert_eq!(j["per_connector"][0]["errors"][0], "nope");
        assert_eq!(j["per_connector"][0]["duration_ms"], 42);
        assert_eq!(j["changes"]["nodes_upserted"], 2);
        assert_eq!(j["changes"]["nodes_removed"], 1);
        assert_eq!(j["changes"]["edges_upserted"], 1);
        assert_eq!(j["changes"]["edges_removed"], 0);
    }
}
