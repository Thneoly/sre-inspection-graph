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
    let summary = state.runtime.sync_all(cfg).await;
    state
        .storage
        .upsert_facts(summary.batch.as_slice())
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
        })
        .collect();
    Ok(SyncSummaryDto {
        facts,
        per_connector,
        total_errors: summary.total_errors,
        total_duration_ms: summary.total_duration_ms,
    })
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
            }],
            total_errors: 1,
            total_duration_ms: 42,
        };
        let j = serde_json::to_value(&dto).expect("serialize");
        assert_eq!(j["total_errors"], 1);
        assert_eq!(j["total_duration_ms"], 42);
        assert_eq!(j["per_connector"][0]["name"], "k8s-mini");
        assert_eq!(j["per_connector"][0]["fact_count"], 3);
        assert_eq!(j["per_connector"][0]["errors"][0], "nope");
    }
}
