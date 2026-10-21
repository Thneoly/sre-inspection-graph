//! engine-wasm
//!
//! wasmtime runtime + capability injection — host side of WASM connector / rule
//! / handler 加载。Phase 2 起接入真实 wasmtime Component Model 加载。

#![deny(unsafe_code)]
#![warn(missing_docs)]

/// Phase 2 runtime —— 真实 wasmtime Component 加载 + capability 注入。
pub mod runtime;

/// Phase 1 G —— `http-client` capability 的 host 端纯函数实装
/// (`http_get` + `HostHttpResponse`/`HostHttpError`)。刻意与 WIT binding
/// 类型解耦,`runtime.rs::HttpClientHost for State` 是它到 WIT 的薄适配。
pub mod http_host;

/// Phase 8.1 —— `fs-read` capability 的 host 端纯函数实装
/// (`read_file`/`read_dir` + `HostFsEntry`/`HostFsError` + path-root allow-list)。
/// 与 `http_host` 同构,`runtime.rs::FsReadHost for State` 是它到 WIT 的薄适配。
pub mod fs_host;

/// 多 connector 编排 —— `WasmRuntime` 持多个 `WasmConnector`,
/// `sync_all` 一次跑完,`tick_loop` 周期跑。
pub mod multi;

/// Phase 3.9a-3b2 -- `WasmHandlerExecutor`(impl `HandlerExecutor`,调 `WasmHandler`
/// 真改集群)。6 k8s action 走 WasmHandler,kill_query/clear_cache 留 Mock。
pub mod handler_executor;

pub use handler_executor::WasmHandlerExecutor;
pub use multi::{
    select_config, ConnectorEntry, ConnectorSyncStatus, HandlerEntry, SyncSummary, WasmRuntime,
};
pub use runtime::{ExecError, ExecResult, HostFact, SyncOutcome, WasmConnector, WasmHandler};

/// 模块声明使用的 WASI ABI 版本。
///
/// - `P2`(默认)→ `wasm32-wasip2` rustc target,Tier 2 stable,2026-06 时
///   能 `rustup target add` 直接装,wasmtime 46 完全兼容
/// - `P3` → `wasm32-wasip3` rustc target,**Tier 3**(2026-06 仍未上 stable),
///   需要 nightly + `-Z build-std` + wasi-sdk 22+;wasmtime 46 host 端已默认
///   支持 WASI 0.3.0 + `component-model-async`,等 rustc 把 wasip3 提到 Tier 2
///   后 modules 切此值即可
///
/// 详见 `doc/16 §3 WASI ABI 演进策略`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WasiVersion {
    /// WASIp2(default) — Tier 2 stable rustc target,生产路径。
    #[default]
    P2,
    /// WASIp3 — Tier 3,需要 nightly,带 async/futures-native 支持。
    P3,
}

impl WasiVersion {
    /// Rustc target triple,例如 `wasm32-wasip2`。
    pub fn target_triple(&self) -> &'static str {
        match self {
            Self::P2 => "wasm32-wasip2",
            Self::P3 => "wasm32-wasip3",
        }
    }

    /// 当前是否依赖 nightly toolchain。
    pub fn requires_nightly(&self) -> bool {
        matches!(self, Self::P3)
    }
}

/// 单个 WASM 模块的元信息(从 `modules/manifest.toml` 读)。
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ModuleManifest {
    /// 模块名(用作注册键)。
    pub name: String,
    /// 模块类型:connector / rule / handler。
    #[serde(rename = "type")]
    pub kind: String,
    /// 编译产物路径(相对 modules/)。
    pub wasm_path: String,
    /// SemVer 版本号。
    pub version: String,
    /// 申明的能力,引擎按此注入 capability。
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// 模块所用的 WASI ABI 版本(默认 P2)。
    #[serde(default)]
    pub wasi_version: WasiVersion,
    /// connector 周期同步间隔(秒)。
    ///
    /// `WasmRuntime::tick_loop` 用此值跑 `tokio::time::interval`。0 / 缺省 →
    /// 走 [`default_sync_interval_seconds`]。
    #[serde(default = "default_sync_interval_seconds")]
    pub sync_interval_seconds: u64,
    /// 该模块是否启用加载。
    ///
    /// 缺省 `true`。设 `false` -> [`WasmRuntime::from_manifest`] 跳过该模块(不加载、
    /// 不进 load_errors)。用于在不删 manifest 条目的前提下禁用 scaffolding connector
    /// (如 hello-world / k8s-mini),让真集群阶段只跑 k8s / prometheus。
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// 该 connector 的 per-connector sync config(TOML inline table -> JSON object)。
    ///
    /// 缺省 `None` -> `sync_all` 回退到调用方传入的全局 broadcast config(向后兼容)。
    /// 设值后 [`WasmRuntime::sync_all`] 走 [`select_config`] 把它序列化成 JSON 字符串
    /// 单独喂给该 connector,而不是一刀切广播。这样 k8s connector 拿 `{api_base,...}`,
    /// prometheus 拿 `{prometheus_url,...}`,互不串扰。
    #[serde(default)]
    pub config: Option<serde_json::Value>,
    /// fs-read capability 允许扫描的根目录(Phase 8.1,security grant)。
    ///
    /// 缺省 `None` → 无 fs-read 权限。code-repo connector 申明 `fs-read` capability
    /// 时在此列出允许扫描的绝对路径;[`WasmRuntime::from_manifest`] canonicalize
    /// 后传入 [`WasmConnector`],host 每次 read 时校验请求路径 canonicalize 后落在
    /// 某根下(防目录穿越 `../../etc/passwd` + 符号链接逃逸)。非 fs-read connector
    /// 忽略此字段。详见 [`crate::fs_host`]。
    #[serde(default)]
    pub fs_roots: Option<Vec<String>>,
    /// 二进制 sha256(可选)。Phase 2 起对 enabled 模块强制校验,
    /// Phase 1 留空即可。
    #[serde(default)]
    pub sha256: String,
}

/// `enabled` 缺省值 -- `true`(不写就是启用)。
pub fn default_enabled() -> bool {
    true
}

/// `sync_interval_seconds` 缺省值 —— 30s,与 PRD-004 connector 现网默认一致。
pub fn default_sync_interval_seconds() -> u64 {
    30
}

/// manifest 顶层 `[defaults]` 段 —— 跨 connector 共享的缺省 config 键。
///
/// 换集群 / namespace 过去要手改 5 处内联 config(cluster/namespace/api_base
/// 在每个 module 里重复),现在 `[defaults]` 一处声明、各 module config 未写的
/// 键从这里补(module config 显式键仍优先)。注意 jaeger/flagd 的完整 URL 里
/// 内嵌的 namespace 路径无法经此注入,换 namespace 仍须手改那两个 URL。
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ManifestDefaults {
    /// 缺省 cluster 标识(展示/命名用)。
    #[serde(default)]
    pub cluster: Option<String>,
    /// 缺省 namespace。
    #[serde(default)]
    pub namespace: Option<String>,
    /// 缺省 K8s API base(kubectl proxy)。
    #[serde(default)]
    pub api_base: Option<String>,
}

impl ManifestDefaults {
    /// 展开成 JSON object(None 字段省略)。
    pub fn to_json_object(&self) -> serde_json::Value {
        let mut m = serde_json::Map::new();
        if let Some(v) = &self.cluster {
            m.insert("cluster".into(), serde_json::Value::String(v.clone()));
        }
        if let Some(v) = &self.namespace {
            m.insert("namespace".into(), serde_json::Value::String(v.clone()));
        }
        if let Some(v) = &self.api_base {
            m.insert("api_base".into(), serde_json::Value::String(v.clone()));
        }
        serde_json::Value::Object(m)
    }
}

/// 把 `[defaults]` 键 overlay 到 module config 下(module config 显式键优先)。
/// 返回合成后的 config object;两者皆空则返空 object(调用方自行回退 global)。
pub fn merge_defaults(
    manifest_config: Option<&serde_json::Value>,
    defaults: &ManifestDefaults,
) -> serde_json::Value {
    let mut base = defaults.to_json_object();
    if let (Some(b), Some(over)) = (base.as_object_mut(), manifest_config.and_then(|c| c.as_object()))
    {
        for (k, v) in over {
            b.insert(k.clone(), v.clone());
        }
    }
    base
}

/// manifest 顶层 `[network]` 段 —— http capability 的 host allow-list(Phase 3)。
///
/// 与 per-module `capabilities` 互补:capability 决定「能不能发 HTTP」,
/// `[network].allowed_hosts` 决定「能发到哪」。所有持 `http-client`/`http-write`
/// 的模块共享同一张表(全局并集语义);比 per-module 表粗,但把「任意 URL」
/// 收敛到「声明过的 host」,且 manifest 只需维护一处。
///
/// 缺省(不写 `[network]` 段)= [`crate::http_host::AllowedHosts::NotConfigured`]:
/// allow-all + 启动 warn —— 不 break 现有 manifest,生产/分发前应 pin。
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct NetworkConfig {
    /// 允许访问的 host 列表。条目 `host` 或 `host:port`(大小写不敏感);
    /// 纯 host 条目匹配任意端口。`Some([])` = 显式全拒。
    /// 无论怎么配,link-local 目标(169.254.0.0/16、fe80::/10)始终硬拒。
    #[serde(default)]
    pub allowed_hosts: Option<Vec<String>>,
}

/// 整张 modules/manifest.toml 反序列化结果。
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ManifestFile {
    /// 当前 schema 版本号(字符串,SemVer-like)。
    pub schema_version: String,
    /// 全部模块列表。
    #[serde(default, rename = "modules")]
    pub modules: Vec<ModuleManifest>,
    /// 顶层 `[defaults]` 段(跨 connector 缺省 config 键)。
    #[serde(default)]
    pub defaults: ManifestDefaults,
    /// 顶层 `[network]` 段(http host allow-list)。缺省 NotConfigured(allow-all + warn)。
    #[serde(default)]
    pub network: NetworkConfig,
}

impl ManifestFile {
    /// 解析 toml 字符串。失败返回 `toml::de::Error`。
    pub fn from_toml_str(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }
}

/// Crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_round_trips() {
        let m = ModuleManifest {
            name: "hello-world".into(),
            kind: "connector".into(),
            wasm_path: "target/wasm32-wasip2/release/hello_world.wasm".into(),
            version: "0.1.0".into(),
            capabilities: vec![],
            wasi_version: WasiVersion::P2,
            sync_interval_seconds: 30,
            enabled: true,
            config: None,
            fs_roots: None,
            sha256: String::new(),
        };
        let s = serde_json::to_string(&m).unwrap();
        let back: ModuleManifest = serde_json::from_str(&s).unwrap();
        assert_eq!(back.name, "hello-world");
        assert_eq!(back.wasi_version, WasiVersion::P2);
        assert_eq!(back.sync_interval_seconds, 30);
        assert!(back.enabled);
        assert!(back.config.is_none());
        assert!(back.fs_roots.is_none());
    }

    #[test]
    fn parses_fs_roots() {
        let toml = r#"
schema_version = "1"

[[modules]]
name = "code-repo"
type = "connector"
wasm_path = "x.wasm"
version = "0.1.0"
capabilities = ["logging", "clock", "fs-read"]
fs_roots = ["/tmp/repos", "/home/me/code"]
"#;
        let parsed = ManifestFile::from_toml_str(toml).expect("should parse");
        let m = &parsed.modules[0];
        let roots = m.fs_roots.as_ref().expect("fs_roots present");
        assert_eq!(roots, &vec!["/tmp/repos".to_string(), "/home/me/code".to_string()]);
    }

    #[test]
    fn parses_enabled_false_and_per_connector_config() {
        let toml = r#"
schema_version = "1"

[[modules]]
name = "k8s"
type = "connector"
wasm_path = "x.wasm"
version = "0.1.0"
capabilities = ["http-client"]
enabled = false
config = { api_base = "http://127.0.0.1:8001", cluster = "vm-cluster", namespace = "otel-demo" }
"#;
        let parsed = ManifestFile::from_toml_str(toml).expect("should parse");
        let m = &parsed.modules[0];
        assert!(!m.enabled);
        let cfg = m.config.as_ref().expect("config present");
        assert_eq!(cfg["api_base"], "http://127.0.0.1:8001");
        assert_eq!(cfg["cluster"], "vm-cluster");
        assert_eq!(cfg["namespace"], "otel-demo");
    }

    #[test]
    fn parses_network_section_and_defaults_to_not_configured() {
        // 配了 [network] -> Some(列表)
        let toml = r#"
schema_version = "1"

[network]
allowed_hosts = ["127.0.0.1", "prometheus.internal:9090"]

[[modules]]
name = "k8s"
type = "connector"
wasm_path = "x.wasm"
version = "0.1.0"
capabilities = ["http-client"]
"#;
        let parsed = ManifestFile::from_toml_str(toml).expect("should parse");
        assert_eq!(
            parsed.network.allowed_hosts.as_deref(),
            Some(["127.0.0.1".to_string(), "prometheus.internal:9090".to_string()].as_slice())
        );

        // 没配 -> None(= AllowedHosts::NotConfigured,allow-all + multi.rs warn)
        let bare = r#"
schema_version = "1"

[[modules]]
name = "hello"
type = "connector"
wasm_path = "x.wasm"
version = "0.1.0"
"#;
        let parsed = ManifestFile::from_toml_str(bare).expect("should parse");
        assert!(parsed.network.allowed_hosts.is_none());
    }

    #[test]
    fn select_config_prefers_manifest_over_defaults_and_global() {
        let no_defaults = ManifestDefaults::default();
        let cfg = serde_json::json!({ "api_base": "http://127.0.0.1:8001" });
        // manifest config -> 序列化成 JSON 字符串(defaults 空时行为不变)
        assert_eq!(
            select_config(Some(&cfg), "{}", &no_defaults),
            r#"{"api_base":"http://127.0.0.1:8001"}"#
        );
        // 无 manifest config 无 defaults -> 回退全局 broadcast(向后兼容)
        assert_eq!(
            select_config(None, r#"{"cluster":"demo"}"#, &no_defaults),
            r#"{"cluster":"demo"}"#
        );
    }

    #[test]
    fn select_config_overlays_defaults_under_manifest_keys() {
        // defaults 打底 + module config 显式键覆盖:namespace 冲突以 module 为准,
        // cluster 从 defaults 补
        let defaults = ManifestDefaults {
            cluster: Some("vm-cluster".into()),
            namespace: Some("otel-demo".into()),
            api_base: Some("http://127.0.0.1:8001".into()),
        };
        let cfg = serde_json::json!({ "namespace": "monitoring", "release_prefix": "otel" });
        let out = select_config(Some(&cfg), "{}", &defaults);
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["namespace"], "monitoring", "module key wins");
        assert_eq!(v["cluster"], "vm-cluster", "defaults fill missing key");
        assert_eq!(v["api_base"], "http://127.0.0.1:8001");
        assert_eq!(v["release_prefix"], "otel");
        // 无 module config 时 defaults 即整体 config(global 被取代)
        let out2 = select_config(None, "{}", &defaults);
        let v2: serde_json::Value = serde_json::from_str(&out2).expect("valid json");
        assert_eq!(v2["namespace"], "otel-demo");
    }

    #[test]
    fn parses_modules_manifest_toml() {
        let toml = r#"
schema_version = "1"

[[modules]]
name = "hello-world"
type = "connector"
wasm_path = "x.wasm"
version = "0.1.0"
capabilities = []
"#;
        let parsed = ManifestFile::from_toml_str(toml).expect("should parse");
        assert_eq!(parsed.schema_version, "1");
        assert_eq!(parsed.modules.len(), 1);
        assert_eq!(parsed.modules[0].name, "hello-world");
        assert_eq!(parsed.modules[0].kind, "connector");
        // 缺省字段 → 默认 P2
        assert_eq!(parsed.modules[0].wasi_version, WasiVersion::P2);
    }

    #[test]
    fn parses_p3_module() {
        let toml = r#"
schema_version = "1"

[[modules]]
name = "async-connector"
type = "connector"
wasm_path = "x.wasm"
version = "0.1.0"
capabilities = []
wasi_version = "p3"
"#;
        let parsed = ManifestFile::from_toml_str(toml).expect("should parse");
        assert_eq!(parsed.modules[0].wasi_version, WasiVersion::P3);
        assert_eq!(
            parsed.modules[0].wasi_version.target_triple(),
            "wasm32-wasip3"
        );
        assert!(parsed.modules[0].wasi_version.requires_nightly());
    }

    #[test]
    fn wasi_version_target_triple() {
        assert_eq!(WasiVersion::P2.target_triple(), "wasm32-wasip2");
        assert_eq!(WasiVersion::P3.target_triple(), "wasm32-wasip3");
        assert!(!WasiVersion::P2.requires_nightly());
        assert!(WasiVersion::P3.requires_nightly());
    }
}
