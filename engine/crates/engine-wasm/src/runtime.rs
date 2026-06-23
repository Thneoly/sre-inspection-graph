//! Phase 2 host runtime — 真实 wasmtime Component 加载 + capability 注入。
//!
//! 设计参考:`/home/cc/Desktop/code/ntx/show/ntxdemo/src/wasm_engine/engine.rs`。
//!
//! - `bindgen!` 一次生成 connector-world 的 host trait + 强类型 export
//! - `State` 持 WasiCtx + ResourceTable;`WasiView::ctx` 同时给 ctx 和 table
//! - `logging` / `clock` / `http-client` host trait 各自实现一次
//! - `WasmConnector::load(path).await` 做 Config / Engine / Linker / Store / Component / instantiate
//! - `WasmConnector::sync(json).await` 调真实 guest export 拿回 SyncResult + Facts
//!
//! 关于 http-client:WIT world 声明了 import,host 必须实现,即便 guest 不调
//! (wasmtime 46 严格校验)。Phase 2 给 stub(返 network error),Phase 3 接 reqwest。

use std::path::Path;

use anyhow::{anyhow, Result};
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

// 一次性 bindgen connector-world。
// `imports / exports: { default: async }` 与 ntx/show 的 actions-executor
// 同款,与 wasmtime 46 默认开启的 component-model-async 配套。
mod bindings {
    wasmtime::component::bindgen!({
        world: "connector-world",
        path: "../../../specs/wit",
        imports: { default: async },
        exports: { default: async },
    });
}

use bindings::sre::inspection::clock::Host as ClockHost;
use bindings::sre::inspection::http_client::{
    Error as HttpError, Host as HttpClientHost, Response as HttpResponse,
};
use bindings::sre::inspection::logging::{Host as LoggingHost, Level};
use bindings::ConnectorWorld;

/// wasmtime 46 的 `wasmtime::Error` 不 impl `std::error::Error`(为了寄存器
/// 优化),所以 `anyhow::Context::context` 用不了。统一用此 helper 转 anyhow。
fn wasm_err(e: wasmtime::Error) -> anyhow::Error {
    anyhow!("wasmtime: {e}")
}

/// 单个 WASM connector 实例的 host-side 句柄。
///
/// 拥有自己的 Store(状态在里面 — WasiCtx、资源表、capability 句柄等)。
/// 每个 connector 一个 WasmConnector,host 端按 manifest.toml 实例化 N 个。
pub struct WasmConnector {
    store: Store<State>,
    bindings: ConnectorWorld,
}

/// host 端 Store 内挂载的状态。`WasiView::ctx` 返回 `WasiCtxView` 同时
/// 暴露 WasiCtx + ResourceTable,wasmtime-wasi 46+ 不再要分别的 IoView。
pub struct State {
    table: ResourceTable,
    wasi: WasiCtx,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

// ============================================================================
// Host trait impls — wit-bindgen 生成的 capability traits(wasmtime 46 用
// native async fn,不需要 async_trait 宏)
// ============================================================================

impl LoggingHost for State {
    async fn log(&mut self, level: Level, message: String) {
        match level {
            Level::Debug => tracing::debug!(target: "wasm-guest", "{}", message),
            Level::Info => tracing::info!(target: "wasm-guest", "{}", message),
            Level::Warn => tracing::warn!(target: "wasm-guest", "{}", message),
            Level::Error => tracing::error!(target: "wasm-guest", "{}", message),
        }
    }
}

impl ClockHost for State {
    async fn now_seconds(&mut self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

impl HttpClientHost for State {
    /// Phase 2 stub。Phase 3 接 reqwest + capability allow-list 校验。
    async fn get(
        &mut self,
        url: String,
        _headers: Vec<(String, String)>,
    ) -> std::result::Result<HttpResponse, HttpError> {
        tracing::warn!(url = %url, "http-client not implemented yet (Phase 3)");
        Err(HttpError::Network(
            "http-client not implemented in Phase 2".to_string(),
        ))
    }
}

// ============================================================================
// WasmConnector — 实例生命周期
// ============================================================================

impl WasmConnector {
    /// 加载 .wasm Component 并实例化。
    ///
    /// 步骤:
    /// 1. Config:wasm_component_model + async(wasmtime 46 已默认开 async)
    /// 2. Engine 创建一次,多个 WasmConnector 可共享(但 Store 各自一份)
    /// 3. Linker 接 WASI p2 全套 + 我们的 connector-world host traits
    /// 4. Store 装 State(WasiCtx + ResourceTable)
    /// 5. Component::from_file 读 .wasm
    /// 6. ConnectorWorld::instantiate_async 把 instance bind 到强类型 bindings
    pub async fn load(wasm_path: &Path) -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        // wasmtime 46:async_support 已默认开,不要显式调(deprecated)。
        let engine = Engine::new(&config).map_err(wasm_err)?;

        let mut linker = Linker::<State>::new(&engine);
        // 接全套 WASI p2 imports(io/streams/cli/clocks/sockets/...)。
        // hello-world 的 std 库会用到 wasi:io / wasi:cli 子集。
        wasmtime_wasi::p2::add_to_linker_async(&mut linker).map_err(wasm_err)?;
        // 接我们的 connector-world capability(logging + clock + http-client)。
        // wit-bindgen 把所有 import 整合到一个 add_to_linker 调用 — 加 capability
        // 时只改 connector.wit + 给 State 加 impl,这里不动。
        ConnectorWorld::add_to_linker::<State, wasmtime::component::HasSelf<State>>(
            &mut linker,
            |s| s,
        )
        .map_err(wasm_err)?;

        let mut store = Store::new(
            &engine,
            State {
                table: ResourceTable::new(),
                wasi: WasiCtxBuilder::new().inherit_stdio().build(),
            },
        );

        let component = Component::from_file(&engine, wasm_path)
            .map_err(|e| anyhow!("load wasm component from {}: {e}", wasm_path.display()))?;

        let bindings = ConnectorWorld::instantiate_async(&mut store, &component, &linker)
            .await
            .map_err(wasm_err)?;

        Ok(Self { store, bindings })
    }

    /// 调 guest 的 sync(config_json) → 返回 facts + errors + 耗时。
    ///
    /// 双层 Result:外层是 wasmtime 调用本身的成败(trap、链接错等);内层
    /// 是 guest 的 sync-error variant(config / runtime / timeout / capability-denied)。
    pub async fn sync(&mut self, config_json: &str) -> Result<SyncOutcome> {
        let raw = self
            .bindings
            .sre_inspection_connector()
            .call_sync(&mut self.store, config_json)
            .await
            .map_err(wasm_err)?;

        match raw {
            Ok(sync_result) => Ok(SyncOutcome {
                facts: sync_result
                    .facts
                    .into_iter()
                    .map(HostFact::from_guest)
                    .collect(),
                errors: sync_result.errors,
                duration_ms: sync_result.duration_ms,
            }),
            Err(e) => Err(anyhow!("guest sync returned error: {:?}", e)),
        }
    }

    /// 调 guest 的 health-check — guest 自报当前是否健康。
    pub async fn health_check(&mut self) -> Result<bool> {
        self.bindings
            .sre_inspection_connector()
            .call_health_check(&mut self.store)
            .await
            .map_err(wasm_err)
    }
}

/// host 端组装好的 sync 结果。
#[derive(Debug, Clone)]
pub struct SyncOutcome {
    /// 本轮采集的 fact。
    pub facts: Vec<HostFact>,
    /// guest 报的 non-fatal 错误。
    pub errors: Vec<String>,
    /// guest 端实际耗时。
    pub duration_ms: u64,
}

/// host 端 Fact —— 与 guest WIT 的 `record fact` 字段一一对应。
/// engine-core 后续把它转 Arrow RecordBatch。
#[derive(Debug, Clone)]
pub struct HostFact {
    /// 全局唯一 ID。
    pub id: String,
    /// Fact 类型(topology-node / topology-edge / metric / change / alert)。
    pub kind: String,
    /// 数据源 — 模块名,例如 "k8s-connector"。
    pub source: String,
    /// 资源 ID — 与 reference 的 resource_id 兼容。
    pub resource_id: String,
    /// 资源类型 — L1 14 类型之一。
    pub resource_type: String,
    /// 时间戳(Unix 秒)。
    pub timestamp: u64,
    /// 属性 — JSON 编码字符串。
    pub attributes_json: String,
}

impl HostFact {
    /// 从 wit-bindgen 生成的强类型 Fact 转 host 端 plain struct。
    fn from_guest(guest: bindings::exports::sre::inspection::connector::Fact) -> Self {
        Self {
            id: guest.id,
            kind: guest.kind,
            source: guest.source,
            resource_id: guest.resource_id,
            resource_type: guest.resource_type,
            timestamp: guest.timestamp,
            attributes_json: guest.attributes_json,
        }
    }
}

/// 适配器:`HostFact` → engine-core canonical `Fact`。
///
/// host runtime 拿到 wasmtime 调出来的 `HostFact` 后,工程上不直接消费它 ——
/// engine-storage / engine-cli / Arrow Flight 全用 [`engine_core::Fact`] 这一规范型。
/// 转换是字段平移(完全同构),零分配的复用都已经在 String move 里完成。
impl From<HostFact> for engine_core::Fact {
    fn from(h: HostFact) -> Self {
        engine_core::Fact {
            id: h.id,
            kind: h.kind,
            source: h.source,
            resource_id: h.resource_id,
            resource_type: h.resource_type,
            timestamp: h.timestamp,
            attributes_json: h.attributes_json,
        }
    }
}

impl SyncOutcome {
    /// 消费此 SyncOutcome,把 [`HostFact`] 全转成 canonical [`engine_core::Fact`] 列表。
    ///
    /// 调用方场景:`WasmRuntime::sync_all` 把多 connector 的 Fact 攒进一个
    /// `FactBatch`,这里是单 connector 的批转换入口。
    pub fn into_canonical_facts(self) -> Vec<engine_core::Fact> {
        self.facts.into_iter().map(Into::into).collect()
    }
}
