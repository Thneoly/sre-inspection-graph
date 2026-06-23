//! Integration: 真实读取 modules/manifest.toml 校验解析正确性。
//!
//! 通过环境变量 `MODULES_MANIFEST_PATH` 让测试在不同 working dir 都跑得起来,
//! 缺省按 `../../modules/manifest.toml` 找(从 engine/crates/engine-wasm/ 出发)。

use std::fs;
use std::path::PathBuf;

fn manifest_path() -> PathBuf {
    if let Ok(p) = std::env::var("MODULES_MANIFEST_PATH") {
        return PathBuf::from(p);
    }
    let crate_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(crate_dir).join("../../../modules/manifest.toml")
}

#[test]
fn reads_real_modules_manifest() {
    let path = manifest_path();
    if !path.exists() {
        // 允许 CI 在不同布局下跳过(例如纯 engine/ 测试拷贝)
        eprintln!("skipping: {:?} not found", path);
        return;
    }
    let text = fs::read_to_string(&path).expect("read manifest");
    let parsed = engine_wasm::ManifestFile::from_toml_str(&text).expect("parse");
    assert_eq!(parsed.schema_version, "1");
    let names: Vec<&str> = parsed.modules.iter().map(|m| m.name.as_str()).collect();
    assert!(
        names.contains(&"hello-world"),
        "expected hello-world in manifest, got {:?}",
        names
    );
    // 所有模块默认或显式声明 wasi_version,Phase 1 全 P2
    for m in &parsed.modules {
        assert_eq!(
            m.wasi_version,
            engine_wasm::WasiVersion::P2,
            "module {} should declare wasi_version=p2 in Phase 1",
            m.name
        );
    }
}
