//! engine-wasm
//!
//! wasmtime runtime + capability injection — host side of WASM connector / rule
//! / handler 加载。Phase 2 起接入真实 wasmtime Component Model 加载。

#![deny(unsafe_code)]
#![warn(missing_docs)]

/// Phase 2 runtime —— 真实 wasmtime Component 加载 + capability 注入。
pub mod runtime;

/// 多 connector 编排 —— `WasmRuntime` 持多个 `WasmConnector`,
/// `sync_all` 一次跑完,`tick_loop` 周期跑。
pub mod multi;

pub use multi::{ConnectorEntry, SyncSummary, WasmRuntime};
pub use runtime::{HostFact, SyncOutcome, WasmConnector};

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
    /// 二进制 sha256(可选)。Phase 2 起对 enabled 模块强制校验,
    /// Phase 1 留空即可。
    #[serde(default)]
    pub sha256: String,
}

/// `sync_interval_seconds` 缺省值 —— 30s,与 PRD-004 connector 现网默认一致。
pub fn default_sync_interval_seconds() -> u64 {
    30
}

/// 整张 modules/manifest.toml 反序列化结果。
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ManifestFile {
    /// 当前 schema 版本号(字符串,SemVer-like)。
    pub schema_version: String,
    /// 全部模块列表。
    #[serde(default, rename = "modules")]
    pub modules: Vec<ModuleManifest>,
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
            sha256: String::new(),
        };
        let s = serde_json::to_string(&m).unwrap();
        let back: ModuleManifest = serde_json::from_str(&s).unwrap();
        assert_eq!(back.name, "hello-world");
        assert_eq!(back.wasi_version, WasiVersion::P2);
        assert_eq!(back.sync_interval_seconds, 30);
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
