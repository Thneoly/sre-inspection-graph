//! `WasmRuntime` —— 持多个 [`WasmConnector`] 的编排器。
//!
//! 一个 host 进程通常会同时跑 N 个 connector(k8s / prometheus / jaeger / ...),
//! 每个对应 `modules/manifest.toml` 里的一条 `[[modules]]`。`WasmRuntime` 把它们
//! 组装到一个结构里,提供两个层级的入口:
//!
//! - [`WasmRuntime::sync_all`] —— 一次性跑所有 connector,返一个聚合
//!   [`engine_core::FactBatch`](`engine_core::FactBatch`) + 每个 connector 的状态
//! - [`WasmRuntime::tick_loop`] —— Phase 3 的留口,周期跑 `sync_all`(本期仅
//!   骨架,真上线之前会按 connector 各自 `sync_interval_seconds` 调度,不是一刀切)
//!
//! 设计要点:
//!
//! 1. **manifest 入口 + 模块根目录**:`from_manifest(modules_root, manifest)` 拼
//!    `modules_root.join(module.wasm_path)`,与 `engine-cli`/测试 / Tauri 三处对齐
//! 2. **kind 过滤**:Phase 2 只加载 `kind == "connector"` 的模块,rule/handler
//!    在 Phase 3 引入(各有不同的 WIT world,需要单独的 runtime)
//! 3. **失败隔离**:某个模块 wasm 文件不存在 / wasmtime instantiate 失败 → 记入
//!    `load_errors`,不中断其它模块加载。`sync_all` 同理 —— 单个 connector sync
//!    报错只让它 `errors` 字段记录,不影响其它
//! 4. **Send + Sync 边界**:`WasmConnector` 持 `Store<State>` 不是 Send 安全的
//!    (wasmtime Store 是 `!Sync`),所以 `WasmRuntime` 内部用 `Mutex` 包每个
//!    connector;`sync_all` 是 sequential await,Phase 3 真要并发 sync 再上
//!    `tokio::task::spawn_local` 或者把 connector 拆到独立 task

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use engine_core::FactBatch;
use tokio::sync::Mutex;

use crate::runtime::{SyncOutcome, WasmConnector};
use crate::{ManifestFile, ModuleManifest};

/// 单个已加载 connector 的运行时条目。
///
/// `manifest` 字段用 `ModuleManifest` 平移,方便 `sync_all` 拿 name / sync_interval
/// 等元信息又不需要再回查。
pub struct ConnectorEntry {
    /// connector 名(`manifest.name`)。
    pub name: String,
    /// 原 manifest 完整副本(name/wasm_path/sync_interval 等)。
    pub manifest: ModuleManifest,
    /// 加载好的 wasm connector —— 用 Mutex 因为 wasmtime Store !Sync,
    /// 但允许在 async 上下文里逐个 lock 调 sync。
    connector: Mutex<WasmConnector>,
}

impl ConnectorEntry {
    /// 给单 connector 跑一次 `sync(config_json)`,返 host-side 的 SyncOutcome。
    /// 错误整理成字符串放入返回的 outcome.errors —— 不向上抛(沿用 sync_all 的
    /// 「单 connector 失败不影响其它」策略)。
    pub async fn run_sync(&self, config_json: &str) -> SyncOutcome {
        let mut c = self.connector.lock().await;
        match c.sync(config_json).await {
            Ok(outcome) => outcome,
            Err(e) => SyncOutcome {
                facts: vec![],
                errors: vec![format!("sync failed: {e}")],
                duration_ms: 0,
            },
        }
    }

    /// 给单 connector 跑 health check。失败返 false + 在 log 里记一条。
    pub async fn run_health_check(&self) -> bool {
        let mut c = self.connector.lock().await;
        match c.health_check().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(name = %self.name, error = %e, "health-check failed");
                false
            }
        }
    }
}

/// 多 connector 编排器。
pub struct WasmRuntime {
    /// 模块根目录(`modules/`),用来解析 `manifest.wasm_path` 的相对路径。
    pub modules_root: PathBuf,
    /// 已成功加载的 connector 列表。
    pub entries: Vec<ConnectorEntry>,
    /// 加载过程中失败的模块及错误(module_name → error message)。
    pub load_errors: Vec<(String, String)>,
}

impl WasmRuntime {
    /// 按 manifest 加载所有 `kind == "connector"` 的模块。
    ///
    /// `modules_root` 是 manifest 里 `wasm_path` 的相对 base —— 与
    /// `modules/manifest.toml` 同目录。例:
    /// ```ignore
    /// let runtime = WasmRuntime::from_manifest(
    ///     Path::new("/path/to/repo/modules"),
    ///     &manifest_file,
    /// ).await?;
    /// ```
    ///
    /// 单个模块加载失败 → 记入 `load_errors`,不中断 —— 这样运维拿到的运行时
    /// 至少跑得动其它 connector。如果**所有**模块都失败,返 `WasmRuntime`(空
    /// entries + 满 load_errors),由调用方决定是不是要 fail-fast。
    pub async fn from_manifest(modules_root: &Path, manifest: &ManifestFile) -> Result<Self> {
        let mut entries = Vec::new();
        let mut load_errors = Vec::new();

        for module in &manifest.modules {
            if module.kind != "connector" {
                // Phase 2 只跑 connector,rule/handler 各自有独立 world 待补
                continue;
            }
            let wasm_path = modules_root.join(&module.wasm_path);
            if !wasm_path.exists() {
                load_errors.push((
                    module.name.clone(),
                    format!("wasm file not found: {}", wasm_path.display()),
                ));
                continue;
            }
            // Phase 1 G:把 manifest 申明的 capabilities 传入 WasmConnector,
            // host 端 http_get 按此 allow-list gate(deny by default)。
            // 无 http-client 能力的 connector(hello-world / k8s-mini)传此值
            // 后,guest 即便调 get 也会被 host 拒回 Unauthorized。
            let capabilities: std::collections::HashSet<String> =
                module.capabilities.iter().cloned().collect();
            match WasmConnector::load(&wasm_path, capabilities).await {
                Ok(c) => entries.push(ConnectorEntry {
                    name: module.name.clone(),
                    manifest: module.clone(),
                    connector: Mutex::new(c),
                }),
                Err(e) => {
                    load_errors.push((module.name.clone(), format!("load failed: {e}")));
                }
            }
        }

        Ok(Self {
            modules_root: modules_root.to_path_buf(),
            entries,
            load_errors,
        })
    }

    /// 不加载任何 connector 的空 runtime。
    ///
    /// 用于宿主启动时 manifest 缺失 / 解析失败的 fallback —— 让 UI 起得来,
    /// `connector_count() == 0` 时前端引导用户跑 build。Tauri F path 用。
    pub fn empty(modules_root: impl Into<PathBuf>) -> Self {
        Self {
            modules_root: modules_root.into(),
            entries: Vec::new(),
            load_errors: Vec::new(),
        }
    }

    /// 已加载 connector 数。
    pub fn connector_count(&self) -> usize {
        self.entries.len()
    }

    /// 列出 connector 名(按 manifest 顺序)。
    pub fn connector_names(&self) -> Vec<&str> {
        self.entries.iter().map(|e| e.name.as_str()).collect()
    }

    /// 顺序跑所有 connector 的 sync,聚合 Fact 进单一 [`FactBatch`]。
    ///
    /// `config_json` 是 connector 共享的 sync 入参(Phase 3 起改成 per-connector
    /// 从 manifest 读 `[modules.config]` 段)。返回:
    ///
    /// - `batch` —— 全部 connector 产出的 [`engine_core::Fact`] 聚合后的批,可直接
    ///   `.to_record_batch()` 走 Arrow
    /// - `summary` —— 每个 connector 的 sync 摘要(facts 数 / errors / 耗时)
    pub async fn sync_all(&self, config_json: &str) -> SyncSummary {
        let mut batch = FactBatch::new();
        let mut per_connector = Vec::with_capacity(self.entries.len());
        let mut total_errors: u64 = 0;
        let mut total_duration_ms: u64 = 0;

        for entry in &self.entries {
            let outcome = entry.run_sync(config_json).await;
            total_duration_ms = total_duration_ms.saturating_add(outcome.duration_ms);
            total_errors = total_errors.saturating_add(outcome.errors.len() as u64);

            let fact_count = outcome.facts.len();
            let errors = outcome.errors.clone();
            let canonical = outcome.into_canonical_facts();
            batch.extend(canonical);

            per_connector.push(ConnectorSyncStatus {
                name: entry.name.clone(),
                fact_count,
                errors,
            });
        }

        SyncSummary {
            batch,
            per_connector,
            total_errors,
            total_duration_ms,
        }
    }

    /// **骨架**周期 sync —— 当前简化为「每 `interval_seconds` 跑一次全量 sync_all」。
    ///
    /// Phase 3 起改成 per-connector 调度(每个 connector 按自己的
    /// `sync_interval_seconds` 走独立 `tokio::time::interval`),并支持
    /// 取消 / 优雅退出。本期只暴露入口形状,engine-cli 暂不开启长时间循环
    /// (`tick --once` 仅调 `sync_all` 一次)。
    pub async fn tick_loop(&self, interval_seconds: u64) -> Result<()> {
        if self.entries.is_empty() {
            return Err(anyhow!("no connectors loaded — refusing to start tick loop"));
        }
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_seconds));
        loop {
            ticker.tick().await;
            let summary = self.sync_all("{}").await;
            tracing::info!(
                connectors = self.entries.len(),
                facts = summary.batch.len(),
                errors = summary.total_errors,
                "tick: sync_all"
            );
        }
    }
}

/// 单 connector 在一次 `sync_all` 的状态。
#[derive(Debug, Clone)]
pub struct ConnectorSyncStatus {
    /// connector 名。
    pub name: String,
    /// 本次产出的 fact 数。
    pub fact_count: usize,
    /// 本次的 non-fatal 错误(guest 端 errors 列表)。
    pub errors: Vec<String>,
}

/// `sync_all` 的聚合返回。
#[derive(Debug, Clone)]
pub struct SyncSummary {
    /// 全部 connector 的 fact 聚合后的批(可直接 `.to_record_batch()`)。
    pub batch: FactBatch,
    /// 每个 connector 的明细。
    pub per_connector: Vec<ConnectorSyncStatus>,
    /// 全 connector 总 non-fatal 错误数。
    pub total_errors: u64,
    /// 全 connector 总耗时(毫秒,guest 自报)。
    pub total_duration_ms: u64,
}
