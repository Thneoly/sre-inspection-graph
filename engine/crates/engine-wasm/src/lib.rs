//! engine-wasm
//!
//! wasmtime runtime + capability injection — host side of WASM connector / rule
//! / handler 加载。Phase 1 占位;wasmtime 依赖延迟到 Step 4 第一个真实 connector
//! 接入时引入(可显著加快 cargo check)。

#![deny(unsafe_code)]
#![warn(missing_docs)]

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
            wasm_path: "connectors/hello-world/target/wasm32-wasip2/release/hello_world.wasm"
                .into(),
            version: "0.1.0".into(),
            capabilities: vec![],
        };
        let s = serde_json::to_string(&m).unwrap();
        let back: ModuleManifest = serde_json::from_str(&s).unwrap();
        assert_eq!(back.name, "hello-world");
    }
}
