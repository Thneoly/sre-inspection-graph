//! hello-world — 真 WASM Component connector,导出 `sre:inspection/connector`
//! 接口的 `sync()` 和 `health-check()`,每次 sync 产一条 demo Fact。
//!
//! 这是 Phase 2 第一刀:把之前的纯 Rust 函数升级为真正的 Component(用
//! wit-bindgen 的 `generate!` 宏生成 export glue)。host 端
//! engine-wasm 用 wasmtime 加载此产物并调 sync,拿回 Fact 列表。
//!
//! 设计参考 /home/cc/Desktop/code/ntx/show/ntxdemo/component/scheduler/src/lib.rs
//! 的 `wit_bindgen::generate!` + `export!` 双段模式。
//!
//! cfg(target_arch = "wasm32") 隔离:wit-bindgen 生成的 export symbol 包含
//! `:` / `@` 字符(如 `cabi_post_sre:inspection/connector@0.1.0#sync`),host
//! 链接器认不出。所以 host 编译时这段代码不参与,空 crate 链接成功。
//! 真正用此 crate 的只有 wasm32-wasip2 target build。

#![allow(missing_docs)]

#[cfg(target_arch = "wasm32")]
mod bindings {
    wit_bindgen::generate!({
        world: "connector-world",
        path: "../../../specs/wit",
        // 让 wit-bindgen 把所有共享类型(Fact 等)生成到 bindings::exports::...
        generate_all,
    });
}

#[cfg(target_arch = "wasm32")]
use bindings::exports::sre::inspection::connector::{Fact, Guest, SyncError, SyncResult};

#[cfg(target_arch = "wasm32")]
struct HelloWorld;

#[cfg(target_arch = "wasm32")]
impl Guest for HelloWorld {
    /// 单次 sync — 产一条 demo Fact。`config-json` 当前不用,Phase 3 起从此读
    /// connector 实例配置(认证、scrape interval 覆盖等)。
    fn sync(_config_json: String) -> Result<SyncResult, SyncError> {
        // wit-bindgen 把 clock interface 生成成 host import,这里调它拿时间戳
        let now_seconds = bindings::sre::inspection::clock::now_seconds();
        bindings::sre::inspection::logging::log(
            bindings::sre::inspection::logging::Level::Info,
            "hello-world sync invoked",
        );

        let fact = Fact {
            id: "hello-world-fact-1".to_string(),
            kind: "topology-node".to_string(),
            source: "hello-world".to_string(),
            resource_id: "demo:placeholder:default:hello".to_string(),
            resource_type: "Placeholder".to_string(),
            timestamp: now_seconds,
            attributes_json: r#"{"greeting":"hello, world"}"#.to_string(),
        };

        Ok(SyncResult {
            facts: vec![fact],
            errors: vec![],
            duration_ms: 0,
        })
    }

    fn health_check() -> bool {
        true
    }
}

// 把上面的 HelloWorld 注册为 connector world 的 export。
#[cfg(target_arch = "wasm32")]
bindings::export!(HelloWorld with_types_in bindings);
