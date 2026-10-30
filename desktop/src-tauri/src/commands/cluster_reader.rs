//! K8sClusterReader —— [`engine_recovery::ClusterReader`] 的 desktop 实装。
//!
//! real 模式 verifier 的集群回读通道:canonical resource_id → K8s API 路径,
//! 经 kubectl proxy(与 handler 同一 api_base)GET 资源当前态 JSON。
//! 与 `HandlerExecutor` 注入模式对齐:engine 只认 trait,I/O 留在最外层。
//!
//! 只在 `SRE_GRAPH_HANDLER_MODE=real` 且 k8s-handler 加载时构造(`lib.rs` setup);
//! mock 模式不挂(None = verifier 纯 twin 自报,演练语义)。

use serde_json::Value;

use engine_recovery::ClusterReader;

/// canonical resource_id → K8s API 路径。
///
/// - `deploy:{c}:{ns}:{n}` → `/apis/apps/v1/namespaces/{ns}/deployments/{n}`
/// - `pod:{c}:{ns}:{n}`    → `/api/v1/namespaces/{ns}/pods/{n}`
/// - `secret:{c}:{ns}:{n}` → `/api/v1/namespaces/{ns}/secrets/{n}`
/// - `svc:{c}:{ns}:{n}`    → `/api/v1/namespaces/{ns}/endpoints/{n}`
///   (restart_service 的验证对象是 Endpoints;Service 自身不受该动作影响)
/// - `node:{c}:{n}`        → `/api/v1/nodes/{n}`(node 无 namespace)
///
/// 其它前缀(mysql:/redis:/image: 等)→ Err(verifier 侧回落 twin)。
pub fn resource_id_to_api_path(resource_id: &str) -> Result<String, String> {
    let parts: Vec<&str> = resource_id.split(':').collect();
    let path = match parts.as_slice() {
        [deploy, _c, ns, n] if *deploy == "deploy" => {
            format!("/apis/apps/v1/namespaces/{ns}/deployments/{n}")
        }
        [pod, _c, ns, n] if *pod == "pod" => format!("/api/v1/namespaces/{ns}/pods/{n}"),
        [secret, _c, ns, n] if *secret == "secret" => {
            format!("/api/v1/namespaces/{ns}/secrets/{n}")
        }
        [svc, _c, ns, n] if *svc == "svc" => format!("/api/v1/namespaces/{ns}/endpoints/{n}"),
        [node, _c, n] if *node == "node" => format!("/api/v1/nodes/{n}"),
        _ => return Err(format!("resource_id '{resource_id}' maps to no K8s API path")),
    };
    Ok(path)
}

/// 经 kubectl proxy 回读集群资源的 reader。
pub struct K8sClusterReader {
    client: reqwest::Client,
    api_base: String,
}

impl K8sClusterReader {
    /// `api_base`:kubectl proxy 地址(与 handler executor 同源,`SRE_GRAPH_K8S_API_BASE`
    /// / manifest `[defaults].api_base` 的解析结果由 setup 传入)。
    pub fn new(api_base: &str) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("reqwest client"),
            api_base: api_base.trim_end_matches('/').to_string(),
        }
    }
}

#[async_trait::async_trait]
impl ClusterReader for K8sClusterReader {
    async fn get_resource_json(&self, resource_id: &str) -> Result<Value, String> {
        let path = resource_id_to_api_path(resource_id)?;
        let url = format!("{}{}", self.api_base, path);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("GET {url}: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("GET {url}: HTTP {}", status.as_u16()));
        }
        resp.json::<Value>()
            .await
            .map_err(|e| format!("GET {url}: decode json: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[test]
    fn maps_all_supported_prefixes() {
        assert_eq!(
            resource_id_to_api_path("deploy:vm:otel-demo:frontend").unwrap(),
            "/apis/apps/v1/namespaces/otel-demo/deployments/frontend"
        );
        assert_eq!(
            resource_id_to_api_path("pod:vm:otel-demo:frontend-abc").unwrap(),
            "/api/v1/namespaces/otel-demo/pods/frontend-abc"
        );
        assert_eq!(
            resource_id_to_api_path("secret:vm:otel-demo:db-pass").unwrap(),
            "/api/v1/namespaces/otel-demo/secrets/db-pass"
        );
        // svc → endpoints(restart_service 的验证对象)
        assert_eq!(
            resource_id_to_api_path("svc:vm:otel-demo:web").unwrap(),
            "/api/v1/namespaces/otel-demo/endpoints/web"
        );
        // node 无 namespace
        assert_eq!(
            resource_id_to_api_path("node:vm:worker-1").unwrap(),
            "/api/v1/nodes/worker-1"
        );
    }

    #[test]
    fn rejects_non_k8s_prefixes() {
        assert!(resource_id_to_api_path("mysql:vm:otel-demo:db").is_err());
        assert!(resource_id_to_api_path("image:vm:otel-demo:nginx:1.2").is_err());
        assert!(resource_id_to_api_path("garbage").is_err());
    }

    #[tokio::test]
    async fn reader_gets_json_from_local_server() {
        // 最小 HTTP server 回固定 JSON,验证 GET 拼 URL + 解码
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf).await;
                let body = r#"{"spec":{"replicas":3}}"#;
                let resp = format!(
                    "HTTP/1.0 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(resp.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
        let reader = K8sClusterReader::new(&format!("http://127.0.0.1:{port}"));
        let v = reader
            .get_resource_json("deploy:c:ns:api")
            .await
            .expect("readback ok");
        assert_eq!(v.pointer("/spec/replicas"), Some(&serde_json::json!(3)));
        let _ = handle.await;
    }

    #[tokio::test]
    async fn reader_404_is_err() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf).await;
                let resp = "HTTP/1.0 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = socket.write_all(resp.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
        let reader = K8sClusterReader::new(&format!("http://127.0.0.1:{port}"));
        let err = reader
            .get_resource_json("pod:c:ns:gone")
            .await
            .expect_err("404 must be Err");
        assert!(err.contains("404"), "err: {err}");
        let _ = handle.await;
    }
}
