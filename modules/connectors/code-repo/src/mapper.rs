//! Code-repo mapper — 纯函数,host target 可单测(对照 doc/12 PRD-006 Sprint 1)。
//!
//! 把本地仓库的 Dockerfile / lockfile 文本映射成 topology-node / topology-edge Fact:
//!
//! - `CodeRepo` 节点(`repo:<host>:<group>:<name>`,attrs git_url/language/topics/...)
//! - `Library` 节点(`pkg:<ecosystem>:<name>@<version>`,purl 风格)
//! - `DEPENDS_ON` 边(`CodeRepo` -> `Library`,两端都由本 connector 产 -> v0 下挂得上)
//! - `BUILDS` 边(`CodeRepo` -> `image:<cluster>:<ns>:<ref>`,**v0 best-effort**:
//!   repo 不知 cluster/ns,合成 target 仅当 k8s connector 产的同 ref 完全一致才挂上;
//!   k8s 不做镜像 normalize,故不匹配即被 `facts_to_graph` 当悬空边过滤。robust
//!   repo->image 合并(经 image-ref correlation key + normalize)留 C1 / Phase 8.2)
//!
//! **不产 change/metric/alert**:PR/MR 事件 = webhook(桌面反模式,跳过);规则抽取
//! (InspectionRule)留 Sprint 2。无状态映射;仓库发现 + 文件读取经 fs-read 在 lib.rs(guest)。

pub use module_sdk::Fact;
use serde_json::{json, Value};

/// source 标识(Fact.source = connector 名)。
pub const SOURCE: &str = "code-repo";
const NODE_KIND: &str = "topology-node";
const EDGE_KIND: &str = "topology-edge";

/// 聚合参数(由 lib.rs 从 config_json + clock 组装)。
#[derive(Clone)]
pub struct Cfg {
    /// 代码仓 host 标识(如 "gitlab" / "github" / "local")。
    pub host: String,
    /// BUILDS target 用的 cluster(k8s connector 同款;v0 best-effort)。
    pub cluster: String,
    /// BUILDS target 用的 namespace。
    pub namespace: String,
    /// 本轮 sync 时间戳。
    pub now: u64,
}

impl Cfg {
    /// 构造聚合参数。
    pub fn new(host: &str, cluster: &str, namespace: &str, now: u64) -> Self {
        Self {
            host: host.to_string(),
            cluster: cluster.to_string(),
            namespace: namespace.to_string(),
            now,
        }
    }
}

/// repo resource_id:`repo:<host>:<group>:<name>`(对照 doc/12 §3.1)。
pub fn repo_id(host: &str, group: &str, name: &str) -> String {
    format!("repo:{host}:{group}:{name}")
}

/// library resource_id:`pkg:<ecosystem>:<name>@<version>`(purl 风格,doc/12 §5.3)。
pub fn library_id(ecosystem: &str, name: &str, version: &str) -> String {
    format!("pkg:{ecosystem}:{name}@{version}")
}

/// 已解析的依赖引用(供 lib.rs 发 Library 节点 + DEPENDS_ON 边)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryRef {
    /// 包生态(npm / go / cargo / pypi)。
    pub ecosystem: &'static str,
    /// 包名(go module path / npm name / crate / pypi name)。
    pub name: String,
    /// 清理后的版本(剥 ^/~/>=/v 前缀)。
    pub version: String,
}

/// CodeRepo 节点的富化 attrs(lib.rs 从仓库内容推测,缺省空)。
#[derive(Default, Clone)]
pub struct RepoAttrs {
    /// `.git/config` remote url(可选,本地扫描可能没填)。
    pub git_url: String,
    /// 默认分支(如 "main";可选)。
    pub default_branch: String,
    /// 主语言(由 manifest 推测:rust/javascript/go/python)。
    pub language: String,
    /// 仓库 topics(可选)。
    pub topics: Vec<String>,
    /// 拥有团队(可选)。
    pub owner_team: String,
    /// 最近 commit 时间(可选)。
    pub last_commit_at: String,
}

/// CodeRepo 节点 Fact。
pub fn repo_node_fact(cfg: &Cfg, group: &str, name: &str, attrs: RepoAttrs) -> Fact {
    let rid = repo_id(&cfg.host, group, name);
    node_fact(
        cfg.now,
        &rid,
        "CodeRepo",
        json!({
            "name": name,
            "git_url": attrs.git_url,
            "default_branch": attrs.default_branch,
            "language": attrs.language,
            "topics": attrs.topics,
            "owner_team": attrs.owner_team,
            "host": cfg.host,
            "group": group,
            "last_commit_at": attrs.last_commit_at,
        }),
    )
}

/// Library 节点 Fact。
pub fn library_node_fact(cfg: &Cfg, lib: &LibraryRef) -> Fact {
    let rid = library_id(lib.ecosystem, &lib.name, &lib.version);
    node_fact(
        cfg.now,
        &rid,
        "Library",
        json!({
            "ecosystem": lib.ecosystem,
            "name": lib.name,
            "version": lib.version,
        }),
    )
}

/// `DEPENDS_ON` 边(repo -> library)。两端都由本 connector 产 -> v0 下挂得上。
pub fn depends_on_edge_fact(cfg: &Cfg, repo_rid: &str, lib: &LibraryRef) -> Fact {
    let target = library_id(lib.ecosystem, &lib.name, &lib.version);
    edge_fact(cfg.now, "DEPENDS_ON", repo_rid, &target)
}

/// `BUILDS` 边(repo -> ContainerImage)。**v0 best-effort**:target 用 cfg.cluster/ns
/// + Dockerfile image-ref 合成。仅当 k8s connector 产的同 ref 完全一致才挂上,否则悬空。
pub fn builds_edge_fact(cfg: &Cfg, repo_rid: &str, image_ref: &str) -> Fact {
    let target = format!("image:{}:{}:{}", cfg.cluster, cfg.namespace, image_ref);
    edge_fact(cfg.now, "BUILDS", repo_rid, &target)
}

// ── 解析器(纯函数,host 可测)──────────────────────────────────────────

/// Dockerfile `FROM` 行 -> 基础镜像 ref 列表(去重)。忽略 `--platform=` flag 与
/// `AS <alias>` 多阶段别名。对照 doc/12 §5.2 `parse_dockerfile`。
pub fn parse_dockerfile(content: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.to_ascii_lowercase().starts_with("from ") {
            continue;
        }
        // FROM 之后的 token:跳 --flag,遇 AS 停,取首个镜像 ref。
        let mut image: Option<&str> = None;
        for tok in trimmed.split_whitespace().skip(1) {
            if tok.eq_ignore_ascii_case("as") {
                break;
            }
            if tok.starts_with("--") {
                continue;
            }
            image = Some(tok);
            break;
        }
        if let Some(img) = image {
            if !img.is_empty() && !out.contains(&img.to_string()) {
                out.push(img.to_string());
            }
        }
    }
    out
}

/// 按 manifest 文件名分发依赖解析。未知文件名 -> 空。
pub fn parse_deps(filename: &str, content: &str) -> Vec<LibraryRef> {
    match filename {
        "package.json" => parse_npm(content),
        "go.mod" => parse_go(content),
        "Cargo.toml" => parse_cargo(content),
        "requirements.txt" => parse_pypi(content),
        _ => Vec::new(),
    }
}

/// npm:dependencies + devDependencies + optionalDependencies(对照 doc/12 §5.3)。
fn parse_npm(content: &str) -> Vec<LibraryRef> {
    let Ok(v) = serde_json::from_str::<Value>(content) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for section in ["dependencies", "devDependencies", "optionalDependencies"] {
        if let Some(obj) = v.get(section).and_then(Value::as_object) {
            for (name, ver) in obj {
                if let Some(ver_str) = ver.as_str() {
                    let ver = clean_version(ver_str);
                    if !ver.is_empty() {
                        out.push(LibraryRef {
                            ecosystem: "npm",
                            name: name.clone(),
                            version: ver,
                        });
                    }
                }
            }
        }
    }
    out
}

/// go.mod:require 块 + 单行 require(对照 doc/12 §5.3)。
fn parse_go(content: &str) -> Vec<LibraryRef> {
    let mut out = Vec::new();
    let mut in_block = false;
    for line in content.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with("//") {
            continue;
        }
        if t.starts_with("require") && t.contains('(') {
            in_block = true;
            continue;
        }
        if in_block && t == ")" {
            in_block = false;
            continue;
        }
        let rest = if in_block {
            t
        } else {
            t.strip_prefix("require").map(str::trim).unwrap_or("")
        };
        if rest.is_empty() {
            continue;
        }
        let mut toks = rest.split_whitespace();
        let (Some(name), Some(ver)) = (toks.next(), toks.next()) else {
            continue;
        };
        let ver = clean_version(ver);
        if !ver.is_empty() {
            out.push(LibraryRef {
                ecosystem: "go",
                name: name.to_string(),
                version: ver,
            });
        }
    }
    out
}

/// Cargo.toml:[dependencies] / [dev-dependencies] / [build-dependencies]。
/// 支持 `name = "1.0"` 与 `name = { version = "1.0", ... }`(取首个引号串)。
fn parse_cargo(content: &str) -> Vec<LibraryRef> {
    let mut out = Vec::new();
    let mut section = String::new();
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            section = t[1..t.len() - 1].to_string();
            continue;
        }
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if !matches!(
            section.as_str(),
            "dependencies" | "dev-dependencies" | "build-dependencies"
        ) {
            continue;
        }
        let Some((name, rest)) = t.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        // table 形 `{ version = "1.0", ... }` 找 `version` key;inline `"1.0"` 取首引号串。
        // path / git deps(无 version key)-> 空 -> 跳。
        let rest_trim = rest.trim_start();
        let version = if rest_trim.starts_with('{') {
            rest.find("version").map(|i| {
                let after = &rest[i..];
                after
                    .find('"')
                    .map(|s| after[s + 1..].split('"').next().unwrap_or("").to_string())
                    .unwrap_or_default()
            })
        } else {
            rest.find('"')
                .map(|s| rest[s + 1..].split('"').next().unwrap_or("").to_string())
        }
        .unwrap_or_default();
        let version = clean_version(&version);
        if !version.is_empty() {
            out.push(LibraryRef {
                ecosystem: "cargo",
                name: name.to_string(),
                version,
            });
        }
    }
    out
}

/// requirements.txt:`name==1.0` / `name>=1.0` 等(对照 doc/12 §5.3)。
fn parse_pypi(content: &str) -> Vec<LibraryRef> {
    let mut out = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') || t.starts_with('-') {
            continue;
        }
        // 砍环境标记:`name==1.0 ; python_version >= "3.8"`
        let t = t.split(';').next().unwrap_or(t).trim();
        let Some(idx) = find_pypi_op(t) else {
            continue;
        };
        let name = t[..idx].trim();
        let ver = clean_version(t[idx..].trim());
        if !name.is_empty() && !ver.is_empty() {
            out.push(LibraryRef {
                ecosystem: "pypi",
                name: name.to_string(),
                version: ver,
            });
        }
    }
    out
}

/// 找 pypi 版本操作符(== / >= / <= / ~= / > / <)的起始 index。
fn find_pypi_op(s: &str) -> Option<usize> {
    ["==", ">=", "<=", "~=", ">", "<"]
        .iter()
        .filter_map(|op| s.find(op))
        .min()
}

/// 剥版本串前缀的 semver 操作符:`^4.17.21` -> `4.17.21`、`>=1.0` -> `1.0`。
///
/// **不剥 `v` 前缀**:Go module 版本(`v1.2.3`)的 `v` 是规范一部分,剥了会丢语义。
fn clean_version(v: &str) -> String {
    v.trim_start_matches(['^', '~', '>', '<', '=', ' '])
        .trim()
        .to_string()
}

// ── fact helpers(对照 k8s mapper node_fact/edge_fact 形状,facts_to_graph 认)──

fn node_fact(now: u64, resource_id: &str, resource_type: &str, attrs: Value) -> Fact {
    Fact {
        id: format!("{SOURCE}:{resource_id}:{now}"),
        kind: NODE_KIND.to_string(),
        source: SOURCE.to_string(),
        resource_id: resource_id.to_string(),
        resource_type: resource_type.to_string(),
        timestamp: now,
        attributes_json: attrs.to_string(),
    }
}

fn edge_fact(now: u64, edge_type: &str, source: &str, target: &str) -> Fact {
    let resource_id = format!("edge:{edge_type}:{source}->{target}");
    Fact {
        id: format!("{SOURCE}:edge:{edge_type}:{source}->{target}:{now}"),
        kind: EDGE_KIND.to_string(),
        source: SOURCE.to_string(),
        resource_id,
        resource_type: "Edge".to_string(),
        timestamp: now,
        attributes_json: json!({ "source": source, "target": target, "edge_type": edge_type })
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Cfg {
        Cfg::new("gitlab", "vm-cluster", "otel-demo", 1_700_000_000)
    }

    fn attr(f: &Fact, pointer: &str) -> Value {
        let v: Value = serde_json::from_str(&f.attributes_json).unwrap();
        v.pointer(pointer).cloned().unwrap_or(Value::Null)
    }

    // ── id 格式 ──

    #[test]
    fn repo_and_library_id_formats() {
        assert_eq!(repo_id("gitlab", "order", "order-svc"), "repo:gitlab:order:order-svc");
        assert_eq!(library_id("npm", "lodash", "4.17.21"), "pkg:npm:lodash@4.17.21");
    }

    #[test]
    fn repo_node_fact_shape() {
        let f = repo_node_fact(
            &cfg(),
            "order",
            "order-svc",
            RepoAttrs { language: "rust".into(), git_url: "https://x/order-svc".into(), ..Default::default() },
        );
        assert_eq!(f.kind, "topology-node");
        assert_eq!(f.resource_type, "CodeRepo");
        assert_eq!(f.resource_id, "repo:gitlab:order:order-svc");
        assert_eq!(attr(&f, "/language"), json!("rust"));
        assert_eq!(attr(&f, "/git_url"), json!("https://x/order-svc"));
        assert_eq!(f.source, "code-repo");
    }

    #[test]
    fn library_node_and_depends_on_edge() {
        let c = cfg();
        let lib = LibraryRef { ecosystem: "npm", name: "lodash".into(), version: "4.17.21".into() };
        let libf = library_node_fact(&c, &lib);
        assert_eq!(libf.resource_id, "pkg:npm:lodash@4.17.21");
        assert_eq!(libf.resource_type, "Library");

        let repo = repo_id("gitlab", "order", "order-svc");
        let ef = depends_on_edge_fact(&c, &repo, &lib);
        assert_eq!(ef.kind, "topology-edge");
        assert_eq!(attr(&ef, "/edge_type"), json!("DEPENDS_ON"));
        assert_eq!(attr(&ef, "/source"), json!("repo:gitlab:order:order-svc"));
        assert_eq!(attr(&ef, "/target"), json!("pkg:npm:lodash@4.17.21"));
    }

    #[test]
    fn builds_edge_targets_k8s_image_id_shape() {
        let repo = repo_id("gitlab", "order", "order-svc");
        let ef = builds_edge_fact(&cfg(), &repo, "ghcr.io/otel/demo/cart:0.1.2");
        assert_eq!(attr(&ef, "/edge_type"), json!("BUILDS"));
        assert_eq!(
            attr(&ef, "/target"),
            json!("image:vm-cluster:otel-demo:ghcr.io/otel/demo/cart:0.1.2")
        );
    }

    // ── parse_dockerfile ──

    #[test]
    fn dockerfile_extracts_from_images() {
        let df = r#"
FROM --platform=$BUILDPLATFORM golang:1.22 AS builder
WORKDIR /src
COPY . .
FROM alpine:3.19
COPY --from=builder /src/app /app
"#;
        let imgs = parse_dockerfile(df);
        assert_eq!(imgs, vec!["golang:1.22".to_string(), "alpine:3.19".to_string()]);
    }

    #[test]
    fn dockerfile_dedups_and_ignores_non_from() {
        let df = "FROM redis:7\nRUN apt-get update\nFROM redis:7\n";
        assert_eq!(parse_dockerfile(df), vec!["redis:7".to_string()]);
    }

    #[test]
    fn dockerfile_empty() {
        assert!(parse_dockerfile("no dockerfile here\n").is_empty());
    }

    // ── parse_deps:每种语言 ──

    #[test]
    fn parse_npm_deps() {
        let pj = r#"{"dependencies":{"lodash":"^4.17.21","express":"~4.18.0"},"devDependencies":{"jest":"29.0.0"}}"#;
        let deps = parse_deps("package.json", pj);
        assert_eq!(deps.len(), 3);
        assert!(deps.contains(&LibraryRef { ecosystem: "npm", name: "lodash".into(), version: "4.17.21".into() }));
        assert!(deps.contains(&LibraryRef { ecosystem: "npm", name: "express".into(), version: "4.18.0".into() }));
        assert!(deps.contains(&LibraryRef { ecosystem: "npm", name: "jest".into(), version: "29.0.0".into() }));
    }

    #[test]
    fn parse_go_mod_block_and_single() {
        let go = r#"
module example.com/app
go 1.22
require (
    github.com/foo/bar v1.2.3
    github.com/baz v0.5.0 // indirect
)
require github.com/single v2.0.0
"#;
        let deps = parse_deps("go.mod", go);
        assert_eq!(deps.len(), 3);
        assert!(deps.iter().any(|d| d.name == "github.com/foo/bar" && d.version == "v1.2.3"));
        assert!(deps.iter().any(|d| d.name == "github.com/single" && d.version == "v2.0.0"));
        assert!(deps.iter().all(|d| d.ecosystem == "go"));
    }

    #[test]
    fn parse_cargo_toml_inline_and_table() {
        let cargo = r#"
[package]
name = "app"
version = "0.1.0"
[dependencies]
serde = "1.0"
tokio = { version = "1.35", features = ["full"] }
local = { path = "../local" }
[dev-dependencies]
proptest = "1.4"
"#;
        let deps = parse_deps("Cargo.toml", cargo);
        // serde / tokio / proptest(local path dep 无版本 -> 跳)
        assert_eq!(deps.len(), 3);
        assert!(deps.iter().any(|d| d.name == "serde" && d.version == "1.0" && d.ecosystem == "cargo"));
        assert!(deps.iter().any(|d| d.name == "tokio" && d.version == "1.35"));
        assert!(deps.iter().any(|d| d.name == "proptest"));
        assert!(!deps.iter().any(|d| d.name == "local"));
        assert!(!deps.iter().any(|d| d.name == "app")); // [package] section 不算
    }

    #[test]
    fn parse_requirements_txt() {
        let req = r#"
requests==2.31.0
flask>=2.0
numpy~=1.26 ; python_version >= "3.9"
# a comment
-c constraints.txt
"#;
        let deps = parse_deps("requirements.txt", req);
        assert_eq!(deps.len(), 3);
        assert!(deps.iter().any(|d| d.name == "requests" && d.version == "2.31.0" && d.ecosystem == "pypi"));
        assert!(deps.iter().any(|d| d.name == "flask" && d.version == "2.0"));
        assert!(deps.iter().any(|d| d.name == "numpy" && d.version == "1.26"));
    }

    #[test]
    fn parse_deps_unknown_filename() {
        assert!(parse_deps("Gemfile", "gem 'rails'").is_empty());
    }

    #[test]
    fn clean_version_strips_operators() {
        assert_eq!(clean_version("^4.17.21"), "4.17.21");
        assert_eq!(clean_version(">=1.0"), "1.0");
        assert_eq!(clean_version("v1.2.3"), "v1.2.3"); // Go: v 前缀保留
        assert_eq!(clean_version("2.0"), "2.0");
    }
}
