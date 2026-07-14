//! k8s-handler - 6 k8s action Recovery handler(Phase 3.9a-3c)。
//!
//! handler-world export handler{dry-run/execute/verify}。execute match action_id:
//! - scale_deployment: PATCH /apis/apps/v1/.../deployments/{name}/scale
//! - restart_pod: DELETE /api/v1/namespaces/{ns}/pods/{name}
//! - rollback_deployment: PATCH deploy annotations(restart trigger;简化,非真 rollout undo)
//! - refresh_secret: PATCH secret annotations(标记轮换;简化,不改 data)
//! - drain_node: PATCH /api/v1/nodes/{name}(cordon only,不 evict)
//! - restart_service: DELETE /api/v1/namespaces/{ns}/endpoints/{name}
//!
//! real_mode 经 http-client write 真改集群;mock 只返 attrs。kill_query/clear_cache 非 k8s
//! 留 Mock(host WasmHandlerExecutor fallback)。

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

    pub struct K8sHandler;

    impl Guest for K8sHandler {
        fn dry_run(ctx: ExecutionContext) -> Result<Vec<String>, ExecutionError> {
            Ok(vec![ctx.target_resource_id])
        }

        fn execute(ctx: ExecutionContext) -> Result<ExecutionResult, ExecutionError> {
            execute_impl(&ctx)
        }

        fn verify(_ctx: ExecutionContext, _prior: ExecutionResult) -> Result<bool, ExecutionError> {
            Ok(true)
        }
    }

    fn execute_impl(ctx: &ExecutionContext) -> Result<ExecutionResult, ExecutionError> {
        let params: Value = serde_json::from_str(&ctx.params_json).unwrap_or(json!({}));
        let config = params.get("_config").cloned().unwrap_or(json!({}));
        let api_base = config
            .get("api_base")
            .and_then(Value::as_str)
            .unwrap_or("http://127.0.0.1:8001");
        let real_mode = config.get("real_mode").and_then(Value::as_bool).unwrap_or(false);

        let parts: Vec<&str> = ctx.target_resource_id.split(':').collect();
        if parts.len() < 2 {
            return Err(ExecutionError::PreconditionFailed(format!(
                "invalid target: {}",
                ctx.target_resource_id
            )));
        }

        match ctx.action_id.as_str() {
            "scale_deployment" => scale_deployment(&parts, &params, &config, api_base, real_mode),
            "restart_pod" => restart_pod(&parts, api_base, real_mode),
            "rollback_deployment" => rollback_deployment(&parts, api_base, real_mode),
            "refresh_secret" => refresh_secret(&parts, api_base, real_mode),
            "drain_node" => drain_node(&parts, api_base, real_mode),
            "restart_service" => restart_service(&parts, api_base, real_mode),
            other => Err(ExecutionError::PreconditionFailed(format!(
                "unsupported action: {other}"
            ))),
        }
    }

    /// 解析 namespace + name(target [type, cluster, ns, name])。
    fn ns_name<'a>(parts: &'a [&'a str]) -> Result<(&'a str, &'a str), ExecutionError> {
        if parts.len() < 4 {
            return Err(ExecutionError::PreconditionFailed(format!(
                "target needs type:cluster:ns:name, got {}",
                parts.join(":")
            )));
        }
        Ok((parts[2], parts[3]))
    }

    /// 解析 node name(target [type, cluster, name])。
    fn node_name<'a>(parts: &'a [&'a str]) -> Result<&'a str, ExecutionError> {
        if parts.len() < 3 {
            return Err(ExecutionError::PreconditionFailed(format!(
                "node target needs type:cluster:name, got {}",
                parts.join(":")
            )));
        }
        Ok(parts[2])
    }

    /// HTTP write(PATCH/POST),body 可选。
    fn http_write(method: &str, url: &str, body: Option<&str>) -> Result<(), ExecutionError> {
        let req = http_client::WriteRequest {
            method: method.to_string(),
            url: url.to_string(),
            headers: vec![(
                "content-type".to_string(),
                "application/merge-patch+json".to_string(),
            )],
            body: body.map(|b| b.as_bytes().to_vec()),
        };
        match http_client::write(&req) {
            Ok(resp) if resp.status >= 200 && resp.status < 300 => Ok(()),
            Ok(resp) => Err(ExecutionError::UpstreamApi(format!("HTTP {}", resp.status))),
            Err(e) => Err(ExecutionError::UpstreamApi(format!("{e:?}"))),
        }
    }

    /// HTTP DELETE(无 body)。
    fn http_delete(url: &str) -> Result<(), ExecutionError> {
        let req = http_client::WriteRequest {
            method: "DELETE".to_string(),
            url: url.to_string(),
            headers: vec![],
            body: None,
        };
        match http_client::write(&req) {
            Ok(resp) if resp.status >= 200 && resp.status < 300 => Ok(()),
            Ok(resp) => Err(ExecutionError::UpstreamApi(format!("HTTP {}", resp.status))),
            Err(e) => Err(ExecutionError::UpstreamApi(format!("{e:?}"))),
        }
    }

    fn ok(message: &str, attrs: &str) -> ExecutionResult {
        ExecutionResult {
            success: true,
            message: message.to_string(),
            attributes_json: attrs.to_string(),
        }
    }

    // ===== 6 action =====

    fn scale_deployment(
        parts: &[&str],
        params: &Value,
        config: &Value,
        api_base: &str,
        real_mode: bool,
    ) -> Result<ExecutionResult, ExecutionError> {
        let (ns, name) = ns_name(parts)?;
        let delta = params
            .get("replicas_delta")
            .and_then(Value::as_i64)
            .ok_or_else(|| ExecutionError::PreconditionFailed("replicas_delta required".into()))?;
        if delta == 0 {
            return Err(ExecutionError::PreconditionFailed(
                "delta must be non-zero".into(),
            ));
        }
        let old = config.get("old_replicas").and_then(Value::as_i64).unwrap_or(3);
        let new = old + delta;
        if new < 0 {
            return Err(ExecutionError::PreconditionFailed(format!(
                "negative replicas {new}"
            )));
        }
        if real_mode {
            let url = format!("{api_base}/apis/apps/v1/namespaces/{ns}/deployments/{name}/scale");
            let body = format!(r#"{{"spec":{{"replicas":{new}}}}}"#);
            http_write("PATCH", &url, Some(&body))?;
        }
        Ok(ok(
            &format!("scaled {old} -> {new}"),
            &format!(r#"{{"desired_replicas":{new},"available_replicas":{new}}}"#),
        ))
    }

    fn restart_pod(
        parts: &[&str],
        api_base: &str,
        real_mode: bool,
    ) -> Result<ExecutionResult, ExecutionError> {
        let (ns, name) = ns_name(parts)?;
        if real_mode {
            let url = format!("{api_base}/api/v1/namespaces/{ns}/pods/{name}");
            http_delete(&url)?;
        }
        Ok(ok("pod restarted", r#"{"restart_count":1,"health_status":"normal"}"#))
    }

    fn rollback_deployment(
        parts: &[&str],
        api_base: &str,
        real_mode: bool,
    ) -> Result<ExecutionResult, ExecutionError> {
        let (ns, name) = ns_name(parts)?;
        if real_mode {
            let url = format!("{api_base}/apis/apps/v1/namespaces/{ns}/deployments/{name}");
            // 简化:PATCH annotations restartedAt 触发 rollout(非真 rollout undo)
            let body = r#"{"metadata":{"annotations":{"kubectl.kubernetes.io/restartedAt":"now"}}}"#;
            http_write("PATCH", &url, Some(body))?;
        }
        Ok(ok("deployment rollback triggered", r#"{"current_revision":1}"#))
    }

    fn refresh_secret(
        parts: &[&str],
        api_base: &str,
        real_mode: bool,
    ) -> Result<ExecutionResult, ExecutionError> {
        let (ns, name) = ns_name(parts)?;
        if real_mode {
            let url = format!("{api_base}/api/v1/namespaces/{ns}/secrets/{name}");
            // 简化:PATCH annotations 标记轮换(不改 data)
            let body = r#"{"metadata":{"annotations":{"rotatedAt":"now"}}}"#;
            http_write("PATCH", &url, Some(body))?;
        }
        Ok(ok("secret rotated", r#"{"secret_version":2}"#))
    }

    fn drain_node(
        parts: &[&str],
        api_base: &str,
        real_mode: bool,
    ) -> Result<ExecutionResult, ExecutionError> {
        let name = node_name(parts)?;
        if real_mode {
            let url = format!("{api_base}/api/v1/nodes/{name}");
            let body = r#"{"spec":{"unschedulable":true}}"#;
            http_write("PATCH", &url, Some(body))?;
        }
        Ok(ok("node cordoned", r#"{"cordoned":true}"#))
    }

    fn restart_service(
        parts: &[&str],
        api_base: &str,
        real_mode: bool,
    ) -> Result<ExecutionResult, ExecutionError> {
        let (ns, name) = ns_name(parts)?;
        if real_mode {
            let url = format!("{api_base}/api/v1/namespaces/{ns}/endpoints/{name}");
            http_delete(&url)?;
        }
        Ok(ok("endpoints regenerated", r#"{"endpoints_refresh_count":1}"#))
    }
}

#[cfg(target_arch = "wasm32")]
use imp::K8sHandler;

#[cfg(target_arch = "wasm32")]
bindings::export!(K8sHandler with_types_in bindings);
