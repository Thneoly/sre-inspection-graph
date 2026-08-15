//! wasmtime Component 实例化耗时基准(宿主加载一个 connector 的成本)。
//!
//! 对目录下所有 connector wasm 各实例化 N 次,输出 mean / p50 / max。
//! handler-world 的 wasm(scale_deploy / k8s_handler)不是 connector world,
//! 加载失败会被跳过并标注。
//!
//! ```bash
//! cargo run -p engine-wasm --release --example bench_load -- \
//!   modules/target/wasm32-wasip2/release 20
//! ```

use engine_wasm::WasmConnector;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "modules/target/wasm32-wasip2/release".into());
    let iters: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    // 传入超集 capability:本基准只测实例化,不测运行期授权。
    let caps: std::collections::HashSet<String> = [
        "logging",
        "clock",
        "http-client",
        "http-write",
        "fs-read",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    let mut paths: Vec<_> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "wasm"))
        .collect();
    paths.sort();

    println!("wasm\tsize_kb\tmean_ms\tp50_ms\tmax_ms\tn");
    for path in paths {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let size_kb = std::fs::metadata(&path)?.len() / 1024;
        let mut samples = Vec::with_capacity(iters);
        let mut skip = false;
        for _ in 0..iters {
            let t = Instant::now();
            match WasmConnector::load(&path, caps.clone()).await {
                Ok(c) => {
                    let dt = t.elapsed();
                    drop(c);
                    samples.push(dt.as_secs_f64() * 1000.0);
                }
                Err(_) => {
                    // handler-world 的 wasm 不是 connector world —— 跳过
                    skip = true;
                    break;
                }
            }
        }
        if skip {
            println!("{name}\t{size_kb}\t-(- skip: 非 connector world)");
            continue;
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mean: f64 = samples.iter().sum::<f64>() / samples.len() as f64;
        println!(
            "{name}\t{size_kb}\t{mean:.1}\t{:.1}\t{:.1}\t{iters}",
            samples[samples.len() / 2],
            samples[samples.len() - 1],
        );
    }
    Ok(())
}
