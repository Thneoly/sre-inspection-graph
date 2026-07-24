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
//! (wasmtime 46 严格校验)。**Phase 1 G** 起 [`http_host::http_get`] 真实装
//! (reqwest GET + capability allow-list),本文件 `HttpClientHost for State`
//! 是它到 WIT 类型的薄适配。host 类型 ↔ binding 类型的字段平移在这里。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

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

// Phase 3.9a-3b2a - handler-world bindgen(与 connector-world 共用 specs/wit)
mod handler_bindings {
    wasmtime::component::bindgen!({
        world: "handler-world",
        path: "../../../specs/wit",
        imports: { default: async },
        exports: { default: async },
    });
}

use bindings::sre::inspection::clock::Host as ClockHost;
use bindings::sre::inspection::fs_read::{
    Entry as FsEntry, Error as FsError, Host as FsReadHost,
};
use bindings::sre::inspection::http_client::{
    Error as HttpError, Host as HttpClientHost, Response as HttpResponse, WriteRequest,
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
///
/// Phase 1 G 起额外持:
/// - `http_client` —— 共享的 reqwest Client(`Arc<Inner>` 内部,clone 廉价),
///   `HttpClientHost::get` 用它发 GET。由 [`WasmConnector::load_with_http`]
///   注入,便于测试换成预配 timeout 的 Client
/// - `allowed_capabilities` —— 该 connector 在 manifest 申明的 capability
///   allow-list。`http_get` 每次调用查 `"http-client"` 是否在内,deny by default
pub struct State {
    table: ResourceTable,
    wasi: WasiCtx,
    /// `http-client` capability 用的 reqwest Client(共享,clone 廉价)。
    http_client: reqwest::Client,
    /// 该 connector 申明的 capability 集合(`manifest.capabilities`)。
    allowed_capabilities: HashSet<String>,
    /// `fs-read` capability 允许访问的根目录(canonicalize 后)。
    /// 由 [`WasmConnector::load_with_fs_roots`] 注入(Phase 8.1);
    /// 非 fs-read connector 传空 Vec。`fs_host::read_file` 每次 read 校验
    /// 请求路径 canonicalize 后落在某根下(防目录穿越 / 符号链接逃逸)。
    fs_roots: Vec<PathBuf>,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

/// Phase 1 G 默认 reqwest Client —— 30s timeout,带 rustls TLS(对齐
/// workspace `reqwest` features)。`WasmConnector::load` 旧签名走此默认;
/// 测试 / WasmConnector::load_with_http 可注入自定义 Client。
///
/// Phase 3 可提到 `WasmRuntime` 级共享一个 Client(见 http_host.rs §4 注释)。
fn default_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("reqwest::Client::build with only timeout cannot fail")
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
    /// Phase 1 G —— 委托 [`crate::http_host::http_get`](纯函数 + capability
    /// allow-list + reqwest GET),再把 host 侧 `HostHttpResponse`/`HostHttpError`
    /// 平移到 WIT binding 的 `Response`/`Error`。
    ///
    /// host 实装与 WIT 类型刻意解耦(见 http_host.rs 顶部说明):这里只做字段
    /// 平移,真正的 capability 检查 + 网络调用都在 `http_host`。
    async fn get(
        &mut self,
        url: String,
        headers: Vec<(String, String)>,
    ) -> std::result::Result<HttpResponse, HttpError> {
        let resp = crate::http_host::http_get(
            &self.http_client,
            &self.allowed_capabilities,
            &url,
            &headers,
        )
        .await
        .map_err(map_host_err_to_wit)?;
        Ok(HttpResponse {
            status: resp.status,
            body: resp.body,
        })
    }

    /// Phase 3.9 -- 委托 [`crate::http_host::http_write`](cap `http-write` gate
    /// + reqwest PATCH/POST/DELETE),WIT `write-request` -> host 类型平移。
    async fn write(
        &mut self,
        req: WriteRequest,
    ) -> std::result::Result<HttpResponse, HttpError> {
        let resp = crate::http_host::http_write(
            &self.http_client,
            &self.allowed_capabilities,
            &req.method,
            &req.url,
            &req.headers,
            req.body.as_deref(),
        )
        .await
        .map_err(map_host_err_to_wit)?;
        Ok(HttpResponse {
            status: resp.status,
            body: resp.body,
        })
    }
}

impl FsReadHost for State {
    /// Phase 8.1 —— 委托 [`crate::fs_host::read_file`](capability `fs-read` gate +
    /// path-root allow-list + canonicalize 防穿越),再把 host 侧 `HostFsEntry`/
    /// `HostFsError` 平移到 WIT `entry` / `error`。真正的安全检查都在 `fs_host`。
    async fn read_file(&mut self, path: String) -> std::result::Result<FsEntry, FsError> {
        let entry = crate::fs_host::read_file(&self.allowed_capabilities, &self.fs_roots, &path)
            .map_err(map_fs_err_to_wit)?;
        Ok(FsEntry {
            path: entry.path,
            content: entry.content,
        })
    }

    /// 同上,委托 [`crate::fs_host::read_dir`](同款 gate + allow-list)。
    async fn read_dir(&mut self, path: String) -> std::result::Result<Vec<String>, FsError> {
        crate::fs_host::read_dir(&self.allowed_capabilities, &self.fs_roots, &path)
            .map_err(map_fs_err_to_wit)
    }
}

/// host 侧 [`crate::fs_host::HostFsError`] → WIT binding `FsError` 一一映射。
fn map_fs_err_to_wit(e: crate::fs_host::HostFsError) -> FsError {
    use crate::fs_host::HostFsError;
    match e {
        HostFsError::NotFound => FsError::NotFound,
        HostFsError::PermissionDenied(m) => FsError::PermissionDenied(m),
        HostFsError::Io(m) => FsError::Io(m),
    }
}

// host 侧 [`http_host::HostHttpError`] → WIT binding `HttpError` 的一一映射。
//
// `Unauthorized` / `NotFound` / `Timeout` 无负载,直接对位;`Network(String)`
// 透出底层错误字符串,guest 自己决定怎么处理。
// ============================================================================
// State impl handler_bindings Host traits(Phase 3.9a-3b2a)
// ============================================================================
// wasmtime bindgen 每 world 生成独立 Host trait,即使 WIT interface 相同。
// handler-world 的 logging/clock/http_client 与 connector-world 相同 interface,
// 但 trait 类型不同。State 需 impl 两套(委托同一 http_host 纯函数)。

impl handler_bindings::sre::inspection::logging::Host for State {
    async fn log(
        &mut self,
        level: handler_bindings::sre::inspection::logging::Level,
        message: String,
    ) {
        match level {
            handler_bindings::sre::inspection::logging::Level::Debug => {
                tracing::debug!(target: "wasm-guest", "{}", message)
            }
            handler_bindings::sre::inspection::logging::Level::Info => {
                tracing::info!(target: "wasm-guest", "{}", message)
            }
            handler_bindings::sre::inspection::logging::Level::Warn => {
                tracing::warn!(target: "wasm-guest", "{}", message)
            }
            handler_bindings::sre::inspection::logging::Level::Error => {
                tracing::error!(target: "wasm-guest", "{}", message)
            }
        }
    }
}

impl handler_bindings::sre::inspection::clock::Host for State {
    async fn now_seconds(&mut self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

impl handler_bindings::sre::inspection::http_client::Host for State {
    async fn get(
        &mut self,
        url: String,
        headers: Vec<(String, String)>,
    ) -> std::result::Result<
        handler_bindings::sre::inspection::http_client::Response,
        handler_bindings::sre::inspection::http_client::Error,
    > {
        let resp = crate::http_host::http_get(
            &self.http_client,
            &self.allowed_capabilities,
            &url,
            &headers,
        )
        .await
        .map_err(map_host_err_to_handler)?;
        Ok(handler_bindings::sre::inspection::http_client::Response {
            status: resp.status,
            body: resp.body,
        })
    }

    async fn write(
        &mut self,
        req: handler_bindings::sre::inspection::http_client::WriteRequest,
    ) -> std::result::Result<
        handler_bindings::sre::inspection::http_client::Response,
        handler_bindings::sre::inspection::http_client::Error,
    > {
        let resp = crate::http_host::http_write(
            &self.http_client,
            &self.allowed_capabilities,
            &req.method,
            &req.url,
            &req.headers,
            req.body.as_deref(),
        )
        .await
        .map_err(map_host_err_to_handler)?;
        Ok(handler_bindings::sre::inspection::http_client::Response {
            status: resp.status,
            body: resp.body,
        })
    }
}

fn map_host_err_to_handler(
    e: crate::http_host::HostHttpError,
) -> handler_bindings::sre::inspection::http_client::Error {
    use crate::http_host::HostHttpError;
    match e {
        HostHttpError::Unauthorized(_) => {
            handler_bindings::sre::inspection::http_client::Error::Unauthorized
        }
        HostHttpError::NotFound => handler_bindings::sre::inspection::http_client::Error::NotFound,
        HostHttpError::Timeout => handler_bindings::sre::inspection::http_client::Error::Timeout,
        HostHttpError::Network(m) => {
            handler_bindings::sre::inspection::http_client::Error::Network(m)
        }
    }
}

fn map_host_err_to_wit(e: crate::http_host::HostHttpError) -> HttpError {
    use crate::http_host::HostHttpError;
    match e {
        HostHttpError::Unauthorized(_) => HttpError::Unauthorized,
        HostHttpError::NotFound => HttpError::NotFound,
        HostHttpError::Timeout => HttpError::Timeout,
        HostHttpError::Network(msg) => HttpError::Network(msg),
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
    /// 4. Store 装 State(WasiCtx + ResourceTable + http_client + capabilities)
    /// 5. Component::from_file 读 .wasm
    /// 6. ConnectorWorld::instantiate_async 把 instance bind 到强类型 bindings
    ///
    /// `capabilities` 是该 connector 在 manifest 申明的 allow-list(`logging` /
    /// `clock` / `http-client` ...),`http_get` 调用时按此 gate。无 capability
    /// 的 connector(如 hello-world)传空集合即可。
    pub async fn load(wasm_path: &Path, capabilities: HashSet<String>) -> Result<Self> {
        Self::load_with_http(wasm_path, capabilities, default_http_client()).await
    }

    /// 与 [`load`] 同,但允许注入自定义 reqwest `client`。
    ///
    /// **用途**:
    /// - 测试:注入预配短 timeout 的 Client,避免单测因 30s 默认超时变慢
    /// - Phase 3:多个 connector 共享一个 WasmRuntime 级 Client(连接池复用)
    ///
    /// 注:`capabilities` 字面平移进 `State`,host 实装不复制;`client` 是
    /// `Arc<Inner>` clone 廉价(reqwest 文档保证)。
    pub async fn load_with_http(
        wasm_path: &Path,
        capabilities: HashSet<String>,
        client: reqwest::Client,
    ) -> Result<Self> {
        Self::load_with_fs_roots(wasm_path, capabilities, client, Vec::new()).await
    }

    /// 与 [`load`] 同,但用默认 reqwest Client + 注入 `fs_roots`(fs-read grant)。
    ///
    /// 供 [`crate::multi::WasmRuntime::from_manifest`] 给申明 `fs-read` 的 connector
    /// 传根目录。`fs_roots` 为空时即便 capabilities 含 `fs-read` 也无访问
    /// (有 cap 无根 = 无访问,见 [`crate::fs_host`])。
    pub async fn load_with_roots(
        wasm_path: &Path,
        capabilities: HashSet<String>,
        fs_roots: Vec<PathBuf>,
    ) -> Result<Self> {
        Self::load_with_fs_roots(wasm_path, capabilities, default_http_client(), fs_roots).await
    }

    /// 真实加载实装:Config / Engine / Linker(WASI p2 + connector-world)/ Store
    /// (含 `fs_roots`)/ Component / instantiate。
    ///
    /// 注:`capabilities` 字面平移进 `State`,host 实装不复制;`client` 是
    /// `Arc<Inner>` clone 廉价(reqwest 文档保证);`fs_roots` 应已 canonicalize。
    pub async fn load_with_fs_roots(
        wasm_path: &Path,
        capabilities: HashSet<String>,
        client: reqwest::Client,
        fs_roots: Vec<PathBuf>,
    ) -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        // wasmtime 46:async_support 已默认开,不要显式调(deprecated)。
        let engine = Engine::new(&config).map_err(wasm_err)?;

        let mut linker = Linker::<State>::new(&engine);
        // 接全套 WASI p2 imports(io/streams/cli/clocks/sockets/...)。
        // hello-world 的 std 库会用到 wasi:io / wasi:cli 子集。
        wasmtime_wasi::p2::add_to_linker_async(&mut linker).map_err(wasm_err)?;
        // 接我们的 connector-world capability(logging + clock + http-client + fs-read)。
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
                http_client: client,
                allowed_capabilities: capabilities,
                fs_roots,
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

// host 端组装好的 sync 结果。
// ============================================================================
// WasmHandler - handler-world 的 host-side 句柄(Phase 3.9a-3b2a)
// ============================================================================

/// 单个 WASM handler 实例的 host-side 句柄(Phase 3.9a-3b2a)。
///
/// 与 [`WasmConnector`] 同构:各自的 bindgen world(connector-world vs handler-world)+
/// `Store<State>` + bindings。handler-world export `handler{dry-run/execute/verify}`,
/// host 经 [`WasmHandler::execute`] 调 `call_execute`。
///
/// `State` 复用 [`WasmConnector`] 的(LoggingHost/ClockHost/HttpClientHost 含 write),
/// handler-world import logging/clock/http-client 与 connector-world 相同。
pub struct WasmHandler {
    store: Store<State>,
    bindings: handler_bindings::HandlerWorld,
}

impl WasmHandler {
    /// 加载 handler-world .wasm Component 并实例化。
    ///
    /// 与 [`WasmConnector::load_with_http`] 同,但 `HandlerWorld::add_to_linker`
    /// (handler-world import logging/clock/http-client,与 connector-world 共用 State impl)。
    pub async fn load_with_http(
        wasm_path: &Path,
        capabilities: HashSet<String>,
        client: reqwest::Client,
    ) -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        let engine = Engine::new(&config).map_err(wasm_err)?;

        let mut linker = Linker::<State>::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker).map_err(wasm_err)?;
        handler_bindings::HandlerWorld::add_to_linker::<State, wasmtime::component::HasSelf<State>>(
            &mut linker,
            |s| s,
        )
        .map_err(wasm_err)?;

        let mut store = Store::new(
            &engine,
            State {
                table: ResourceTable::new(),
                wasi: WasiCtxBuilder::new().inherit_stdio().build(),
                http_client: client,
                allowed_capabilities: capabilities,
                fs_roots: Vec::new(),
            },
        );

        let component = Component::from_file(&engine, wasm_path)
            .map_err(|e| anyhow!("load handler wasm component from {}: {e}", wasm_path.display()))?;

        let bindings = handler_bindings::HandlerWorld::instantiate_async(&mut store, &component, &linker)
            .await
            .map_err(wasm_err)?;

        Ok(Self { store, bindings })
    }

    /// 便捷加载(默认 reqwest Client)。
    pub async fn load(wasm_path: &Path, capabilities: HashSet<String>) -> Result<Self> {
        Self::load_with_http(wasm_path, capabilities, default_http_client()).await
    }

    /// 调 guest handler `execute(ctx)` -> 返回 [`ExecResult`] 或 [`ExecError`]。
    ///
    /// 双层 Result:外层 wasmtime 调用成败(trap 等);内层 WIT `execution-error`
    /// (precondition-failed / capability-denied / upstream-api / timeout)。
    ///
    /// 高层 API:接 &str(内部构造 WIT ExecutionContext),返 host 侧简单类型
    /// (避免暴露 handler_bindings WIT 类型)。
    pub async fn execute(
        &mut self,
        action_id: &str,
        target_resource_id: &str,
        params_json: &str,
        initiated_by: &str,
    ) -> Result<Result<ExecResult, ExecError>, anyhow::Error> {
        let ctx = handler_bindings::exports::sre::inspection::handler::ExecutionContext {
            action_id: action_id.to_string(),
            target_resource_id: target_resource_id.to_string(),
            params_json: params_json.to_string(),
            initiated_by: initiated_by.to_string(),
        };
        let raw = self
            .bindings
            .sre_inspection_handler()
            .call_execute(&mut self.store, &ctx)
            .await
            .map_err(wasm_err)?;
        Ok(raw
            .map(|r| ExecResult {
                success: r.success,
                message: r.message,
                attributes_json: r.attributes_json,
            })
            .map_err(|e| ExecError(format!("{e:?}"))))
    }
}

/// handler 执行结果(host 侧简单类型,避免暴露 handler_bindings WIT 类型)。
#[derive(Debug, Clone)]
pub struct ExecResult {
    /// 是否成功。
    pub success: bool,
    /// 人读消息。
    pub message: String,
    /// 动作生效后的新 attrs JSON 字符串。
    pub attributes_json: String,
}

/// handler 执行错误(host 侧简单类型)。
#[derive(Debug, Clone)]
pub struct ExecError(
    /// 错误描述(WIT execution-error variant 的 Debug)。
    pub String,
);

#[derive(Debug, Clone)]
/// host 端组装好的 sync 结果。
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
