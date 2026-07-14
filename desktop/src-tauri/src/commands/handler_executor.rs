//! WasmHandlerExecutor - impl async HandlerExecutor,调 WasmHandler(Phase 3.9a-3b2b)。
//!
//! scale_deployment -> WasmHandler.execute(WASM handler-world,real_mode 经 http-client
//! write PATCH scale 真改集群);其他 action -> MockHandlerExecutor fallback。
//!
//! config 传递:WasmHandler.execute 接 params_json,host 把 _config(api_base/real_mode/
//! old_replicas from target attrs)注入 params_json._config。

use std::sync::Arc;

use async_trait::async_trait;
use engine_identity::ResolvedNode;
use engine_recovery::{ExecutionContext, HandlerExecutor, HandlerOutcome, MockHandlerExecutor};
use engine_wasm::WasmHandler;
use serde_json::{json, Value};
use tokio::sync::Mutex;

/// WASM handler 执行器:scale_deployment 走 WasmHandler,其他走 MockHandlerExecutor。
pub struct WasmHandlerExecutor {
    handler: Arc<Mutex<WasmHandler>>,
    mock: MockHandlerExecutor,
    api_base: String,
    real_mode: bool,
}

impl WasmHandlerExecutor {
    /// 构造。`handler` 是 WasmRuntime.handlers 里 scale-deploy 的 Arc<Mutex<WasmHandler>>。
    pub fn new(handler: Arc<Mutex<WasmHandler>>, api_base: String, real_mode: bool) -> Self {
        Self {
            handler,
            mock: MockHandlerExecutor,
            api_base,
            real_mode,
        }
    }
}

#[async_trait]
impl HandlerExecutor for WasmHandlerExecutor {
    async fn execute(
        &self,
        action_id: &str,
        target: &ResolvedNode,
        params: &Value,
        ctx: &ExecutionContext,
    ) -> HandlerOutcome {
        // 只 scale_deployment 走 WasmHandler;其他 action Mock fallback
        if action_id != "scale_deployment" {
            return self.mock.execute(action_id, target, params, ctx).await;
        }

        // 从 target attrs 读 old_replicas(desired_replicas),注入 _config
        let attrs: Value = serde_json::from_str(&target.attributes_json).unwrap_or(json!({}));
        let old_replicas = attrs.get("desired_replicas").and_then(Value::as_i64).unwrap_or(3);
        let config = json!({
            "api_base": self.api_base,
            "real_mode": self.real_mode,
            "old_replicas": old_replicas,
        });
        let mut params_with_config = params.clone();
        if let Value::Object(m) = &mut params_with_config {
            m.insert("_config".to_string(), config);
        }
        let params_json = params_with_config.to_string();

        let mut handler = self.handler.lock().await;
        match handler
            .execute(action_id, &target.resource_id, &params_json, &ctx.initiated_by)
            .await
        {
            Ok(Ok(r)) => HandlerOutcome::ok(
                json!({ "success": r.success, "message": r.message }),
                Some(r.attributes_json),
            ),
            Ok(Err(e)) => HandlerOutcome::err(e.0),
            Err(e) => HandlerOutcome::err(format!("wasm execute failed: {e}")),
        }
    }
}
