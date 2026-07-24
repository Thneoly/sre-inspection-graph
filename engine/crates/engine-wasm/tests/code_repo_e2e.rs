//! 端到端集成测试 —— code-repo connector 经 `fs-read` capability 扫描本地仓库目录,
//! 验证 CodeRepo/Library 节点 + DEPENDS_ON/BUILDS 边。**真 fs**(无 mock):fs_host 读
//! 真实 tmp fixture 目录。+ deny-by-default(缺 fs-read cap -> 0 fact + permission-denied)。
//! 找不到 wasm 跳过(先 `cd modules && cargo build -p code-repo --target wasm32-wasip2 --release`)。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use engine_wasm::WasmConnector;

fn modules_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../modules")
}

fn locate_wasm() -> Option<PathBuf> {
    let modules = modules_root();
    let text = fs::read_to_string(modules.join("manifest.toml")).ok()?;
    let parsed = engine_wasm::ManifestFile::from_toml_str(&text).ok()?;
    let m = parsed.modules.iter().find(|m| m.name == "code-repo")?;
    let wasm = modules.join(&m.wasm_path);
    wasm.exists().then_some(wasm)
}

/// 在系统 temp 下建一个 fixture 仓库树,返 canonical 根路径。
///
/// `tag` 让并行测试用各自独立目录(同 pid 下两个 #[tokio::test] 共享 tmp 路径会 race:
/// 一个 remove_dir_all 另一个的文件)。
fn make_fixture_tree(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "sre-code-repo-e2e-{}-{tag}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    // order-svc:Dockerfile(2 FROM)+ package.json(1 dep)
    let order = root.join("order-svc");
    fs::create_dir_all(&order).unwrap();
    fs::write(
        order.join("Dockerfile"),
        "FROM node:18 AS build\nRUN echo hi\nFROM alpine:3.19\n",
    )
    .unwrap();
    fs::write(
        order.join("package.json"),
        r#"{"dependencies":{"express":"^4.18.0"}}"#,
    )
    .unwrap();
    // cart-svc:go.mod(1 require)+ 无 Dockerfile
    let cart = root.join("cart-svc");
    fs::create_dir_all(&cart).unwrap();
    fs::write(
        cart.join("go.mod"),
        "module example.com/cart\n\ngo 1.22\n\nrequire (\n\tgithub.com/foo v1.0.0\n)\n",
    )
    .unwrap();
    root.canonicalize().expect("canonicalize fixture root")
}

#[tokio::test]
async fn code_repo_scans_fixtures_into_topology_facts() {
    let Some(wasm_path) = locate_wasm() else {
        eprintln!(
            "skipping: code_repo.wasm not built. Run `cargo build -p code-repo --target wasm32-wasip2 --release`."
        );
        return;
    };
    let root = make_fixture_tree("scan");
    let caps: HashSet<String> = ["logging", "clock", "fs-read"]
        .into_iter()
        .map(String::from)
        .collect();
    let mut conn = WasmConnector::load_with_roots(&wasm_path, caps, vec![root.clone()])
        .await
        .expect("load code_repo.wasm");
    let config = serde_json::json!({
        "roots": [root.to_string_lossy()],
        "host": "gitlab",
        "group": "order",
        "cluster": "vm-cluster",
        "namespace": "otel-demo",
        "depth": 2,
    });

    let out = conn.sync(&config.to_string()).await.expect("sync");
    assert!(out.errors.is_empty(), "unexpected errors: {:?}", out.errors);

    // CodeRepo 节点:order-svc + cart-svc
    let repo_rids: Vec<&str> = out
        .facts
        .iter()
        .filter(|f| f.resource_type == "CodeRepo")
        .map(|f| f.resource_id.as_str())
        .collect();
    assert_eq!(repo_rids.len(), 2, "2 repos: {:?}", repo_rids);
    assert!(repo_rids.contains(&"repo:gitlab:order:order-svc"));
    assert!(repo_rids.contains(&"repo:gitlab:order:cart-svc"));

    // Library 节点:express(npm)+ github.com/foo(go)
    let lib_rids: Vec<&str> = out
        .facts
        .iter()
        .filter(|f| f.resource_type == "Library")
        .map(|f| f.resource_id.as_str())
        .collect();
    assert_eq!(lib_rids.len(), 2, "2 libs: {:?}", lib_rids);
    assert!(lib_rids.contains(&"pkg:npm:express@4.18.0"));
    assert!(lib_rids.contains(&"pkg:go:github.com/foo@v1.0.0"));

    // DEPENDS_ON 边:每 repo -> 各自 lib(2 条)
    let depends: Vec<&engine_wasm::HostFact> = out
        .facts
        .iter()
        .filter(|f| f.kind == "topology-edge" && edge_type(f) == "DEPENDS_ON")
        .collect();
    assert_eq!(depends.len(), 2, "2 DEPENDS_ON edges");
    let dep_targets: Vec<String> = depends.iter().map(|f| edge_target(f)).collect();
    assert!(dep_targets.contains(&"pkg:npm:express@4.18.0".to_string()));
    assert!(dep_targets.contains(&"pkg:go:github.com/foo@v1.0.0".to_string()));

    // BUILDS 边:order-svc Dockerfile 2 个 FROM(node:18, alpine:3.19);cart-svc 无 Dockerfile
    let builds: Vec<&engine_wasm::HostFact> = out
        .facts
        .iter()
        .filter(|f| f.kind == "topology-edge" && edge_type(f) == "BUILDS")
        .collect();
    assert_eq!(builds.len(), 2, "2 BUILDS edges (only order-svc has Dockerfile)");
    let build_targets: Vec<String> = builds.iter().map(|f| edge_target(f)).collect();
    // Phase 8.2 C1:BUILDS 现指向 image-ref 节点(非 v0 的 image:{c}:{ns}:{ref})。
    assert!(build_targets.contains(&"image-ref:node:18".to_string()));
    assert!(build_targets.contains(&"image-ref:alpine:3.19".to_string()));

    // cleanup(best-effort)
    let _ = fs::remove_dir_all(&root);
}

#[tokio::test]
async fn code_repo_denied_without_fs_read_capability() {
    let Some(wasm_path) = locate_wasm() else {
        eprintln!("skipping: code_repo.wasm not built.");
        return;
    };
    let root = make_fixture_tree("denied");
    // 缺 fs-read cap:host 每次 read 拒回 permission-denied -> 0 fact + error。
    let caps: HashSet<String> = ["logging", "clock"].into_iter().map(String::from).collect();
    let mut conn = WasmConnector::load_with_roots(&wasm_path, caps, vec![root.clone()])
        .await
        .expect("load");
    let config = serde_json::json!({ "roots": [root.to_string_lossy()] });
    let out = conn.sync(&config.to_string()).await.expect("sync");
    assert!(out.facts.is_empty(), "denied -> no facts: {:?}", out.facts);
    assert!(
        out.errors.iter().any(|e| e.contains("permission denied") || e.contains("not found")),
        "expected permission-denied error note: {:?}",
        out.errors
    );
    let _ = fs::remove_dir_all(&root);
}

/// 从 edge fact 的 attributes_json 读 `edge_type`。
fn edge_type(f: &engine_wasm::HostFact) -> String {
    attr_str(f, "/edge_type")
}

/// 从 edge fact 的 attributes_json 读 `target`。
fn edge_target(f: &engine_wasm::HostFact) -> String {
    attr_str(f, "/target")
}

fn attr_str(f: &engine_wasm::HostFact, pointer: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(&f.attributes_json).expect("attrs json");
    v.pointer(pointer)
        .map(|s| s.as_str().unwrap_or("").to_string())
        .unwrap_or_default()
}

#[allow(dead_code)]
fn _ensure_path_send(_p: &Path) {}
