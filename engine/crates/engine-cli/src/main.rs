//! engine-cli — headless binary,Phase 4 起补 REST + Arrow Flight。Phase 1 仅
//! 启动横幅,确认 workspace 链路与依赖图正确。

#![deny(unsafe_code)]

use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!(
        version = engine_core::version(),
        "engine-cli skeleton boot (Phase 1 — REST/Flight not wired yet)"
    );
    info!("storage backend: {}", placeholder_backend_name());

    Ok(())
}

fn placeholder_backend_name() -> &'static str {
    // 用一下 engine_storage trait 确认链接图通。
    struct Stub;
    impl engine_storage::Storage for Stub {
        fn backend_name(&self) -> &'static str {
            "stub-sqlite"
        }
    }
    let s: Box<dyn engine_storage::Storage> = Box::new(Stub);
    s.backend_name()
}
