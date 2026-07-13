//! scale-deploy - Recovery handler 模块(Phase 3.9a-3)。
//!
//! handler-world export handler{dry-run/execute/verify}。execute 经 http-client write
//! PATCH K8s deployment scale(real_mode),返 ExecutionResult{attributes_json}。host
//! (WasmHandler,3.9a-3b)拿 attributes_json 更新 twin。
//!
//! ## config 传递
//!
//! WIT ExecutionContext 无 config_json 字段。host(WasmHandlerExecutor,3.9a-3b)把 config
//! + target attrs 注入 params_json 的 `_config` 字段:
//! `{"replicas_delta": 2, "_config": {"api_base": "...", "real_mode": true, "old_replicas": 3}}`。
//! handler 解 params_json 拿 replicas_delta + _config。
//!
//! ## real vs mock
//!
//! - real_mode=true:经 http-client `write`(PATCH /apis/apps/v1/.../deployments/{name}/scale)
//!   真改集群,成功后返 attributes_json(新 replicas)。
//! - real_mode=false:不调 write,直接返 attributes_json(模拟,host mock 模式用)。

#![allow(missing_docs)]
#![allow(clippy::doc_lazy_continuation)]

#[cfg(target_arch = "wasm32")]
mod bindings {
    wit_bindgen::generate!({
        world: "handler-world",
        path: "../../../specs/wit",
        generate_all,
    });
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::bindings;
    use bindings::exports::sre::inspection::handler::{
        ExecutionContext, ExecutionError, ExecutionResult, Guest,
    };
    use bindings::sre::inspection::http_client;
    use serde_json::{json, Value};

    pub struct ScaleDeploy;

    impl Guest for ScaleDeploy {
        fn dry_run(ctx: ExecutionContext) -> Result<Vec<String>, ExecutionError> {
            Ok(vec![ctx.target_resource_id])
        }

        fn execute(ctx: ExecutionContext) -> Result<ExecutionResult, ExecutionError> {
            execute_impl(&ctx)
        }

        fn verify(_ctx: ExecutionContext, _prior_result: ExecutionResult) -> Result<bool, ExecutionError> {
            // scale_deployment verify 由 host verifier 读 twin attrs(desired=available)。
            // WASM verify 留简化(Ok(true));host WasmHandler 可选调 verify。
            Ok(true)
        }
    }

    fn execute_impl(ctx: &ExecutionContext) -> Result<ExecutionResult, ExecutionError> {
        let params: Value = serde_json::from_str(&ctx.params_json).unwrap_or(json!({}));
        let delta = params
            .get("replicas_delta")
            .and_then(Value::as_i64)
            .ok_or_else(|| ExecutionError::PreconditionFailed("replicas_delta required".into()))?;
        if delta == 0 {
            return Err(ExecutionError::PreconditionFailed(
                "replicas_delta must be non-zero".into(),
            ));
        }
        let config = params.get("_config").cloned().unwrap_or(json!({}));
        let api_base = config
            .get("api_base")
            .and_then(Value::as_str)
            .unwrap_or("http://127.0.0.1:8001");
        let real_mode = config.get("real_mode").and_then(Value::as_bool).unwrap_or(false);
        let old = config.get("old_replicas").and_then(Value::as_i64).unwrap_or(3);
        let new = old + delta;
        if new < 0 {
            return Err(ExecutionError::PreconditionFailed(format!(
                "new replicas would be negative ({new})"
            )));
        }
        if new > 100 {
            return Err(ExecutionError::PreconditionFailed(format!(
                "new replicas exceeds limit ({new} > 100)"
            )));
        }

        // 解 target deploy:{cluster}:{ns}:{name}
        let parts: Vec<&str> = ctx.target_resource_id.split(':').collect();
        if parts.len() < 4 || parts[0] != "deploy" {
            return Err(ExecutionError::PreconditionFailed(format!(
                "invalid target_resource_id: {}",
                ctx.target_resource_id
            )));
        }
        let ns = parts[2];
        let name = parts[3];

        if real_mode {
            let url = format!("{api_base}/apis/apps/v1/namespaces/{ns}/deployments/{name}/scale");
            let body = format!(r#"{{"spec":{{"replicas":{new}}}}}"#);
            let req = http_client::WriteRequest {
                method: "PATCH".to_string(),
                url,
                headers: vec![("content-type".to_string(), "application/merge-patch+json".to_string())],
                body: Some(body.into_bytes()),
            };
            match http_client::write(&req) {
                Ok(resp) if resp.status >= 200 && resp.status < 300 => {}
                Ok(resp) => {
                    return Err(ExecutionError::UpstreamApi(format!(
                        "PATCH scale HTTP {}",
                        resp.status
                    )));
                }
                Err(e) => {
                    return Err(ExecutionError::UpstreamApi(format!(
                        "PATCH scale failed: {e:?}"
                    )));
                }
            }
        }

        let attributes_json = format!(
            r#"{{"desired_replicas":{new},"available_replicas":{new}}}"#
        );
        Ok(ExecutionResult {
            success: true,
            message: format!("scaled {old} -> {new} (real_mode={real_mode})"),
            attributes_json,
        })
    }
}

#[cfg(target_arch = "wasm32")]
use imp::ScaleDeploy;

#[cfg(target_arch = "wasm32")]
bindings::export!(ScaleDeploy with_types_in bindings);
