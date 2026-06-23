//! Integration: 校验 manifest 中声明的 hello_world.wasm 实际存在且是合法
//! WASM 文件(以 `\0asm` magic 开头)。
//!
//! 这个测试默认 **可选** — 找不到产物时跳过,不阻塞 cargo test。CI 中 modules-wasip2
//! job 先 `cargo build --target wasm32-wasip2`,再让 engine workspace 复用该
//! 产物校验路径正确。本地开发也是同模式:先在 modules/ 跑一次 wasip2 build。

use std::fs;
use std::path::PathBuf;

fn modules_root() -> PathBuf {
    let crate_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(crate_dir).join("../../../modules")
}

#[test]
fn hello_world_wasm_is_a_valid_wasm_file() {
    let modules = modules_root();
    let manifest_path = modules.join("manifest.toml");
    if !manifest_path.exists() {
        eprintln!("skipping: {:?} not found", manifest_path);
        return;
    }

    let text = fs::read_to_string(&manifest_path).expect("read manifest");
    let parsed = engine_wasm::ManifestFile::from_toml_str(&text).expect("parse");

    let hello = parsed
        .modules
        .iter()
        .find(|m| m.name == "hello-world")
        .expect("hello-world should be registered in manifest.toml");

    let wasm_full = modules.join(&hello.wasm_path);
    if !wasm_full.exists() {
        eprintln!(
            "skipping: {:?} not built yet. Run `cd modules && cargo build --release --target wasm32-wasip2` to produce it.",
            wasm_full
        );
        return;
    }

    let bytes = fs::read(&wasm_full).expect("read wasm");
    assert!(
        bytes.len() > 8,
        "wasm file should be larger than its magic + version header"
    );
    // WebAssembly binary format magic: 0x00 'a' 's' 'm'
    assert_eq!(&bytes[0..4], b"\0asm", "missing wasm magic header");

    // Version header. wasm32-wasip2 默认产 Component(`0d 00 01 00`),
    // 不是 core module(`01 00 00 00`)。两者都接受,但记录是哪种。
    let version = &bytes[4..8];
    let kind = match version {
        [0x01, 0x00, 0x00, 0x00] => "core module",
        [0x0d, 0x00, 0x01, 0x00] => "component",
        other => panic!("unrecognized wasm version header: {:?}", other),
    };
    eprintln!(
        "✓ {} bytes, valid wasm ({}) at {}",
        bytes.len(),
        kind,
        wasm_full.display()
    );

    // Phase 1 hello-world 在 wasip2 下应当是 Component。Phase 2 接 wit-bindgen
    // 后变成更真实的 component(导出 connector world);现在断言它至少是 component。
    assert_eq!(
        kind, "component",
        "wasm32-wasip2 默认产 Component,但拿到 {} — 工具链可能改了",
        kind
    );
}
