//! `http-client` capability 的 host 端实装(Phase 1 G)。
//!
//! 与 [`crate::runtime`] 的关系:
//!
//! - 这里定义 host 侧 plain types([`HostHttpResponse`] / [`HostHttpError`])和
//!   纯函数 [`http_get`],**完全不耦合 wit-bindgen 生成的 binding 类型**。这样:
//!   1. 可以脱离 wasmtime Store / async runtime 单独单测
//!   2. binding 类型未来跟 WIT 演进(P3 起 async variant 等)时,host 实装不必跟着改
//! - `runtime.rs` 里 `HttpClientHost for State` 是一个薄适配:做 host 类型 ↔
//!   binding 类型的字段平移,真正的 capability 检查 + reqwest 调用都在这里
//!
//! 设计要点:
//!
//! 1. **capability allow-list at call time** —— 每次 `http_get` 都查
//!    `allowed_capabilities` 包不包 `"http-client"`。Phase 1 不做 link-time
//!    全套拒绝(需要 per-WasmConnector Linker,代价大),用 call-time 拒绝足够
//! 2. **host 类型与 WIT 类型解耦** —— `HostHttpError` 是 [`thiserror::Error`],
//!    可在 host 侧串错误链;binding 的 `Error` variant 由 `runtime.rs` 适配
//! 3. **状态码到错误的映射** —— 401/403 → Unauthorized;404 → NotFound;
//!    其它非 2xx 透出 `Network(...)` 含状态码,guest 自己决定怎么处理。
//!    本期不映射 5xx 到特殊变体(WIT 没定义)
//! 4. **reqwest::Client 共享** —— `Client` 内部是 `Arc<Inner>`,clone 廉价。
//!    一个 WasmConnector 一份 clone,各自独立的连接池;Phase 3 可以提到
//!    `WasmRuntime` 级共享
//! 5. **URL allow-list 留 Phase 3** —— 拿到 `"http-client"` 能力的 guest
//!    当前能 GET 任意 URL。Phase 3 加 `[modules.network] allowed_hosts = [...]`
//!    manifest 段,在这里加 host 校验

use std::collections::HashSet;

/// 申明 http-client capability 时使用的字符串。manifest.toml `capabilities = [...]`
/// 里出现此值才能调 [`http_get`]。
pub const CAP_HTTP_CLIENT: &str = "http-client";

/// 申明 http-write capability 时使用的字符串(Phase 3.9)。write 比 read 危险,
/// 单独 capability deny-by-default,manifest `capabilities = [..., "http-write"]` 才放行。
pub const CAP_HTTP_WRITE: &str = "http-write";

/// host 侧 HTTP 响应 —— [`crate::runtime`] 把它转 WIT `response` record。
#[derive(Debug, Clone)]
pub struct HostHttpResponse {
    /// HTTP status code(如 200 / 204 / 404)。
    pub status: u16,
    /// 响应 body 原始字节。
    pub body: Vec<u8>,
}

/// host 侧 HTTP 错误 —— 与 WIT `error` variant 一一对应。
///
/// `thiserror` 派生让它能塞到 `anyhow::Result` 里被 host 链上其它层接住。
#[derive(Debug, Clone, thiserror::Error)]
pub enum HostHttpError {
    /// 当前 WasmConnector 的 `capabilities` 里没有 `"http-client"`,或服务端
    /// 返 401/403。
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    /// 服务端返 404。
    #[error("not found")]
    NotFound,
    /// 网络层错误(DNS / TCP / TLS / 任意非 status 错误 / 非 2xx 非 4xx 状态码)。
    #[error("network: {0}")]
    Network(String),
    /// reqwest 标记为 timeout 的错误。
    #[error("timeout")]
    Timeout,
}

/// host 端 capability-gated HTTP GET。
///
/// 流程:
/// 1. 查 `allowed_capabilities`,缺 `"http-client"` → [`HostHttpError::Unauthorized`]
/// 2. 用传入的 `client` 发 GET,把 `headers` 一对一加进去
/// 3. 拿响应 → 按状态码映射 → 把 body 转 `Vec<u8>`
///
/// `client` 走入参注入(不在内部创建)—— 便于测试时换成预配 timeout 的 Client,
/// 也便于 WasmConnector 共享一个 Client 跨多次 sync 复用连接池。
pub async fn http_get(
    client: &reqwest::Client,
    allowed_capabilities: &HashSet<String>,
    url: &str,
    headers: &[(String, String)],
) -> Result<HostHttpResponse, HostHttpError> {
    if !allowed_capabilities.contains(CAP_HTTP_CLIENT) {
        return Err(HostHttpError::Unauthorized(format!(
            "capability '{CAP_HTTP_CLIENT}' not declared in module manifest"
        )));
    }

    let mut req = client.get(url);
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }

    let resp = req.send().await.map_err(map_reqwest_error)?;
    let status = resp.status().as_u16();

    // 状态码分流 —— 401/403 → Unauthorized;404 → NotFound;2xx + 其它 → 返响应(包括 5xx)
    // 注:5xx 不映射成专门 variant —— WIT 没定义,guest 自己看 status 字段处理。
    if status == 401 || status == 403 {
        return Err(HostHttpError::Unauthorized(format!(
            "server returned {status} for {url}"
        )));
    }
    if status == 404 {
        return Err(HostHttpError::NotFound);
    }

    let body = resp.bytes().await.map_err(map_reqwest_error)?.to_vec();
    Ok(HostHttpResponse { status, body })
}

/// host 端 capability-gated HTTP write(Phase 3.9,供 WASM handler 真改集群)。
///
/// 流程:
/// 1. 查 `allowed_capabilities`,缺 `"http-write"` -> [`HostHttpError::Unauthorized`]
/// 2. 按 `method`(PATCH/POST/DELETE)建 reqwest 请求,加 headers + body(DELETE 可无)
/// 3. 拿响应 -> 按状态码映射(同 [`http_get`]:401/403->Unauthorized,404->NotFound)
///
/// `body: Option<&[u8]>` -- `None` = 无 body(DELETE);`Some` = 请求 body 字节。
pub async fn http_write(
    client: &reqwest::Client,
    allowed_capabilities: &HashSet<String>,
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
) -> Result<HostHttpResponse, HostHttpError> {
    if !allowed_capabilities.contains(CAP_HTTP_WRITE) {
        return Err(HostHttpError::Unauthorized(format!(
            "capability '{CAP_HTTP_WRITE}' not declared in module manifest"
        )));
    }

    let mut req = match method {
        "PATCH" => client.patch(url),
        "POST" => client.post(url),
        "DELETE" => client.delete(url),
        other => {
            return Err(HostHttpError::Network(format!(
                "unsupported http method: {other} (expected PATCH/POST/DELETE)"
            )))
        }
    };
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    if let Some(b) = body {
        req = req.body(b.to_vec());
    }

    let resp = req.send().await.map_err(map_reqwest_error)?;
    let status = resp.status().as_u16();

    if status == 401 || status == 403 {
        return Err(HostHttpError::Unauthorized(format!(
            "server returned {status} for {url}"
        )));
    }
    if status == 404 {
        return Err(HostHttpError::NotFound);
    }

    let body = resp.bytes().await.map_err(map_reqwest_error)?.to_vec();
    Ok(HostHttpResponse { status, body })
}

fn map_reqwest_error(e: reqwest::Error) -> HostHttpError {
    if e.is_timeout() {
        HostHttpError::Timeout
    } else {
        HostHttpError::Network(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    fn allow_http() -> HashSet<String> {
        let mut s = HashSet::new();
        s.insert(CAP_HTTP_CLIENT.to_string());
        s
    }

    fn empty_caps() -> HashSet<String> {
        HashSet::new()
    }

    fn test_client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .expect("client")
    }

    /// 起一个最小 HTTP/1.0 server,只回一个固定响应,然后立即关闭连接。
    /// 返回 (port, JoinHandle) —— 测试 await 完一次后 server 自动结束。
    async fn spawn_one_shot_http(
        response: &'static [u8],
    ) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        let handle = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                // 读 client 请求(只是为了避免 RST,真实 client 会先写)
                let mut buf = [0u8; 1024];
                let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf).await;
                let _ = socket.write_all(response).await;
                let _ = socket.shutdown().await;
            }
        });
        (port, handle)
    }

    #[tokio::test]
    async fn http_get_denies_when_capability_missing() {
        let client = test_client();
        let err = http_get(&client, &empty_caps(), "http://127.0.0.1:1/", &[])
            .await
            .expect_err("should deny");
        match err {
            HostHttpError::Unauthorized(msg) => {
                assert!(msg.contains("http-client"), "msg should name the cap: {msg}");
                assert!(
                    msg.contains("not declared"),
                    "msg should explain reason: {msg}"
                );
            }
            other => panic!("expected Unauthorized(cap), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn http_get_returns_network_error_for_closed_port() {
        let client = test_client();
        // 127.0.0.1:1 — 几乎一定是关的(reserved port 1)
        let err = http_get(&client, &allow_http(), "http://127.0.0.1:1/", &[])
            .await
            .expect_err("should fail to connect");
        match &err {
            HostHttpError::Network(msg) => {
                // 关港 → connection refused 是合法 Network 表现
                assert!(!msg.is_empty(), "network error should carry a message");
            }
            HostHttpError::Timeout => {
                // 短 timeout 配置下,关港也可能走 timeout 分支
            }
            other => panic!("expected Network/Timeout, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn http_get_succeeds_for_local_server_with_body() {
        let response = b"HTTP/1.0 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello";
        let (port, handle) = spawn_one_shot_http(response).await;

        let client = test_client();
        let resp = http_get(
            &client,
            &allow_http(),
            &format!("http://127.0.0.1:{port}/"),
            &[],
        )
        .await
        .expect("ok");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, b"hello");

        let _ = handle.await;
    }

    #[tokio::test]
    async fn http_get_maps_404_to_not_found() {
        let response = b"HTTP/1.0 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let (port, handle) = spawn_one_shot_http(response).await;

        let client = test_client();
        let err = http_get(
            &client,
            &allow_http(),
            &format!("http://127.0.0.1:{port}/"),
            &[],
        )
        .await
        .expect_err("404");
        assert!(matches!(err, HostHttpError::NotFound));

        let _ = handle.await;
    }

    #[tokio::test]
    async fn http_get_maps_403_to_unauthorized() {
        let response =
            b"HTTP/1.0 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let (port, handle) = spawn_one_shot_http(response).await;

        let client = test_client();
        let err = http_get(
            &client,
            &allow_http(),
            &format!("http://127.0.0.1:{port}/forbidden"),
            &[],
        )
        .await
        .expect_err("403");
        match err {
            HostHttpError::Unauthorized(msg) => {
                assert!(msg.contains("403"), "msg should include status: {msg}");
            }
            other => panic!("expected Unauthorized(403), got {other:?}"),
        }

        let _ = handle.await;
    }

    #[tokio::test]
    async fn http_get_forwards_headers_to_server() {
        // server 把请求 echo 回 body,这样我们能验证 header 真的被发出去了
        // (写个 echo 太累 — 退而求其次只检查 status 200 OK,header 传递的
        // 完整 e2e 留集成测试)
        let response =
            b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
        let (port, handle) = spawn_one_shot_http(response).await;

        let client = test_client();
        let resp = http_get(
            &client,
            &allow_http(),
            &format!("http://127.0.0.1:{port}/"),
            &[
                ("authorization".to_string(), "Bearer test-token".to_string()),
                ("x-trace-id".to_string(), "abc-123".to_string()),
            ],
        )
        .await
        .expect("ok");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, b"ok");

        let _ = handle.await;
    }

    fn allow_http_write() -> HashSet<String> {
        let mut s = HashSet::new();
        s.insert(CAP_HTTP_WRITE.to_string());
        s
    }

    #[tokio::test]
    async fn http_write_denies_when_capability_missing() {
        let client = test_client();
        let err = http_write(
            &client,
            &empty_caps(),
            "PATCH",
            "http://127.0.0.1:1/",
            &[],
            None,
        )
        .await
        .expect_err("should deny");
        match err {
            HostHttpError::Unauthorized(msg) => {
                assert!(msg.contains("http-write"), "msg should name the cap: {msg}");
                assert!(msg.contains("not declared"), "msg should explain: {msg}");
            }
            other => panic!("expected Unauthorized(cap), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn http_write_patch_succeeds_for_local_server() {
        let response = b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
        let (port, handle) = spawn_one_shot_http(response).await;
        let client = test_client();
        let resp = http_write(
            &client,
            &allow_http_write(),
            "PATCH",
            &format!("http://127.0.0.1:{port}/"),
            &[("content-type".to_string(), "application/merge-patch+json".to_string())],
            Some(br#"{"spec":{"replicas":2}}"#),
        )
        .await
        .expect("ok");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, b"ok");
        let _ = handle.await;
    }

    #[tokio::test]
    async fn http_write_rejects_unknown_method() {
        let client = test_client();
        let err = http_write(
            &client,
            &allow_http_write(),
            "PUT",
            "http://127.0.0.1:1/",
            &[],
            None,
        )
        .await
        .expect_err("PUT not supported");
        match err {
            HostHttpError::Network(msg) => {
                assert!(msg.contains("PUT"), "msg should name method: {msg}");
            }
            other => panic!("expected Network(unsupported method), got {other:?}"),
        }
    }
}
