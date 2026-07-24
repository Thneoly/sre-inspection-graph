//! code-repo — 代码仓数据源 connector(对照 doc/12 PRD-006 Sprint 1)。
//!
//! **本地文件系统扫描**(非 GitLab/GitHub API —— 桌面 / data-stays-on-machine 适配):
//! 经 `fs-read` capability(deny-by-default + host path-root allow-list)读本地克隆的
//! 仓库目录,把 Dockerfile / lockfile 映射成 topology Fact:
//!
//! - `CodeRepo` 节点(repo:<host>:<group>:<name>)
//! - `Library` 节点(pkg:<ecosystem>:<name>@<version>)+ `DEPENDS_ON` 边(repo -> library)
//! - `BUILDS` 边(repo -> image,v0 best-effort;dangles 除非 ref 与 k8s 完全一致)
//!
//! **无状态**:重扫幂等(同 repo 产同 resource_id,`facts_to_graph` 按 newest dedup)。
//! 仓库发现 = 目录有 `.git` 或已知 manifest 即视为 repo;扫到 repo 后**不再下钻其子目录**
//! (避开 node_modules / target 爆炸)。
//!
//! ## config_json
//!
//! ```json
//! { "roots": ["/abs/path/to/repos"], "host": "gitlab", "group": "order",
//!   "cluster": "vm-cluster", "namespace": "otel-demo", "depth": 2 }
//! ```
//! `roots` 必须与 manifest `fs_roots` 一致(host allow-list 据此放行)。

#![allow(missing_docs)]

pub mod mapper;

#[cfg(target_arch = "wasm32")]
mod bindings {
    wit_bindgen::generate!({
        world: "connector-world",
        path: "../../../specs/wit",
        generate_all,
    });
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::{bindings, mapper};
    use bindings::exports::sre::inspection::connector::{Fact, Guest, SyncError, SyncResult};
    use bindings::sre::inspection::{clock, fs_read, logging};
    use mapper::{Cfg, RepoAttrs};
    use serde::Deserialize;

    pub struct CodeRepo;

    /// 已知 manifest 文件名(同时是 repo 标识 + 解析器选路)。
    const MANIFESTS: &[&str] = &[
        "Dockerfile",
        "package.json",
        "go.mod",
        "Cargo.toml",
        "requirements.txt",
    ];

    #[derive(Deserialize)]
    struct Config {
        /// 待扫描的根目录(绝对路径,须在 host manifest fs_roots 内)。
        #[serde(default)]
        roots: Vec<String>,
        #[serde(default = "default_host")]
        host: String,
        #[serde(default = "default_group")]
        group: String,
        /// BUILDS target 用的 cluster / namespace(v0 best-effort)。
        #[serde(default = "default_cluster")]
        cluster: String,
        #[serde(default = "default_namespace")]
        namespace: String,
        /// 目录递归深度(根 = 0)。
        #[serde(default = "default_depth")]
        depth: u32,
    }
    fn default_host() -> String {
        "local".to_string()
    }
    fn default_group() -> String {
        "local".to_string()
    }
    fn default_cluster() -> String {
        "local".to_string()
    }
    fn default_namespace() -> String {
        "default".to_string()
    }
    fn default_depth() -> u32 {
        2
    }

    impl Guest for CodeRepo {
        fn sync(config_json: String) -> Result<SyncResult, SyncError> {
            let cfg: Config = if config_json.trim().is_empty() {
                Config {
                    roots: Vec::new(),
                    host: default_host(),
                    group: default_group(),
                    cluster: default_cluster(),
                    namespace: default_namespace(),
                    depth: default_depth(),
                }
            } else {
                serde_json::from_str(&config_json)
                    .map_err(|e| SyncError::Config(format!("invalid config_json: {e}")))?
            };
            let now = clock::now_seconds();
            let mcfg = Cfg::new(&cfg.host, &cfg.cluster, &cfg.namespace, now);

            logging::log(
                logging::Level::Info,
                &format!(
                    "code-repo sync: {} root(s) host={} group={}",
                    cfg.roots.len(),
                    cfg.host,
                    cfg.group
                ),
            );

            let mut errors: Vec<String> = Vec::new();
            if cfg.roots.is_empty() {
                errors.push("no roots configured, skipping".to_string());
                return Ok(SyncResult { facts: vec![], errors, duration_ms: 0 });
            }

            let mut facts: Vec<mapper::Fact> = Vec::new();
            for root in &cfg.roots {
                scan_dir(&mut facts, &mut errors, root, &cfg, &mcfg, 0);
            }

            // module_sdk::Fact -> WIT Fact 字段平移。
            let facts: Vec<Fact> = facts
                .into_iter()
                .map(|f| Fact {
                    id: f.id,
                    kind: f.kind,
                    source: f.source,
                    resource_id: f.resource_id,
                    resource_type: f.resource_type,
                    timestamp: f.timestamp,
                    attributes_json: f.attributes_json,
                })
                .collect();

            Ok(SyncResult { facts, errors, duration_ms: 0 })
        }

        fn health_check() -> bool {
            true
        }
    }

    /// 递归扫描 dir:若该目录是 repo(含 .git / manifest)则 scan_repo(不下钻其子目录);
    /// 否则在 depth 内继续下钻子目录。
    fn scan_dir(
        out: &mut Vec<mapper::Fact>,
        errors: &mut Vec<String>,
        dir: &str,
        cfg: &Config,
        mcfg: &Cfg,
        depth: u32,
    ) {
        let Some(children) = read_dir_safe(dir, errors) else {
            return;
        };
        if children.iter().any(|c| is_repo_marker(c)) {
            scan_repo(out, errors, dir, cfg, mcfg, &children);
            return;
        }
        if depth < cfg.depth {
            for child in &children {
                scan_dir(out, errors, child, cfg, mcfg, depth + 1);
            }
        }
    }

    /// 扫单个 repo 目录:发 CodeRepo 节点 + Dockerfile BUILDS + 各 lockfile 的 Library + DEPENDS_ON。
    fn scan_repo(
        out: &mut Vec<mapper::Fact>,
        errors: &mut Vec<String>,
        repo_dir: &str,
        cfg: &Config,
        mcfg: &Cfg,
        children: &[String],
    ) {
        let name = basename(repo_dir);
        let language = guess_language(children).to_string();
        let rid = mapper::repo_id(&cfg.host, &cfg.group, name);
        out.push(mapper::repo_node_fact(
            mcfg,
            &cfg.group,
            name,
            RepoAttrs {
                language,
                ..Default::default()
            },
        ));

        // Dockerfile FROM -> BUILDS(repo -> image)。
        if let Some(df) = children.iter().find(|c| basename(c) == "Dockerfile") {
            if let Some(content) = read_file_safe(df, errors) {
                for img in mapper::parse_dockerfile(&content) {
                    out.push(mapper::builds_edge_fact(mcfg, &rid, &img));
                }
            }
        }

        // 各 lockfile -> Library 节点 + DEPENDS_ON 边。
        for manifest in MANIFESTS {
            if *manifest == "Dockerfile" {
                continue;
            }
            let Some(path) = children.iter().find(|c| basename(c) == *manifest) else {
                continue;
            };
            if let Some(content) = read_file_safe(path, errors) {
                for lib in mapper::parse_deps(manifest, &content) {
                    out.push(mapper::library_node_fact(mcfg, &lib));
                    out.push(mapper::depends_on_edge_fact(mcfg, &rid, &lib));
                }
            }
        }
    }

    /// 路径最后一段(repo / 文件名)。
    fn basename(path: &str) -> &str {
        path.rsplit_once('/').map(|(_, name)| name).unwrap_or(path)
    }

    /// path 是 repo 标识子项(`.git` 或已知 manifest 文件)。
    fn is_repo_marker(path: &str) -> bool {
        let name = basename(path);
        name == ".git" || MANIFESTS.contains(&name)
    }

    /// 由 manifest 文件存在推测主语言。
    fn guess_language(children: &[String]) -> &'static str {
        let has = |n: &str| children.iter().any(|c| basename(c) == n);
        if has("Cargo.toml") {
            "rust"
        } else if has("package.json") {
            "javascript"
        } else if has("go.mod") {
            "go"
        } else if has("requirements.txt") {
            "python"
        } else {
            ""
        }
    }

    fn read_file_safe(path: &str, errors: &mut Vec<String>) -> Option<String> {
        match fs_read::read_file(path) {
            Ok(entry) => Some(String::from_utf8_lossy(&entry.content).into_owned()),
            Err(e) => {
                errors.push(format!("read {}: {}", basename(path), fs_err(e)));
                None
            }
        }
    }

    fn read_dir_safe(path: &str, errors: &mut Vec<String>) -> Option<Vec<String>> {
        match fs_read::read_dir(path) {
            Ok(entries) => Some(entries),
            Err(e) => {
                errors.push(format!("read_dir {}: {}", basename(path), fs_err(e)));
                None
            }
        }
    }

    fn fs_err(e: fs_read::Error) -> String {
        match e {
            fs_read::Error::NotFound => "not found".to_string(),
            fs_read::Error::PermissionDenied(m) => format!("permission denied: {m}"),
            fs_read::Error::Io(m) => format!("io: {m}"),
        }
    }
}

#[cfg(target_arch = "wasm32")]
use imp::CodeRepo;

#[cfg(target_arch = "wasm32")]
bindings::export!(CodeRepo with_types_in bindings);
