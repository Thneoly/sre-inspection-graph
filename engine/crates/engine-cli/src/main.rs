//! engine-cli —— headless 入口。Phase 1 是 banner,Phase 2 起加 `tick` 子命令
//! 把 [`engine_wasm::WasmRuntime`] 拉起来跑 sync_all。
//!
//! 用法:
//!
//! ```bash
//! # 启动 banner(原 Phase 1 行为)
//! engine-cli
//!
//! # 单次 sync —— 加载 manifest、跑所有 connector、打印 batch 摘要 + Fact JSON
//! engine-cli tick
//!
//! # 持续 sync —— 周期跑,直到 Ctrl-C
//! engine-cli tick --loop --interval=30
//! ```
//!
//! `MODULES_ROOT` env 可覆盖 manifest 根目录,缺省按 cwd 探测(`./modules` →
//! `../modules` → `../../modules`)。Phase 3 起改成 clap + config file,本期保留
//! 零依赖最小实现。

#![deny(unsafe_code)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use engine_wasm::{ManifestFile, WasmRuntime};
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => run_banner(),
        Some("tick") => run_tick(&args[1..]).await,
        Some(other) => Err(anyhow!(
            "unknown subcommand: {other}; supported: tick"
        )),
    }
}

/// Phase 1 boot banner —— REST/Flight 还没接,启动证明 workspace 链接图正确。
fn run_banner() -> Result<()> {
    info!(
        version = engine_core::version(),
        "engine-cli skeleton boot (Phase 1 — REST/Flight not wired yet)"
    );
    info!("storage backend: {}", placeholder_backend_name());
    info!("hint: try `engine-cli tick` to load manifest and run sync_all");
    Ok(())
}

fn placeholder_backend_name() -> &'static str {
    struct Stub;
    impl engine_storage::Storage for Stub {
        fn backend_name(&self) -> &'static str {
            "stub-sqlite"
        }
    }
    let s: Box<dyn engine_storage::Storage> = Box::new(Stub);
    s.backend_name()
}

/// `tick` 子命令实现。
///
/// flags:
/// - `--loop` —— 不退出,持续 sync。无此 flag → 跑一次后退出
/// - `--interval=<seconds>` —— `--loop` 模式下的轮询间隔(默认 30)
async fn run_tick(args: &[String]) -> Result<()> {
    let mut continuous = false;
    let mut interval: u64 = 30;
    for arg in args {
        if arg == "--loop" {
            continuous = true;
        } else if let Some(v) = arg.strip_prefix("--interval=") {
            interval = v
                .parse()
                .with_context(|| format!("invalid --interval value: {v}"))?;
        } else {
            return Err(anyhow!("unknown tick flag: {arg}"));
        }
    }

    let modules_root = resolve_modules_root()?;
    let manifest = load_manifest(&modules_root)?;
    info!(
        modules_root = %modules_root.display(),
        modules_total = manifest.modules.len(),
        connectors = manifest.modules.iter().filter(|m| m.kind == "connector").count(),
        "loaded manifest"
    );

    let runtime = WasmRuntime::from_manifest(&modules_root, &manifest).await?;
    for (name, err) in &runtime.load_errors {
        warn!(connector = %name, error = %err, "skipped — load failed");
    }
    info!(
        loaded = runtime.connector_count(),
        names = ?runtime.connector_names(),
        "WasmRuntime ready"
    );

    if runtime.connector_count() == 0 {
        return Err(anyhow!(
            "no connectors loaded — check {}; have you run `cd modules && cargo wasi-build`?",
            modules_root.join("manifest.toml").display()
        ));
    }

    if continuous {
        info!(interval, "entering tick loop (Ctrl-C to exit)");
        runtime.tick_loop(interval).await?;
        Ok(())
    } else {
        let summary = runtime.sync_all("{}").await;
        print_summary(&summary)?;
        Ok(())
    }
}

/// 解析 modules 根目录。优先级:`MODULES_ROOT` env → cwd 邻近 1-3 层探测。
fn resolve_modules_root() -> Result<PathBuf> {
    if let Ok(v) = env::var("MODULES_ROOT") {
        let p = PathBuf::from(v);
        if !p.exists() {
            return Err(anyhow!("MODULES_ROOT not found: {}", p.display()));
        }
        return Ok(p);
    }
    let cwd = env::current_dir().context("current_dir")?;
    for rel in [".", "..", "../..", "../../.."] {
        let p = cwd.join(rel).join("modules");
        if p.join("manifest.toml").exists() {
            return Ok(p.canonicalize().unwrap_or(p));
        }
    }
    Err(anyhow!(
        "could not find modules/ near cwd ({}); set MODULES_ROOT env",
        cwd.display()
    ))
}

fn load_manifest(modules_root: &Path) -> Result<ManifestFile> {
    let path = modules_root.join("manifest.toml");
    let text = fs::read_to_string(&path)
        .with_context(|| format!("read manifest: {}", path.display()))?;
    ManifestFile::from_toml_str(&text)
        .with_context(|| format!("parse manifest: {}", path.display()))
}

/// 打印 sync_all 摘要 —— 控制台友好的多行格式 + 末尾 JSON dump batch 内容。
fn print_summary(summary: &engine_wasm::SyncSummary) -> Result<()> {
    println!("\n=== tick summary ===");
    println!("connectors: {}", summary.per_connector.len());
    println!("facts:      {}", summary.batch.len());
    println!("errors:     {}", summary.total_errors);
    println!("guest ms:   {}", summary.total_duration_ms);
    println!();
    for s in &summary.per_connector {
        println!(
            "  - {}: {} fact(s){}",
            s.name,
            s.fact_count,
            if s.errors.is_empty() {
                String::new()
            } else {
                format!(" ({} error(s))", s.errors.len())
            },
        );
        for e in &s.errors {
            println!("      ! {e}");
        }
    }
    println!();
    println!("--- facts (JSON) ---");
    let facts_json = serde_json::to_string_pretty(summary.batch.as_slice())
        .context("serialize facts")?;
    println!("{facts_json}");
    // 顺手验一下 → Arrow RecordBatch
    let rb = summary
        .batch
        .to_record_batch()
        .context("batch → RecordBatch")?;
    println!(
        "\n--- arrow RecordBatch: {} rows × {} cols ---",
        rb.num_rows(),
        rb.num_columns()
    );
    Ok(())
}
