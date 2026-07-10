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
//! - 就绪探测:`wait_for_proxy_or_exit` 轮询 TCP 连接到 127.0.0.1:port,**或**子进程
//!   已退出(kubeconfig 错等秒退场景)即快速失败,不等满超时。失败时读 child stderr
//!   拼进 error(否则用户只看到「did not become ready」,看不到 kubectl 真实报错)。
//! - **命令体与薄包装分离**:`start_proxy`/`stop_proxy`/`status_proxy` 吃
//!   `&Mutex<Option<Child>>` + kubectl 路径,可单测(spawn `sleep`/`true`/假脚本 mock);
//!   `#[tauri::command]` 只做 `State<AppState>` -> `&state.proxy` 的薄包装。
//! - **不碰凭据 / 不加 capability**:TLS+认证全留 kubectl + kubeconfig,Rust 端只
//!   spawn 进程;WASM connector 仍只发 HTTP GET(对照 CLAUDE.md kubectl proxy 架构)。
//! - kubectl 路径解析:`KUBECTL_PATH` env > PATH 扫描 > 常见绝对路径(/snap/bin/kubectl 等)。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::State;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

use crate::AppState;

/// 默认 proxy 端口(与 manifest.toml k8s config api_base 对齐)。
pub const DEFAULT_PORT: u16 = 8001;
const READINESS_TIMEOUT: Duration = Duration::from_secs(10);
const READINESS_POLL: Duration = Duration::from_millis(200);
/// 失败时读 child stderr 的最长等待(已退出的 child 立即 EOF,这只为兜底仍活着的)。
const STDERR_READ_TIMEOUT: Duration = Duration::from_millis(500);

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

/// 就绪探测失败原因。
#[derive(Debug)]
enum ProbeError {
    /// 到超时端口仍没 accept。
    Timeout,
    /// 子进程在就绪前已退出(kubeconfig 错等)。
    ChildExited,
}

// ============================================================================
// 命令体核心(吃 `&Mutex<Option<Child>>`,可单测)
// ============================================================================

/// 启动 kubectl proxy(已跑则返回当前状态)。
///
/// `kubectl_path` 由调用方经 [`resolve_kubectl`] 解析(便于测试直接传假脚本路径)。
/// 就绪后子进程句柄存 `proxy`;失败 -> 读 stderr 拼进 error + 杀子进程 + reap。
pub async fn start_proxy(
    proxy: &Mutex<Option<Child>>,
    port: u16,
    kubectl_path: &Path,
) -> Result<ProxyStatusDto, String> {
    // 已在跑 -> 返回当前状态(不重复起)
    {
        let guard = proxy.lock().map_err(|e| e.to_string())?;
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

    let mut child = Command::new(kubectl_path)
        .arg("proxy")
        .arg("--port")
        .arg(port.to_string())
        .arg("--address=127.0.0.1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped()) // 失败时读 stderr 进 error
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("spawn kubectl proxy ({}): {e}", kubectl_path.display()))?;
    let pid = child.id();

    match wait_for_proxy_or_exit(&mut child, port, READINESS_TIMEOUT, READINESS_POLL).await {
        Ok(()) => {
            // 长期运行的 child:关掉 stderr 读端,避免持有 pipe(kubectl proxy
            // stderr 写入量小,关闭后其写入得 EPIPE 自动忽略)。
            drop(child.stderr.take());
            *proxy.lock().map_err(|e| e.to_string())? = Some(child);
            Ok(ProxyStatusDto {
                running: true,
                port,
                api_base: format!("http://127.0.0.1:{port}"),
                pid,
                message: format!("kubectl proxy ready at http://127.0.0.1:{port}"),
            })
        }
        Err(ProbeError::Timeout) => {
            let stderr = read_stderr(&mut child).await;
            let _ = child.start_kill();
            let _ = child.wait().await;
            Err(with_stderr(
                format!(
                    "kubectl proxy did not become ready on 127.0.0.1:{port} within {READINESS_TIMEOUT:?}"
                ),
                stderr,
            ))
        }
        Err(ProbeError::ChildExited) => {
            let stderr = read_stderr(&mut child).await;
            let _ = child.wait().await; // 已死,reap
            Err(with_stderr(
                format!("kubectl proxy exited before serving on 127.0.0.1:{port}"),
                stderr,
            ))
        }
    }
}

/// 停止 kubectl proxy(SIGKILL 子进程 + reap)。
pub async fn stop_proxy(proxy: &Mutex<Option<Child>>) -> Result<ProxyStatusDto, String> {
    // take 出 child 后立刻释放锁,不在持锁期间 await(guard 非 Send,command future 要 Send)。
    let child_opt = {
        let mut guard = proxy.lock().map_err(|e| e.to_string())?;
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

/// 查询 proxy 状态。顺带 reap 已退出的子进程(`try_wait`)。同步(无 await)。
pub fn status_proxy(proxy: &Mutex<Option<Child>>) -> Result<ProxyStatusDto, String> {
    let mut guard = proxy.lock().map_err(|e| e.to_string())?;
    let running = match guard.as_mut() {
        Some(child) => match child.try_wait() {
            Ok(Some(_)) => {
                *guard = None; // 已退出 -> 收尸
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

// ============================================================================
// Tauri 命令薄包装
// ============================================================================

/// 启动 kubectl proxy(已跑则返回当前状态)。
#[tauri::command]
pub async fn start_kubectl_proxy(
    state: State<'_, AppState>,
    port: Option<u16>,
) -> Result<ProxyStatusDto, String> {
    let port = port.unwrap_or(DEFAULT_PORT);
    let kubectl = resolve_kubectl()?;
    start_proxy(&state.proxy, port, &kubectl).await
}

/// 停止 kubectl proxy(SIGKILL 子进程 + reap)。
#[tauri::command]
pub async fn stop_kubectl_proxy(state: State<'_, AppState>) -> Result<ProxyStatusDto, String> {
    stop_proxy(&state.proxy).await
}

/// 查询 proxy 状态。顺带 reap 已退出的子进程(`try_wait`)。
#[tauri::command]
pub async fn proxy_status(state: State<'_, AppState>) -> Result<ProxyStatusDto, String> {
    status_proxy(&state.proxy)
}

// ============================================================================
// 内部 helper
// ============================================================================

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

/// 就绪探测:轮询 TCP 连接到 127.0.0.1:port,**或**子进程已退出即快速失败。
///
/// - TCP 连通 -> `Ok(())`(kubectl proxy bind 后即 accept)。
/// - 子进程在就绪前退出(kubeconfig 错等秒退)-> `Err(ChildExited)`,不等满超时。
/// - 到超时仍未通且子进程活着 -> `Err(Timeout)`。
async fn wait_for_proxy_or_exit(
    child: &mut Child,
    port: u16,
    timeout: Duration,
    poll: Duration,
) -> Result<(), ProbeError> {
    let addr = format!("127.0.0.1:{port}");
    let deadline = Instant::now() + timeout;
    loop {
        if tokio::net::TcpStream::connect(&addr).await.is_ok() {
            return Ok(());
        }
        // 子进程已退出 -> 快速失败(否则要等满 timeout 才发现)
        if let Ok(Some(_)) = child.try_wait() {
            return Err(ProbeError::ChildExited);
        }
        if Instant::now() >= deadline {
            return Err(ProbeError::Timeout);
        }
        tokio::time::sleep(poll).await;
    }
}

/// 读 child stderr 到字符串(已退出的 child 立即 EOF)。超时 / 无 stderr -> None。
async fn read_stderr(child: &mut Child) -> Option<String> {
    let mut stderr = child.stderr.take()?;
    let mut buf = Vec::new();
    match tokio::time::timeout(STDERR_READ_TIMEOUT, stderr.read_to_end(&mut buf)).await {
        Ok(Ok(_)) => {
            let s = String::from_utf8_lossy(&buf).trim().to_string();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        }
        _ => None,
    }
}

/// 把 stderr 拼进 error message(有非空 stderr 才拼)。
fn with_stderr(msg: String, stderr: Option<String>) -> String {
    match stderr {
        Some(s) if !s.is_empty() => format!("{msg}; kubectl stderr: {s}"),
        _ => msg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    // ---------- resolve_kubectl_from(纯逻辑)----------

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, b"#!/bin/sh\n").expect("write fake bin");
        p
    }

    #[test]
    fn resolve_kubectl_from_finds_env_path() {
        let dir = std::env::temp_dir().join(format!("sre-proxy-test-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("mkdir");
        touch(&dir, "kubectl");
        let env_path = dir.join("kubectl");
        let got = resolve_kubectl_from(Some(env_path.to_str().unwrap()), &[], &[]);
        assert_eq!(got.unwrap(), env_path);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_kubectl_from_finds_in_path_dirs() {
        let dir =
            std::env::temp_dir().join(format!("sre-proxy-test-path-{}", std::process::id()));
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

    // ---------- wait_for_proxy_or_exit ----------

    /// 起一个长期存活的子进程(模拟 running proxy),kill_on_drop 防测试 panic 留孤儿。
    fn spawn_sleep() -> Child {
        Command::new("sleep")
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn sleep")
    }

    #[tokio::test]
    async fn probe_succeeds_when_port_listening() {
        // 真 TCP listener + 活着(不退出)的 child -> 探测应立即通过
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let mut child = spawn_sleep();
        let res =
            wait_for_proxy_or_exit(&mut child, port, Duration::from_secs(1), Duration::from_millis(50))
                .await;
        assert!(res.is_ok(), "probe should succeed: {res:?}");
        child.start_kill().ok();
        child.wait().await.ok();
    }

    #[tokio::test]
    async fn probe_times_out_when_port_closed_and_child_alive() {
        // port 1 关闭 + child 活着(sleep) -> 走超时(短超时快速返回)
        let mut child = spawn_sleep();
        let res =
            wait_for_proxy_or_exit(&mut child, 1, Duration::from_millis(200), Duration::from_millis(50))
                .await;
        assert!(matches!(res, Err(ProbeError::Timeout)), "got {res:?}");
        child.start_kill().ok();
        child.wait().await.ok();
    }

    #[tokio::test]
    async fn probe_detects_child_exit() {
        // child 立即退出(true) + port 1 关闭 -> ChildExited(不等超时)
        let mut child = Command::new("true")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn true");
        let res =
            wait_for_proxy_or_exit(&mut child, 1, Duration::from_secs(2), Duration::from_millis(50))
                .await;
        assert!(matches!(res, Err(ProbeError::ChildExited)), "got {res:?}");
        child.wait().await.ok();
    }

    // ---------- start_proxy / stop_proxy / status_proxy(命令体)----------

    #[cfg(unix)]
    fn set_executable(p: &Path) {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(p, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    #[cfg(not(unix))]
    fn set_executable(_p: &Path) {}

    fn tmp_dir(label: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sre-proxy-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d); // 清上一轮残留,防 ETXTBSY
        fs::create_dir_all(&d).expect("mkdir");
        d
    }

    /// 写一个假 kubectl 脚本(写 stderr 后 exit 1),返回其路径(可执行)。
    /// 显式 File + sync_all + drop 再 chmod -- 避免刚写的文件立即 exec 触发
    /// Linux ETXTBSY(写租约未释放)。
    fn write_fake_kubectl(dir: &Path, body: &str) -> PathBuf {
        let p = dir.join("fake-kubectl");
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&p).expect("create fake kubectl");
            f.write_all(body.as_bytes()).expect("write fake kubectl");
            f.sync_all().expect("sync fake kubectl");
        }
        set_executable(&p);
        p
    }

    #[tokio::test]
    async fn start_proxy_returns_already_running_when_child_present() {
        let proxy: Mutex<Option<Child>> = Mutex::new(Some(spawn_sleep()));
        let dir = tmp_dir("alr");
        let fake = write_fake_kubectl(&dir, "#!/bin/sh\nexit 0\n");
        let status = start_proxy(&proxy, 8001, &fake)
            .await
            .expect("already running");
        assert!(status.running);
        assert_eq!(status.message, "proxy already running");
        // 仍是原 child,没被替换
        assert!(proxy.lock().unwrap().is_some());
        stop_proxy(&proxy).await.expect("cleanup");
        fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn start_proxy_failure_includes_stderr() {
        // 假 kubectl:写 stderr 后 exit 1 -> start_proxy 应快速失败且 error 含 stderr。
        let dir = tmp_dir("stderr");
        let fake = write_fake_kubectl(
            &dir,
            "#!/bin/sh\necho 'kubeconfig missing boom' >&2\nexit 1\n",
        );
        let proxy: Mutex<Option<Child>> = Mutex::new(None);
        // 刚写入的可执行脚本立即 exec,在并行测试下偶发 ETXTBSY(Linux 写租约释放时序);
        // 短暂让一让 FS。生产路径 kubectl 是稳定二进制,不会触发。
        tokio::time::sleep(Duration::from_millis(30)).await;
        let err = start_proxy(&proxy, 1, &fake).await.unwrap_err();
        assert!(
            err.contains("kubeconfig missing boom"),
            "err should include kubectl stderr: {err}"
        );
        // 失败后 child 被 reap,mutex 仍空
        assert!(proxy.lock().unwrap().is_none());
        fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn stop_proxy_kills_running_child() {
        let proxy: Mutex<Option<Child>> = Mutex::new(Some(spawn_sleep()));
        let pid = proxy
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .id()
            .expect("pid");
        let status = stop_proxy(&proxy).await.expect("stop");
        assert!(!status.running);
        assert!(proxy.lock().unwrap().is_none());
        // 进程确实被杀 + reap -> /proc/{pid} 不再存在(Linux)
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "pid {pid} should be gone after stop"
        );
    }

    #[tokio::test]
    async fn status_proxy_reports_running_for_live_child() {
        let proxy: Mutex<Option<Child>> = Mutex::new(Some(spawn_sleep()));
        let status = status_proxy(&proxy).expect("status");
        assert!(status.running, "live child should report running");
        assert!(status.pid.is_some());
        stop_proxy(&proxy).await.expect("cleanup");
    }

    #[tokio::test]
    async fn status_proxy_reaps_exited_child() {
        // true 立即退出 -> status_proxy 应判 not running 并收尸
        let child = Command::new("true")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn true");
        let proxy: Mutex<Option<Child>> = Mutex::new(Some(child));
        tokio::time::sleep(Duration::from_millis(100)).await; // 等 true 退出
        let status = status_proxy(&proxy).expect("status");
        assert!(!status.running, "exited child should report not running");
        assert!(
            proxy.lock().unwrap().is_none(),
            "exited child should be reaped (mutex cleared)"
        );
    }
}
