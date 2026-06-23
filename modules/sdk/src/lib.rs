//! module-sdk — 共享 guest 端 SDK。
//!
//! Phase 1 仅提供 Fact / SyncResult / SyncError 等数据结构(与 WIT 接口
//! 字段对齐),以便 connector / rule / handler 在 host 端 dev build 跑测试。
//! 真实 WIT bindings 生成留 Step 4 后续 PR 接入 wit-bindgen 宏。

#![cfg_attr(not(test), no_std)]
#![allow(missing_docs)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

/// 与 specs/wit/connector.wit 的 `fact` record 对齐。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fact {
    pub id: String,
    pub kind: String,
    pub source: String,
    pub resource_id: String,
    pub resource_type: String,
    pub timestamp: u64,
    pub attributes_json: String,
}

/// connector.sync 返回值。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResult {
    pub facts_emitted: u64,
    pub errors: Vec<String>,
    pub duration_ms: u64,
}

/// connector.sync 错误枚举。与 WIT `sync-error` variant 对齐。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "msg", rename_all = "kebab-case")]
pub enum SyncError {
    Config(String),
    Runtime(String),
    Timeout,
    CapabilityDenied(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    extern crate std;
    use std::string::ToString;

    #[test]
    fn fact_round_trips_json() {
        let fact = Fact {
            id: "fact-1".to_string(),
            kind: "topology-node".to_string(),
            source: "hello-world".to_string(),
            resource_id: "demo:cluster:ns:pod".to_string(),
            resource_type: "Pod".to_string(),
            timestamp: 1_700_000_000,
            attributes_json: "{\"foo\":\"bar\"}".to_string(),
        };
        let s = serde_json::to_string(&fact).unwrap();
        let back: Fact = serde_json::from_str(&s).unwrap();
        assert_eq!(back.id, "fact-1");
    }

    #[test]
    fn sync_error_round_trips() {
        let err = SyncError::Timeout;
        let s = serde_json::to_string(&err).unwrap();
        assert!(s.contains("timeout"));
    }
}
