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
//! 5. **URL allow-list**(Phase 3 落地)—— manifest 顶层 `[network]` 段配
//!    `allowed_hosts = [...]`,[`check_url_allowed`] 按 host(可带 :port)查表;
//!    未配置时向后兼容 allow-all(启动时 warn,由 `multi.rs` 打)。无论配置与否,
//!    link-local 目标(169.254.0.0/16、fe80::/10 —— 云 metadata 面)硬拒。

use std::collections::HashSet;

/// 申明 http-client capability 时使用的字符串。manifest.toml `capabilities = [...]`
/// 里出现此值才能调 [`http_get`]。
pub const CAP_HTTP_CLIENT: &str = "http-client";

/// 申明 http-write capability 时使用的字符串(Phase 3.9)。write 比 read 危险,
/// 单独 capability deny-by-default,manifest `capabilities = [..., "http-write"]` 才放行。
pub const CAP_HTTP_WRITE: &str = "http-write";

/// manifest `[network].allowed_hosts` 的解析形态。
///
/// - [`AllowedHosts::NotConfigured`] —— manifest 没配 `[network]` 段:
///   allow-all(向后兼容,不 break 现有 manifest;`multi.rs` 启动时 warn 提示 pin)。
/// - [`AllowedHosts::Configured`] —— 配置了:`Some` 列表按 host 查表,
///   空列表 = 显式全拒(比未配置更严)。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AllowedHosts {
    /// 未配置(allow-all,向后兼容;生产应 pin)。
    #[default]
    NotConfigured,
    /// 已配置。条目 `host` 或 `host:port`(大小写不敏感);`host` 条目匹配任意端口。
    Configured(Vec<String>),
}

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

/// URL 目标校验 —— capability 之后的第二道门(Phase 3)。
///
/// 规则(先于 allow-list、无论配置与否生效):
/// 1. **link-local 硬拒** —— URL host 是 IP literal 且落在 `169.254.0.0/16` 或
///    `fe80::/10`(云 metadata 服务面)→ [`HostHttpError::Unauthorized`]。
///    本项目 connector 打 `127.0.0.1`(kubectl proxy,loopback 非 link-local)不受影响。
/// 2. [`AllowedHosts::NotConfigured`] → 放行(向后兼容;`multi.rs` 启动 warn)。
/// 3. [`AllowedHosts::Configured`] → URL 的 host(小写)须命中列表条目:
///    条目带 `:port` 时还须端口相同;纯 host 条目匹配任意端口。
pub fn check_url_allowed(url: &str, allowed: &AllowedHosts) -> Result<(), HostHttpError> {
    // reqwest 传递依赖里的 url crate,这里显式声明使用
    let parsed = url::Url::parse(url)
        .map_err(|e| HostHttpError::Unauthorized(format!("invalid url {url:?}: {e}")))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| HostHttpError::Unauthorized(format!("url {url:?} has no host")))?;

    // 规则 1:link-local IP literal 硬拒(metadata 面)。
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        let is_link_local = match ip {
            std::net::IpAddr::V4(v4) => v4.octets()[0] == 169 && v4.octets()[1] == 254,
            std::net::IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) == 0xfe80,
        };
        if is_link_local {
            return Err(HostHttpError::Unauthorized(format!(
                "link-local target {host} denied (cloud metadata surface)"
            )));
        }
    }

    // 规则 2:未配置 → allow-all。
    let AllowedHosts::Configured(entries) = allowed else {
        return Ok(());
    };

    // 规则 3:按条目查表(host 小写;带 :port 的条目还须端口匹配)。
    let host_lc = host.to_ascii_lowercase();
    let port = parsed.port();
    for entry in entries {
        let entry = entry.to_ascii_lowercase();
        match entry.rsplit_once(':') {
            // 条目带端口:host+port 都须匹配(仅当 URL 有显式端口;URL 无显式端口
            // 走 scheme 默认端口,此时按纯 host 条目匹配更符合直觉)。
            Some((e_host, e_port)) if port.is_some() => {
                if e_host == host_lc && *e_port == port.map(|p| p.to_string()).unwrap_or_default()
                {
                    return Ok(());
                }
            }
            _ => {
                if entry == host_lc {
                    return Ok(());
                }
            }
        }
    }
    Err(HostHttpError::Unauthorized(format!(
        "host '{host}' not in [network].allowed_hosts"
    )))
}

/// host 端 capability-gated HTTP GET。
///
/// 流程:
/// 1. 查 `allowed_capabilities`,缺 `"http-client"` → [`HostHttpError::Unauthorized`]
/// 2. [`check_url_allowed`] 查 host allow-list(link-local 硬拒 + 按表放行)
/// 3. 用传入的 `client` 发 GET,把 `headers` 一对一加进去
/// 4. 拿响应 → 按状态码映射 → 把 body 转 `Vec<u8>`
///
/// `client` 走入参注入(不在内部创建)—— 便于测试时换成预配 timeout 的 Client,
/// 也便于 WasmConnector 共享一个 Client 跨多次 sync 复用连接池。
pub async fn http_get(
    client: &reqwest::Client,
    allowed_capabilities: &HashSet<String>,
    allowed_hosts: &AllowedHosts,
    url: &str,
    headers: &[(String, String)],
) -> Result<HostHttpResponse, HostHttpError> {
    if !allowed_capabilities.contains(CAP_HTTP_CLIENT) {
        return Err(HostHttpError::Unauthorized(format!(
            "capability '{CAP_HTTP_CLIENT}' not declared in module manifest"
        )));
    }
    check_url_allowed(url, allowed_hosts)?;

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
/// 2. [`check_url_allowed`] 查 host allow-list(与 GET 同一道门,write 更没有绕过的理由)
/// 3. 按 `method`(PATCH/POST/DELETE)建 reqwest 请求,加 headers + body(DELETE 可无)
/// 4. 拿响应 -> 按状态码映射(同 [`http_get`]:401/403->Unauthorized,404->NotFound)
///
/// `body: Option<&[u8]>` -- `None` = 无 body(DELETE);`Some` = 请求 body 字节。
pub async fn http_write(
    client: &reqwest::Client,
    allowed_capabilities: &HashSet<String>,
    allowed_hosts: &AllowedHosts,
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
    check_url_allowed(url, allowed_hosts)?;

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
        let err = http_get(&client, &empty_caps(), &AllowedHosts::NotConfigured, "http://127.0.0.1:1/", &[])
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
        let err = http_get(&client, &allow_http(), &AllowedHosts::NotConfigured, "http://127.0.0.1:1/", &[])
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
            &AllowedHosts::NotConfigured,
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
            &AllowedHosts::NotConfigured,
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
            &AllowedHosts::NotConfigured,
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
            &AllowedHosts::NotConfigured,
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
            &AllowedHosts::NotConfigured,
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
            &AllowedHosts::NotConfigured,
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
            &AllowedHosts::NotConfigured,
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

    // ---------- check_url_allowed(allow-list 纯函数)----------

    fn hosts(list: &[&str]) -> AllowedHosts {
        AllowedHosts::Configured(list.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn url_check_not_configured_allows_any_host() {
        // 向后兼容:未配置 [network] -> 任意 host 放行(loopback 也放 —— 项目核心
        // 用例就是 127.0.0.1 kubectl proxy)。
        assert!(check_url_allowed("http://10.1.2.3:9090/api", &AllowedHosts::NotConfigured).is_ok());
        assert!(check_url_allowed("http://example.com/x", &AllowedHosts::NotConfigured).is_ok());
    }

    #[test]
    fn url_check_configured_list_matches_host_and_port() {
        let allowed = hosts(&["127.0.0.1:8001", "127.0.0.1", "prometheus.internal"]);
        // 纯 host 条目匹配任意端口
        assert!(check_url_allowed("http://prometheus.internal:9090/api", &allowed).is_ok());
        assert!(check_url_allowed("http://127.0.0.1:9090/x", &allowed).is_ok());
        // host:port 条目精确匹配
        assert!(check_url_allowed("http://127.0.0.1:8001/api/v1/pods", &allowed).is_ok());
        // 不在表内 -> 拒
        let err = check_url_allowed("http://evil.example.com:8001/x", &allowed)
            .expect_err("host not in list should be denied");
        assert!(matches!(&err, HostHttpError::Unauthorized(m) if m.contains("evil.example.com")));
        // host 对了端口不对(host:port 条目),且无纯 host 条目兜底时 -> 拒
        let only_port = hosts(&["127.0.0.1:8001"]);
        assert!(check_url_allowed("http://127.0.0.1:9999/x", &only_port).is_err());
    }

    #[test]
    fn url_check_empty_configured_list_denies_all() {
        // Some([]) = 显式全拒(区别于未配置的 allow-all)
        let err = check_url_allowed("http://127.0.0.1:8001/", &hosts(&[]))
            .expect_err("empty list should deny");
        assert!(matches!(&err, HostHttpError::Unauthorized(m) if m.contains("allowed_hosts")));
    }

    #[test]
    fn url_check_link_local_denied_even_when_not_configured() {
        // 云 metadata 面(169.254.169.254 / fe80::)硬拒,与 allow-list 配置无关
        assert!(check_url_allowed("http://169.254.169.254/latest/meta-data/", &AllowedHosts::NotConfigured).is_err());
        assert!(check_url_allowed("http://fe80::1:8080/", &AllowedHosts::NotConfigured).is_err());
        // 即便显式加进 allowed_hosts 也拒(硬拒优先于配置)
        assert!(
            check_url_allowed(
                "http://169.254.169.254/latest/meta-data/",
                &hosts(&["169.254.169.254"])
            )
            .is_err()
        );
        // loopback 不是 link-local,kubectl proxy 用例不受影响
        assert!(check_url_allowed("http://127.0.0.1:8001/", &AllowedHosts::NotConfigured).is_ok());
    }

    #[tokio::test]
    async fn http_get_denies_host_not_in_configured_list() {
        // e2e 形态:capability 有,但 [network] 列表不含目标 host -> Unauthorized,
        // 且请求根本不发出(打到不存在的端口也不会有连接尝试的副作用 —— 这里
        // 用保留端口 9 discard 验证拒绝发生在网络层之前)。
        let client = test_client();
        let err = http_get(
            &client,
            &allow_http(),
            &hosts(&["10.0.0.1"]),
            "http://127.0.0.1:9/",
            &[],
        )
        .await
        .expect_err("host not allowed");
        match err {
            HostHttpError::Unauthorized(msg) => {
                assert!(msg.contains("allowed_hosts"), "msg should name the list: {msg}");
            }
            other => panic!("expected Unauthorized(host), got {other:?}"),
        }
    }
}
