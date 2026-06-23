//! hello-world — 占位 WASM connector,每次 sync 产一条假 Fact。
//!
//! 目的:
//! 1. 验证 modules/ workspace 编译链路(含 wasm32-wasip2 真实产物)
//! 2. 作为 engine-wasm 加载真实 .wasm 的最小例子(Step 4 后续 PR 接入)
//! 3. 给后续 5 个 real connector(k8s/prom/jaeger/flagd/k8s_events)做模板
//!
//! Phase 1 此实现是 host-target 友好的纯 Rust 函数,Step 4 后续 PR 会用
//! wit-bindgen 宏改造为真正的 wasm32-wasip2 Component Model guest。
//!
//! 注:`wasm32-wasip2` 有完整 libstd,这里直接用 std,不需要 no_std。

#![allow(missing_docs)]

use module_sdk::{Fact, SyncError, SyncResult};

/// 一次 sync 调用 — 返回固定一条 demo Fact。
pub fn sync_once(now_seconds: u64) -> Result<(SyncResult, Vec<Fact>), SyncError> {
    let fact = Fact {
        id: "hello-world-fact-1".to_string(),
        kind: "topology-node".to_string(),
        source: "hello-world".to_string(),
        resource_id: "demo:placeholder:default:hello".to_string(),
        resource_type: "Placeholder".to_string(),
        timestamp: now_seconds,
        attributes_json: r#"{"greeting":"hello, world"}"#.to_string(),
    };

    let result = SyncResult {
        facts_emitted: 1,
        errors: Vec::new(),
        duration_ms: 0,
    };
    Ok((result, vec![fact]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_once_emits_one_fact() {
        let (result, facts) = sync_once(1_700_000_000).expect("should succeed");
        assert_eq!(result.facts_emitted, 1);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].source, "hello-world");
        assert_eq!(facts[0].timestamp, 1_700_000_000);
    }

    #[test]
    fn fact_attributes_are_valid_json() {
        let (_, facts) = sync_once(0).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&facts[0].attributes_json).unwrap();
        assert_eq!(parsed["greeting"], "hello, world");
    }
}
