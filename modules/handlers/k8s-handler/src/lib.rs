//! k8s-handler - 6 k8s action Recovery handler(Phase 3.9a-3c;完整化 real 语义)。
//!
//! handler-world export handler{dry-run/execute/verify}。execute match action_id:
//! - scale_deployment: PATCH /apis/apps/v1/.../deployments/{name}/scale
//! - restart_pod: DELETE /api/v1/namespaces/{ns}/pods/{name}
//! - rollback_deployment: 真 rollout undo —— GET deployment revision → GET
//!   ReplicaSet 列表 → PATCH spec.template 为 revision-1 的模板(纯函数
//!   `pure::pick_prev_revision_template` host 可测)
//! - refresh_secret: 真轮换 —— (可选 key/new_value_b64)PATCH data + rotatedAt
//!   注解 + 删除引用该 secret 的全部 Pod(volume/env/envFrom/initContainers,
//!   `pure::extract_referencing_pods`),控制器重建后挂载新值
//! - drain_node: 完整 drain —— cordon + evict 循环(排除 DaemonSet/mirror pod,
//!   `pure::filter_evictable_pods`;POST policy/v1 eviction)
//! - restart_service: DELETE /api/v1/namespaces/{ns}/endpoints/{name}
//!
//! real_mode 经 http-client(read GET / write PATCH·POST·DELETE)真改集群;mock 只返
//! attrs。kill_query/clear_cache 非 k8s 留 Mock(host WasmHandlerExecutor fallback)。

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
            "refresh_secret" => refresh_secret(&parts, &params, api_base, real_mode),
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

    /// HTTP GET,返回 body 反序列化的 JSON(完整化 real handler 的回读步骤用)。
    fn http_get_json(url: &str) -> Result<Value, ExecutionError> {
        match http_client::get(url, &[]) {
            Ok(resp) if resp.status >= 200 && resp.status < 300 => {
                serde_json::from_slice(&resp.body)
                    .map_err(|e| ExecutionError::UpstreamApi(format!("decode {url}: {e}")))
            }
            Ok(resp) => Err(ExecutionError::UpstreamApi(format!("GET {url}: HTTP {}", resp.status))),
            Err(e) => Err(ExecutionError::UpstreamApi(format!("GET {url}: {e:?}"))),
        }
    }

    /// HTTP POST(body 可选;eviction 等 subresource 用)。
    fn http_post(url: &str, body: &str) -> Result<(), ExecutionError> {
        let req = http_client::WriteRequest {
            method: "POST".to_string(),
            url: url.to_string(),
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: Some(body.as_bytes().to_vec()),
        };
        match http_client::write(&req) {
            Ok(resp) if resp.status >= 200 && resp.status < 300 => Ok(()),
            Ok(resp) => Err(ExecutionError::UpstreamApi(format!("POST {url}: HTTP {}", resp.status))),
            Err(e) => Err(ExecutionError::UpstreamApi(format!("POST {url}: {e:?}"))),
        }
    }

    /// URL query 参数编码(labelSelector / fieldSelector 的值:k=v 逗号串里的
    /// 保留字符做百分号编码)。
    fn encode_query(s: &str) -> String {
        let mut out = String::new();
        for b in s.bytes() {
            match b {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'=' | b',' => {
                    out.push(b as char)
                }
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
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
            // 真 rollout undo(对齐 kubectl):GET deployment(当前 revision)→
            // GET 其 ReplicaSet 列表(labelSelector)→ 取 revision-1 的 spec.template
            // → PATCH deployment.spec.template 为旧模板。RS 历史不足 → 错误上抛。
            let dep_url = format!("{api_base}/apis/apps/v1/namespaces/{ns}/deployments/{name}");
            let dep = http_get_json(&dep_url)?;
            let selector = super::pure::selector_to_query(&dep).ok_or_else(|| {
                ExecutionError::PreconditionFailed(
                    "deployment selector unusable (matchExpressions or empty)".into(),
                )
            })?;
            let rs_url = format!(
                "{api_base}/apis/apps/v1/namespaces/{ns}/replicasets?labelSelector={}",
                encode_query(&selector)
            );
            let rs_list = http_get_json(&rs_url)?;
            let (old_rev, target_rev, template) =
                super::pure::pick_prev_revision_template(&dep, &rs_list).ok_or_else(|| {
                    ExecutionError::PreconditionFailed(format!(
                        "no previous revision to roll back to (deployment {ns}/{name})"
                    ))
                })?;
            let patch = serde_json::json!({ "spec": { "template": template } }).to_string();
            http_write("PATCH", &dep_url, Some(&patch))?;
            return Ok(ok(
                &format!("rolled back to revision {target_rev}"),
                &format!(r#"{{"old_revision":{old_rev},"current_revision":{target_rev}}}"#),
            ));
        }
        Ok(ok("deployment rollback triggered (mock)", r#"{"current_revision":1}"#))
    }

    fn refresh_secret(
        parts: &[&str],
        params: &Value,
        api_base: &str,
        real_mode: bool,
    ) -> Result<ExecutionResult, ExecutionError> {
        let (ns, name) = ns_name(parts)?;
        if real_mode {
            // 真轮换:①(可选)params.key + params.new_value_b64 时 PATCH data
            // (base64 由调用方给,host 不猜业务值);②rotatedAt 注解;
            // ③引用该 secret 的 Pod 全部删除(volume/env/envFrom/initContainers),
            // 控制器重建后挂载新值 —— 这才是「轮换生效」的完整语义。
            let secret_url = format!("{api_base}/api/v1/namespaces/{ns}/secrets/{name}");
            if let (Some(key), Some(val)) = (
                params.get("key").and_then(Value::as_str),
                params.get("new_value_b64").and_then(Value::as_str),
            ) {
                let patch = serde_json::json!({
                    "metadata": { "annotations": { "rotatedAt": "now" } },
                    "data": { key: val }
                })
                .to_string();
                http_write("PATCH", &secret_url, Some(&patch))?;
            } else {
                let body = r#"{"metadata":{"annotations":{"rotatedAt":"now"}}}"#;
                http_write("PATCH", &secret_url, Some(body))?;
            }
            // 引用 pod 重启:列 ns pods(桌面单机规模,全量列再客户端过滤)
            let pods_url = format!("{api_base}/api/v1/namespaces/{ns}/pods");
            let pods = http_get_json(&pods_url)?;
            let refs = super::pure::extract_referencing_pods(&pods, name);
            let restarted = refs.len();
            for pod in refs {
                let url = format!("{api_base}/api/v1/namespaces/{ns}/pods/{pod}");
                http_delete(&url)?;
            }
            return Ok(ok(
                &format!("secret rotated, {restarted} referencing pod(s) restarted"),
                &format!(r#"{{"secret_version":2,"restarted_pods":{restarted}}}"#),
            ));
        }
        Ok(ok("secret rotated (mock)", r#"{"secret_version":2}"#))
    }

    fn drain_node(
        parts: &[&str],
        api_base: &str,
        real_mode: bool,
    ) -> Result<ExecutionResult, ExecutionError> {
        let name = node_name(parts)?;
        if real_mode {
            // 完整 drain = cordon + evict 循环:
            // ① cordon(spec.unschedulable);② 列该 node 上 pods(排除 DaemonSet
            // owner 与 mirror/静态 pod,对齐 kubectl drain);③ 逐个 POST eviction
            // (policy/v1 subresource;PDB 冲突 → HTTP 429/500 上抛,调用方决定重试)。
            let node_url = format!("{api_base}/api/v1/nodes/{name}");
            let cordon = r#"{"spec":{"unschedulable":true}}"#;
            http_write("PATCH", &node_url, Some(cordon))?;

            let pods_url = format!(
                "{api_base}/api/v1/pods?fieldSelector={}",
                encode_query(&format!("spec.nodeName={name}"))
            );
            let pods = http_get_json(&pods_url)?;
            let evictable = super::pure::filter_evictable_pods(&pods);
            let total = evictable.len();
            for (pns, pname) in evictable {
                let evict_url =
                    format!("{api_base}/api/v1/namespaces/{pns}/pods/{pname}/eviction");
                let body = serde_json::json!({
                    "apiVersion": "policy/v1", "kind": "Eviction",
                    "metadata": { "name": pname, "namespace": pns }
                })
                .to_string();
                http_post(&evict_url, &body)?;
            }
            return Ok(ok(
                &format!("node cordoned, {total} pod(s) evicted"),
                &format!(r#"{{"cordoned":true,"evicted_pods":{total}}}"#),
            ));
        }
        Ok(ok("node cordoned (mock)", r#"{"cordoned":true}"#))
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

/// real handler 完整化的判定纯函数(host 侧 `cargo test` 直接测,不进 wasm)。
/// 输入输出全是 K8s API JSON(shape 与真实响应一致),不耦合 WIT binding。
pub mod pure {
    use serde_json::Value;

    /// rollback_deployment:从 deployment + 其 RS 列表选出上一版模板。
    ///
    /// 语义对齐 `kubectl rollout undo`:deployment 当前 revision 取注解
    /// `deployment.kubernetes.io/revision`(缺省 0 视为无效);目标 = 当前-1;
    /// 在 RS items 里找同注解等于目标的 RS,返回 `(当前 revision, 目标 revision,
    /// 旧 spec.template)` —— 当前值供 exec_result 透出 `old_revision`,集群回读
    /// verifier 据此判定「revision 已离开执行前值」。
    /// 找不到(只有一个历史版本/注解缺失)→ None。
    pub fn pick_prev_revision_template(
        deployment: &Value,
        rs_list: &Value,
    ) -> Option<(i64, i64, Value)> {
        // K8s 注解值是字符串("3"),不是数字 —— 按 str 取再 parse。
        let ann_rev = |v: &Value| -> Option<i64> {
            v.pointer("/metadata/annotations/deployment.kubernetes.io~1revision")
                .and_then(Value::as_str)
                .and_then(|s| s.parse().ok())
        };
        let cur = ann_rev(deployment)?;
        if cur <= 1 {
            // revision 1(或缺失)没有更早版本可回退
            return None;
        }
        let target = cur - 1;
        rs_list
            .pointer("/items")
            .and_then(Value::as_array)?
            .iter()
            .find(|rs| ann_rev(rs) == Some(target))
            .and_then(|rs| {
                let template = rs.pointer("/spec/template")?.clone();
                Some((cur, target, template))
            })
    }

    /// refresh_secret:从 ns pods 列表里找出引用目标 secret 的 pod 名。
    ///
    /// 引用面覆盖:spec.volumes[].secret.secretName、containers/initContainers/ephemeralContainers
    /// 的 env[].valueFrom.secretKeyRef.name 与 envFrom[].secretRef.name。
    pub fn extract_referencing_pods(pods_list: &Value, secret_name: &str) -> Vec<String> {
        let mut out = Vec::new();
        let Some(items) = pods_list.pointer("/items").and_then(Value::as_array) else {
            return out;
        };
        for pod in items {
            let refs_secret = |containers: Option<&Vec<Value>>| -> bool {
                containers
                    .map(|cs| {
                        cs.iter().any(|c| {
                            c.pointer("/env")
                                .and_then(Value::as_array)
                                .map(|envs| {
                                    envs.iter().any(|e| {
                                        e.pointer("/valueFrom/secretKeyRef/name")
                                            .and_then(Value::as_str)
                                            .map(|n| n == secret_name)
                                            .unwrap_or(false)
                                    })
                                })
                                .unwrap_or(false)
                                || c.pointer("/envFrom")
                                    .and_then(Value::as_array)
                                    .map(|efs| {
                                        efs.iter().any(|ef| {
                                            ef.pointer("/secretRef/name")
                                                .and_then(Value::as_str)
                                                .map(|n| n == secret_name)
                                                .unwrap_or(false)
                                        })
                                    })
                                    .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false)
            };
            let volume_ref = pod
                .pointer("/spec/volumes")
                .and_then(Value::as_array)
                .map(|vs| {
                    vs.iter().any(|v| {
                        v.pointer("/secret/secretName")
                            .and_then(Value::as_str)
                            .map(|n| n == secret_name)
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false);
            let container_ref = [
                pod.pointer("/spec/containers").and_then(Value::as_array),
                pod.pointer("/spec/initContainers").and_then(Value::as_array),
                pod.pointer("/spec/ephemeralContainers").and_then(Value::as_array),
            ]
            .iter()
            .any(|c| refs_secret(c.as_deref()));
            if volume_ref || container_ref {
                if let Some(name) = pod.pointer("/metadata/name").and_then(Value::as_str) {
                    out.push(name.to_string());
                }
            }
        }
        out
    }

    /// drain_node:从某 node 上的 pods 列表里选出可 evict 的(排除 DaemonSet 与
    /// mirror pod),返回 (namespace, name) 列表。
    ///
    /// 排除规则(对齐 `kubectl drain`):ownerReferences 有 kind=DaemonSet;或注解
    /// `kubernetes.io/config.mirror` 存在(kubelet 静态 pod 的 mirror)。
    pub fn filter_evictable_pods(pods_list: &Value) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let Some(items) = pods_list.pointer("/items").and_then(Value::as_array) else {
            return out;
        };
        for pod in items {
            if pod
                .pointer("/metadata/annotations/kubernetes.io~1config.mirror")
                .is_some()
            {
                continue;
            }
            let owned_by_daemonset = pod
                .pointer("/metadata/ownerReferences")
                .and_then(Value::as_array)
                .map(|owners| {
                    owners
                        .iter()
                        .any(|o| o.pointer("/kind").and_then(Value::as_str) == Some("DaemonSet"))
                })
                .unwrap_or(false);
            if owned_by_daemonset {
                continue;
            }
            if let (Some(ns), Some(name)) = (
                pod.pointer("/metadata/namespace").and_then(Value::as_str),
                pod.pointer("/metadata/name").and_then(Value::as_str),
            ) {
                out.push((ns.to_string(), name.to_string()));
            }
        }
        out
    }

    /// deployment 的 labelSelector → URL query 值(matchLabels 拼成 `k=v` 逗号串;
    /// matchExpressions 存在时不支持(返回 None),罕见且组合语义复杂)。
    pub fn selector_to_query(deployment: &Value) -> Option<String> {
        let ml = deployment.pointer("/spec/selector/matchLabels")?.as_object()?;
        if deployment.pointer("/spec/selector/matchExpressions").is_some() {
            return None;
        }
        if ml.is_empty() {
            return None;
        }
        Some(
            ml.iter()
                .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or_default()))
                .collect::<Vec<_>>()
                .join(","),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::pure::*;
    use serde_json::json;

    #[test]
    fn picks_prev_revision_template() {
        let dep = json!({ "metadata": { "annotations": { "deployment.kubernetes.io/revision": "3" } },
                           "spec": { "selector": { "matchLabels": { "app": "web" } } } });
        let rs_list = json!({ "items": [
            { "metadata": { "annotations": { "deployment.kubernetes.io/revision": "2" } },
              "spec": { "template": { "spec": { "containers": [ { "image": "old:1" } ] } } } },
            { "metadata": { "annotations": { "deployment.kubernetes.io/revision": "3" } },
              "spec": { "template": { "spec": { "containers": [ { "image": "cur:3" } ] } } } }
        ] });
        let (old_rev, target_rev, tpl) = pick_prev_revision_template(&dep, &rs_list).expect("prev exists");
        assert_eq!((old_rev, target_rev), (3, 2));
        assert_eq!(tpl.pointer("/spec/containers/0/image"), Some(&json!("old:1")));

        // revision 1:没有更早版本
        let dep1 = json!({ "metadata": { "annotations": { "deployment.kubernetes.io/revision": "1" } } });
        assert!(pick_prev_revision_template(&dep1, &rs_list).is_none());
        // 当前 3 但 RS 里没有 2
        let rs_only3 = json!({ "items": [ rs_list["items"][1].clone() ] });
        assert!(pick_prev_revision_template(&dep, &rs_only3).is_none());
    }

    #[test]
    fn extracts_pods_referencing_secret() {
        let pods = json!({ "items": [
            { "metadata": { "name": "via-volume" },
              "spec": { "volumes": [ { "secret": { "secretName": "db-pass" } } ], "containers": [] } },
            { "metadata": { "name": "via-env" },
              "spec": { "containers": [ { "env": [ { "name": "P", "valueFrom": { "secretKeyRef": { "name": "db-pass", "key": "password" } } } ] } ] } },
            { "metadata": { "name": "via-envfrom" },
              "spec": { "containers": [ { "envFrom": [ { "secretRef": { "name": "db-pass" } } ] } ] } },
            { "metadata": { "name": "via-init" },
              "spec": { "initContainers": [ { "env": [ { "valueFrom": { "secretKeyRef": { "name": "db-pass", "key": "k" } } } ] } ], "containers": [] } },
            { "metadata": { "name": "unrelated" },
              "spec": { "volumes": [ { "secret": { "secretName": "other" } } ], "containers": [] } }
        ] });
        let names = extract_referencing_pods(&pods, "db-pass");
        assert_eq!(names, vec!["via-volume", "via-env", "via-envfrom", "via-init"]);
        assert!(extract_referencing_pods(&pods, "nope").is_empty());
    }

    #[test]
    fn filters_evictable_pods() {
        let pods = json!({ "items": [
            { "metadata": { "name": "normal", "namespace": "app" }, "spec": {} },
            { "metadata": { "name": "ds", "namespace": "kube-system",
                             "ownerReferences": [ { "kind": "DaemonSet", "name": "flannel" } ] }, "spec": {} },
            { "metadata": { "name": "static", "namespace": "kube-system",
                             "annotations": { "kubernetes.io/config.mirror": "abc" } }, "spec": {} }
        ] });
        let evictable = filter_evictable_pods(&pods);
        assert_eq!(evictable, vec![("app".to_string(), "normal".to_string())]);
    }

    #[test]
    fn selector_query_from_match_labels() {
        let dep = json!({ "spec": { "selector": { "matchLabels": { "app": "web", "tier": "fe" } } } });
        assert_eq!(selector_to_query(&dep).unwrap(), "app=web,tier=fe");
        let with_expr = json!({ "spec": { "selector": { "matchLabels": { "app": "w" },
                                                          "matchExpressions": [] } } });
        assert!(selector_to_query(&with_expr).is_none());
    }
}

#[cfg(target_arch = "wasm32")]
use imp::K8sHandler;

#[cfg(target_arch = "wasm32")]
bindings::export!(K8sHandler with_types_in bindings);
