//! `fs-read` capability 的 host 端实装(Phase 8.1)。
//!
//! 与 [`crate::http_host`] 同构 —— 这里定义 host 侧 plain types
//! ([`HostFsEntry`] / [`HostFsError`])和纯函数 [`read_file`] / [`read_dir`],
//! **完全不耦合 wit-bindgen 生成的 binding 类型**。这样:
//! 1. 可以脱离 wasmtime Store 单独单测(见 mod tests)
//! 2. binding 类型随 WIT 演进时,host 实装不必跟着改
//!
//! [`crate::runtime`] 里 `FsReadHost for State` 是薄适配:做 host 类型 ↔
//! binding 类型字段平移,真正的 capability 检查 + path 校验都在这里。
//!
//! # 安全设计(deny-by-default + path-root allow-list)
//!
//! 与 `http-client` 不同,**fs-read 从第一天就强制 path-root allow-list**
//! (`http-client` 的 URL allow-list 留到了 Phase 3 TODO)。文件系统访问一旦
//! 裸开,目录穿越(`../../etc/passwd`)和符号链接逃逸是直接的安全漏洞,故:
//!
//! 1. **capability allow-list at call time** —— 每次 read 都查
//!    `allowed_capabilities` 包不包 `"fs-read"`。
//! 2. **path-root allow-list** —— host 端 `manifest.fs_roots` 列出允许的根目录
//!    (绝对路径,canonicalize 后传入)。请求路径 canonicalize 后须 `starts_with`
//!    某根(组件级匹配,非字符串前缀)。
//! 3. **canonicalize 阻断逃逸** —— `std::fs::canonicalize` 解析 `..` 和符号链接
//!    到真实路径;指向 allowed root 之外的符号链接解析后落在根外 → 拒绝。
//! 4. **read-only** —— 只暴露 read_file / read_dir,无 write/create/delete
//!    (区别于 raw WASI preopens,后者给完整读写删)。

use std::collections::HashSet;
use std::path::PathBuf;

/// 申明 fs-read capability 时使用的字符串。manifest.toml `capabilities = [...]`
/// 里出现此值才能调 [`read_file`] / [`read_dir`]。
pub const CAP_FS_READ: &str = "fs-read";

/// host 侧文件读取结果 —— [`crate::runtime`] 把它转 WIT `entry` record。
#[derive(Debug, Clone)]
pub struct HostFsEntry {
    /// canonicalize 后的真实路径(供 guest 日志 / 调试)。
    pub path: String,
    /// 文件内容原始字节。
    pub content: Vec<u8>,
}

/// host 侧 fs 错误 —— 与 WIT `fs-read.error` variant 一一对应。
#[derive(Debug, Clone, thiserror::Error)]
pub enum HostFsError {
    /// 路径不存在(`canonicalize` / `read` 返 NotFound)。
    #[error("not found")]
    NotFound,
    /// capability 未申明、无 fs_roots、或路径落在 allowed root 之外
    /// (含目录穿越 / 符号链接逃逸)。
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    /// 其它 I/O 错误(权限、磁盘等)。
    #[error("io: {0}")]
    Io(String),
}

/// 把 `std::io::Error` 按 kind 映射到 [`HostFsError`]。
fn map_io_err(path: &str, e: std::io::Error) -> HostFsError {
    use std::io::ErrorKind;
    match e.kind() {
        ErrorKind::NotFound => HostFsError::NotFound,
        ErrorKind::PermissionDenied => {
            HostFsError::PermissionDenied(format!("os denied access to {path}"))
        }
        _ => HostFsError::Io(format!("{path}: {e}")),
    }
}

/// 公共前置校验:capability 检查 + canonicalize + under-allowed-root。
///
/// 返回 canonicalize 后的真实路径,供后续 `read` / `read_dir` 使用。
fn resolve_and_check(
    allowed_capabilities: &HashSet<String>,
    allowed_roots: &[PathBuf],
    path: &str,
) -> Result<PathBuf, HostFsError> {
    if !allowed_capabilities.contains(CAP_FS_READ) {
        return Err(HostFsError::PermissionDenied(format!(
            "capability '{CAP_FS_READ}' not declared in module manifest"
        )));
    }
    if allowed_roots.is_empty() {
        return Err(HostFsError::PermissionDenied(
            "fs-read capability granted but no fs_roots configured in manifest".to_string(),
        ));
    }
    // canonicalize 解析 `..` 和符号链接到真实绝对路径 —— 这是阻断目录穿越 /
    // 符号链接逃逸的关键。路径不存在 -> NotFound。
    let canonical = std::fs::canonicalize(path).map_err(|e| map_io_err(path, e))?;
    // starts_with 是组件级匹配(/a/b 对 /a/bb 为 false),非字符串前缀。
    if !allowed_roots.iter().any(|root| canonical.starts_with(root)) {
        return Err(HostFsError::PermissionDenied(format!(
            "path '{path}' resolves outside all allowed fs_roots"
        )));
    }
    Ok(canonical)
}

/// host 端 capability-gated + root-gated 文件读取。
///
/// 流程:
/// 1. 查 `allowed_capabilities`,缺 `"fs-read"` → [`HostFsError::PermissionDenied`]
/// 2. 查 `allowed_roots` 非空(否则 deny —— 有 cap 无根 = 无访问)
/// 3. canonicalize 请求路径(解析 `..` / 符号链接)
/// 4. 校验 canonical 路径落在某 allowed root 下,否则 deny(目录穿越 / 逃逸)
/// 5. `std::fs::read` 读字节
pub fn read_file(
    allowed_capabilities: &HashSet<String>,
    allowed_roots: &[PathBuf],
    path: &str,
) -> Result<HostFsEntry, HostFsError> {
    let canonical = resolve_and_check(allowed_capabilities, allowed_roots, path)?;
    let content = std::fs::read(&canonical).map_err(|e| map_io_err(path, e))?;
    Ok(HostFsEntry {
        path: canonical.to_string_lossy().into_owned(),
        content,
    })
}

/// host 端 capability-gated + root-gated 目录列举。
///
/// 同 [`read_file`] 的前置校验,然后 `std::fs::read_dir` 列子项,返回各子项
/// 的完整路径(canonical 根 + 子项名,已排序便于确定性测试)。供 guest 驱动
/// 递归扫描(读到子项路径后再 read_file / read_dir)。
pub fn read_dir(
    allowed_capabilities: &HashSet<String>,
    allowed_roots: &[PathBuf],
    path: &str,
) -> Result<Vec<String>, HostFsError> {
    let canonical = resolve_and_check(allowed_capabilities, allowed_roots, path)?;
    let entries = std::fs::read_dir(&canonical).map_err(|e| map_io_err(path, e))?;
    let mut out: Vec<String> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| map_io_err(path, e))?;
        out.push(entry.path().to_string_lossy().into_owned());
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// 在系统 temp 下建一个唯一目录(进程 id 防并行测试互撞),返其路径。
    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sre-fs-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn allow_fs() -> HashSet<String> {
        let mut s = HashSet::new();
        s.insert(CAP_FS_READ.to_string());
        s
    }

    fn empty_caps() -> HashSet<String> {
        HashSet::new()
    }

    #[test]
    fn read_file_denies_when_capability_missing() {
        let root = scratch_dir("no-cap");
        let target = root.join("f.txt");
        fs::write(&target, b"hi").unwrap();
        let err = read_file(&empty_caps(), std::slice::from_ref(&root), &target.to_string_lossy())
            .expect_err("should deny");
        match err {
            HostFsError::PermissionDenied(msg) => {
                assert!(msg.contains("fs-read"), "msg names cap: {msg}");
                assert!(msg.contains("not declared"), "msg explains: {msg}");
            }
            other => panic!("expected PermissionDenied(cap), got {other:?}"),
        }
    }

    #[test]
    fn read_file_denies_when_no_roots_configured() {
        let root = scratch_dir("no-roots");
        let target = root.join("f.txt");
        fs::write(&target, b"hi").unwrap();
        let err = read_file(&allow_fs(), &[], &target.to_string_lossy())
            .expect_err("should deny");
        match err {
            HostFsError::PermissionDenied(msg) => {
                assert!(msg.contains("fs_roots"), "msg mentions fs_roots: {msg}");
            }
            other => panic!("expected PermissionDenied(no roots), got {other:?}"),
        }
    }

    #[test]
    fn read_file_blocks_traversal_outside_root() {
        let root = scratch_dir("traversal");
        let outside = scratch_dir("outside"); // 另一个独立目录
        // root/../../../<outside> —— canonicalize 后落在 root 之外
        let crafted = format!(
            "{}/../{}",
            root.display(),
            outside.file_name().unwrap().to_string_lossy()
        );
        let err = read_file(&allow_fs(), std::slice::from_ref(&root), &crafted).expect_err("should deny");
        match err {
            HostFsError::PermissionDenied(msg) => {
                assert!(msg.contains("outside"), "msg says outside: {msg}");
            }
            other => panic!("expected PermissionDenied(outside), got {other:?}"),
        }
    }

    #[test]
    fn read_file_blocks_absolute_outside_root() {
        let root = scratch_dir("abs-outside");
        let other = scratch_dir("other-abs");
        let target = other.join("secret.txt");
        fs::write(&target, b"x").unwrap();
        // target 真实存在,但不在 root 下
        let err = read_file(&allow_fs(), &[root], &target.to_string_lossy())
            .expect_err("should deny");
        assert!(matches!(err, HostFsError::PermissionDenied(_)));
    }

    #[test]
    fn read_file_blocks_symlink_escape() {
        let root = scratch_dir("symlink");
        let outside = scratch_dir("symlink-outside");
        let real = outside.join("real.txt");
        fs::write(&real, b"secret").unwrap();
        // root/link -> outside/real.txt(符号链接指向根外)
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, root.join("link")).expect("symlink");
        // 非 unix:跳过(本仓 target 含 wasm + linux host,unix 路径为主)
        #[cfg(not(unix))]
        return;
        let err = read_file(
            &allow_fs(),
            std::slice::from_ref(&root),
            &root.join("link").to_string_lossy(),
        )
        .expect_err("symlink escape must be denied");
        assert!(matches!(err, HostFsError::PermissionDenied(_)));
    }

    #[test]
    fn read_file_not_found_under_root() {
        let root = scratch_dir("notfound");
        let missing = root.join("ghost.txt"); // 不存在
        let err = read_file(&allow_fs(), &[root], &missing.to_string_lossy())
            .expect_err("should be NotFound");
        assert!(matches!(err, HostFsError::NotFound));
    }

    #[test]
    fn read_file_happy_path() {
        let root = scratch_dir("happy");
        let target = root.join("f.txt");
        fs::write(&target, b"hello-fs").unwrap();
        let entry = read_file(&allow_fs(), std::slice::from_ref(&root), &target.to_string_lossy())
            .expect("ok");
        assert_eq!(entry.content, b"hello-fs");
        // 返回 canonical 路径(应与 target canonicalize 一致)
        let canon = target.canonicalize().unwrap();
        assert_eq!(Path::new(&entry.path), canon);
    }

    #[test]
    fn read_dir_lists_children_sorted() {
        let root = scratch_dir("listdir");
        fs::write(root.join("b.txt"), b"").unwrap();
        fs::write(root.join("a.txt"), b"").unwrap();
        fs::create_dir_all(root.join("sub")).unwrap();
        let children = read_dir(&allow_fs(), std::slice::from_ref(&root), &root.to_string_lossy())
            .expect("ok");
        // 排序后:a.txt, b.txt, sub
        let names: Vec<String> = children
            .iter()
            .map(|p| Path::new(p).file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.txt", "b.txt", "sub"]);
    }

    #[test]
    fn read_dir_allows_root_itself() {
        // 读 root 自身目录应允许(starts_with 包含自身)
        let root = scratch_dir("rootself");
        fs::write(root.join("x"), b"").unwrap();
        let children = read_dir(&allow_fs(), std::slice::from_ref(&root), &root.to_string_lossy())
            .expect("reading the granted root itself is allowed");
        assert_eq!(children.len(), 1);
    }
}
