//! engine-cli smoke test —— 跑实际 binary 验证 `tick` 子命令端到端 OK。
//!
//! 找不到 hello_world.wasm 时跳过(本地需先 `cd modules && cargo wasi-build`)。
//! 该测试 explicit 跑 release/debug binary 路径,绕过 `cargo test` 默认 in-process
//! 测试模式 —— 因为我们要验的是 main() 启动 + tracing init + tokio runtime 全链。

use std::path::PathBuf;
use std::process::Command;

fn modules_root() -> PathBuf {
    let crate_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(crate_dir).join("../../../modules")
}

fn engine_cli_binary() -> Option<PathBuf> {
    // cargo test 会先把所有 dep binary 构出来。CARGO_BIN_EXE_engine-cli 是
    // cargo 测试时自动注入的、指向 engine-cli debug binary 的绝对路径。
    option_env!("CARGO_BIN_EXE_engine-cli").map(PathBuf::from)
}

fn hello_world_wasm_built() -> bool {
    modules_root()
        .join("target/wasm32-wasip2/release/hello_world.wasm")
        .exists()
}

#[test]
fn banner_runs_without_subcommand() {
    let Some(bin) = engine_cli_binary() else {
        eprintln!("skipping: CARGO_BIN_EXE_engine-cli not set (run via `cargo test`)");
        return;
    };
    let out = Command::new(&bin).output().expect("run engine-cli");
    assert!(
        out.status.success(),
        "engine-cli exited non-zero: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    // tracing-subscriber::fmt() 默认写 stdout(早期版本写 stderr,2024+ 已切 stdout)
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("skeleton boot"),
        "banner missing: {combined}"
    );
}

#[test]
fn tick_loads_manifest_and_emits_facts() {
    let Some(bin) = engine_cli_binary() else {
        return;
    };
    if !hello_world_wasm_built() {
        eprintln!(
            "skipping: hello_world.wasm not built. \
             Run `cd modules && cargo wasi-build` first."
        );
        return;
    }

    let out = Command::new(&bin)
        .arg("tick")
        .env("MODULES_ROOT", modules_root())
        .output()
        .expect("run engine-cli tick");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "tick exited non-zero: stdout={stdout} stderr={stderr}"
    );
    // 关键标记:summary 段、hello-world 行、arrow 行
    assert!(stdout.contains("=== tick summary ==="), "summary missing");
    assert!(
        stdout.contains("hello-world: 1 fact(s)"),
        "fact count missing: {stdout}"
    );
    assert!(
        stdout.contains("arrow RecordBatch"),
        "arrow summary missing"
    );
    // 如果 k8s-mini 也 build 了(modules/manifest.toml 默认包含),要看到它
    if modules_root()
        .join("target/wasm32-wasip2/release/k8s_mini.wasm")
        .exists()
    {
        assert!(
            stdout.contains("k8s-mini:"),
            "k8s-mini summary line missing: {stdout}"
        );
    }
}

#[test]
fn tick_rejects_unknown_flags() {
    let Some(bin) = engine_cli_binary() else {
        return;
    };
    let out = Command::new(&bin)
        .args(["tick", "--bogus"])
        .env("MODULES_ROOT", modules_root())
        .output()
        .expect("run engine-cli");
    assert!(
        !out.status.success(),
        "expected non-zero exit on unknown flag, got success"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unknown tick flag"),
        "error msg missing: {stderr}"
    );
}
