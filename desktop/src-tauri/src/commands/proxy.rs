//! kubectl proxy 生命周期托管(Phase 2.7 part 2)。
//!
//! k8s connector 走本地 `kubectl proxy`(明文 HTTP,TLS+认证留 proxy/kubeconfig)。
//! 本模块让 desktop **托管** proxy 进程:启动(就绪探测后返回)+ 停止 + 状态查询,
//! 用户不再手开终端跑 `kubectl proxy`。
//!
//! ## 实现要点
//!
//! - 用 `tokio::process::Command` 起 `kubectl proxy --port=8001 --address=127.0.0.1`,
//!   子进程句柄存 `AppState.proxy`(`std::sync::Mutex<Option<Child>>`,便于在
//!   `RunEvent::Exit` 同步回调里 kill)。`kill_on_drop(true)` 兜底防孤儿。
//! - 就绪探测:`tokio::net::TcpStream::connect` 轮询 127.0.0.1:port -- kubectl proxy
//!   一旦 bind 即 accept,探测通过即返回。探测失败 -> 杀刚起的 child + 返错。
//! - **不碰凭据 / 不加 capability**:TLS+认证全留 kubectl + kubeconfig,Rust 端只
//!   spawn 进程;WASM connector 仍只发 HTTP GET(对照 doc CLAUDE.md kubectl proxy 架构)。
//! - kubectl 路径解析:`KUBECTL_PATH` env > PATH 扫描 > 常见绝对路径(/snap/bin/kubectl 等)。

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tauri::State;
use tokio::process::Command;

use crate::AppState;

/// 默认 proxy 端口(与 manifest.toml k8s config api_base 对齐)。
pub const DEFAULT_PORT: u16 = 8001;
const READINESS_TIMEOUT: Duration = Duration::from_secs(10);
const READINESS_POLL: Duration = Duration::from_millis(200);

/// proxy 状态(给前端展示)。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProxyStatusDto {
    /// 是否在跑。
    pub running: bool,
    /// 端口。
    pub port: u16,
    /// `http://127.0.0.1:{port}`(未跑时空)。
    pub api_base: String,
    /// 子进程 pid(未跑时 None)。
    pub pid: Option<u32>,
    /// 人读消息。
    pub message: String,
}

/// 启动 kubectl proxy(已跑则返回当前状态)。
///
/// `port` 缺省 8001。就绪后子进程句柄存 `AppState.proxy`,前端可随后 `sync_all_now`
/// 让 k8s connector 经 `api_base` 拉真集群拓扑。
#[tauri::command]
pub async fn start_kubectl_proxy(
    state: State<'_, AppState>,
    port: Option<u16>,
) -> Result<ProxyStatusDto, String> {
    let port = port.unwrap_or(DEFAULT_PORT);

    // 已在跑 -> 返回当前状态(不重复起)
    {
        let guard = state.proxy.lock().map_err(|e| e.to_string())?;
        if let Some(child) = guard.as_ref() {
            return Ok(ProxyStatusDto {
                running: true,
                port,
                api_base: format!("http://127.0.0.1:{port}"),
                pid: child.id(),
                message: "proxy already running".into(),
            });
        }
    }

    let kubectl = resolve_kubectl()?;
    let mut child = Command::new(&kubectl)
        .arg("proxy")
        .arg("--port")
        .arg(port.to_string())
        .arg("--address=127.0.0.1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("spawn kubectl proxy ({}): {e}", kubectl.display()))?;
    let pid = child.id();

    // 就绪探测(不持锁)。失败 -> 杀刚起的 child,不留孤儿。
    if let Err(e) = wait_for_proxy(port, READINESS_TIMEOUT, READINESS_POLL).await {
        let _ = child.start_kill();
        return Err(e);
    }

    *state.proxy.lock().map_err(|e| e.to_string())? = Some(child);
    Ok(ProxyStatusDto {
        running: true,
        port,
        api_base: format!("http://127.0.0.1:{port}"),
        pid,
        message: format!("kubectl proxy ready at http://127.0.0.1:{port}"),
    })
}

/// 停止 kubectl proxy(SIGKILL 子进程 + reap)。
#[tauri::command]
pub async fn stop_kubectl_proxy(state: State<'_, AppState>) -> Result<ProxyStatusDto, String> {
    // take 出 child 后立刻释放锁,不在持锁期间 await(guard 非 Send,command future 要 Send)。
    let child_opt = {
        let mut guard = state.proxy.lock().map_err(|e| e.to_string())?;
        guard.take()
    };
    if let Some(mut child) = child_opt {
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    Ok(ProxyStatusDto {
        running: false,
        port: DEFAULT_PORT,
        api_base: String::new(),
        pid: None,
        message: "proxy stopped".into(),
    })
}

/// 查询 proxy 状态。顺带 reap 已退出的子进程(`try_wait`)。
#[tauri::command]
pub async fn proxy_status(state: State<'_, AppState>) -> Result<ProxyStatusDto, String> {
    let mut guard = state.proxy.lock().map_err(|e| e.to_string())?;
    let running = match guard.as_mut() {
        Some(child) => match child.try_wait() {
            Ok(Some(_)) => {
                // 已退出 -> 收尸
                *guard = None;
                false
            }
            Ok(None) => true,
            Err(_) => false,
        },
        None => false,
    };
    let port = DEFAULT_PORT;
    Ok(ProxyStatusDto {
        running,
        port,
        api_base: if running {
            format!("http://127.0.0.1:{port}")
        } else {
            String::new()
        },
        pid: guard.as_ref().and_then(|c| c.id()),
        message: if running {
            "running".into()
        } else {
            "not running".into()
        },
    })
}

/// 找 kubectl 可执行文件:`KUBECTL_PATH` env -> PATH 扫描 -> 常见绝对路径。
fn resolve_kubectl() -> Result<PathBuf, String> {
    let env_val = std::env::var("KUBECTL_PATH").ok();
    let path_dirs: Vec<String> = std::env::var("PATH")
        .ok()
        .map(|p| p.split(':').map(str::to_string).collect())
        .unwrap_or_default();
    let path_refs: Vec<&str> = path_dirs.iter().map(String::as_str).collect();
    resolve_kubectl_from(
        env_val.as_deref(),
        &path_refs,
        &["/snap/bin/kubectl", "/usr/bin/kubectl", "/usr/local/bin/kubectl"],
    )
}

/// `resolve_kubectl` 的纯逻辑核心(可测):env > path_dirs 扫描 > common 绝对路径。
fn resolve_kubectl_from(
    env_val: Option<&str>,
    path_dirs: &[&str],
    common: &[&str],
) -> Result<PathBuf, String> {
    if let Some(p) = env_val {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Ok(pb);
        }
    }
    for d in path_dirs {
        let cand = PathBuf::from(d).join("kubectl");
        if cand.is_file() {
            return Ok(cand);
        }
    }
    for c in common {
        let pb = PathBuf::from(c);
        if pb.is_file() {
            return Ok(pb);
        }
    }
    Err("kubectl not found on PATH; set KUBECTL_PATH or install kubectl".into())
}

/// 轮询 TCP 连接到 127.0.0.1:port 直到成功或超时。
///
/// kubectl proxy bind 后即 accept,探测通过即视为就绪。
async fn wait_for_proxy(port: u16, timeout: Duration, poll: Duration) -> Result<(), String> {
    let addr = format!("127.0.0.1:{port}");
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if tokio::net::TcpStream::connect(&addr).await.is_ok() {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "kubectl proxy did not become ready on 127.0.0.1:{port} within {timeout:?}"
            ));
        }
        tokio::time::sleep(poll).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(dir: &std::path::Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, b"#!/bin/sh\n").expect("write fake bin");
        p
    }

    #[test]
    fn resolve_kubectl_from_finds_env_path() {
        let dir = std::env::temp_dir().join(format!("sre-proxy-test-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("mkdir");
        touch(&dir, "kubectl");
        // env_val 优先
        let env_path = dir.join("kubectl");
        let got = resolve_kubectl_from(Some(env_path.to_str().unwrap()), &[], &[]);
        assert_eq!(got.unwrap(), env_path);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_kubectl_from_finds_in_path_dirs() {
        let dir = std::env::temp_dir().join(format!("sre-proxy-test-path-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("mkdir");
        touch(&dir, "kubectl");
        let got = resolve_kubectl_from(None, &[dir.to_str().unwrap()], &[]);
        assert_eq!(got.unwrap(), dir.join("kubectl"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_kubectl_from_errors_when_missing() {
        let got = resolve_kubectl_from(None, &["/nonexistent-dir-xyz"], &["/nope/kubectl"]);
        assert!(got.is_err());
    }

    #[tokio::test]
    async fn wait_for_proxy_succeeds_when_listening() {
        // 起一个真 TCP listener,探测应立即通过
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let res = wait_for_proxy(port, Duration::from_secs(1), Duration::from_millis(50)).await;
        assert!(res.is_ok());
        drop(listener);
    }

    #[tokio::test]
    async fn wait_for_proxy_times_out_when_closed() {
        // port 1:特权端口,非 root 一律拒绝 -> 探测必失败,短超时快速返回 Err
        let res = wait_for_proxy(1, Duration::from_millis(150), Duration::from_millis(50)).await;
        assert!(res.is_err());
    }
}
