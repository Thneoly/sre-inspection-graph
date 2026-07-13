//! HandlerExecutor trait + HandlerOutcome + MockHandlerExecutor(Phase 3.9)。
//!
//! 把 handler 调用从 execution.rs 直接调 [`crate::handlers::HANDLERS`] 解耦成 trait 注入:
//! - mock 模式:[`MockHandlerExecutor`] 调 host mock handler(handlers.rs),返 [`HandlerOutcome`]
//! - real 模式(3.9a-3b2):`WasmHandlerExecutor` 调 WASM handler-world execute(真改集群)
//!
//! execution.rs `run_handler` 经 `&dyn HandlerExecutor` 调 handler,替代直接 `get_handler`。
//! handler 不再 mutate twin,而是返 `HandlerOutcome { attributes_json }`,run_handler 据此
//! 更新 twin attrs(动作生效后的新状态,供 verifier 读)。
//!
//! **3.9a async 重构**:`HandlerExecutor` trait 改 `async`(用 `async_trait`),因 real handler
//! (WasmHandler)是 async I/O。execution.rs + chains.rs 全 async,tests tokio::test。

#![allow(missing_docs)]

use async_trait::async_trait;
use engine_identity::ResolvedNode;
use serde_json::Value;

use crate::handlers::get_handler;
use crate::models::ExecutionContext;

/// handler 执行结果。
///
/// - `success`:动作是否成功(取 `result.success`)。
/// - `result`:flat result dict(对齐 reference handler 返的,含 success/error/action-specific 字段)。
/// - `attributes_json`:动作生效后的**新 attrs** JSON 字符串;`None` = twin 不变(无持续副作用)。
#[derive(Debug, Clone)]
pub struct HandlerOutcome {
    pub success: bool,
    pub result: Value,
    pub attributes_json: Option<String>,
}

impl HandlerOutcome {
    pub fn new(result: Value, attributes_json: Option<String>) -> Self {
        let success = result.get("success").and_then(Value::as_bool).unwrap_or(false);
        Self { success, result, attributes_json }
    }

    pub fn ok(result: Value, attributes_json: Option<String>) -> Self {
        Self::new(result, attributes_json)
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            result: serde_json::json!({ "success": false, "error": msg.into() }),
            attributes_json: None,
        }
    }
}

/// handler 执行器 trait(3.9a async:`async fn execute`)。
///
/// execution.rs `run_handler` 经此调 handler。mock 模式用 [`MockHandlerExecutor`];
/// real 模式(3.9a-3b2)用 `WasmHandlerExecutor`(调 WASM handler-world execute,真改集群)。
#[async_trait]
pub trait HandlerExecutor: Send + Sync {
    /// 执行 action,返 [`HandlerOutcome`]。
    async fn execute(
        &self,
        action_id: &str,
        target: &ResolvedNode,
        params: &Value,
        ctx: &ExecutionContext,
    ) -> HandlerOutcome;
}

/// mock 执行器:调 host mock handler([`crate::handlers::HANDLERS`]),返 [`HandlerOutcome`]。
#[derive(Debug, Default, Clone, Copy)]
pub struct MockHandlerExecutor;

#[async_trait]
impl HandlerExecutor for MockHandlerExecutor {
    async fn execute(
        &self,
        action_id: &str,
        target: &ResolvedNode,
        params: &Value,
        ctx: &ExecutionContext,
    ) -> HandlerOutcome {
        match get_handler(action_id) {
            Some(h) => h(target, params, ctx),
            None => HandlerOutcome::err(format!("no handler for action {action_id}")),
        }
    }
}
