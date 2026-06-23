# 16 — 仓库与代码目录设计(Supervised Rewrite + Tauri)

## 0. 上下文

[`doc/14`](./14-long-term-tech-strategy.md) 决定走 **Supervised Rewrite + Tauri 桌面化**(不是 Strangler Fig)。本文落档承载这些决策的物理仓库结构。

> v0.2 大改:v0.1 设计的双语言并存桥接结构被否决。本版本是 **单语言主体(Rust)+ Python reference 旁挂** + Tauri 桌面壳。从 635 行砍到 ~430 行,结构简洁很多。

关键约束:

- **单语言开发心智** — 日常只在 Rust + TS 之间切,不在 Python ↔ Rust 之间切
- **Python reference 旁挂只读** — 本地 dev 跑作 oracle,不部署
- **Tauri 桌面单一交付物** — `cargo tauri build` 出 .app/.AppImage/.msi
- **engine-cli 可选 headless 二进制** — 团队/SaaS 模式留口子
- **WASM 插件独立 workspace** — 编译目标隔离

## 1. 顶层目录

```
graph_data/
├── engine/             # Rust workspace — engine 内核 + CLI binary
├── desktop/            # Tauri 2.x 桌面应用(Rust + React webview)
├── modules/            # WASM 插件源码,独立 Cargo workspace
├── specs/              # WIT / Arrow schema / Tauri commands schema
├── tests/              # Rust 集成测试 + contract test
├── reference/          # ★ 旧 Python(read-only,本地 dev oracle 用)
├── deploy/             # 仅 engine-cli headless 模式用(可选)
├── scripts/            # mock 数据生成 + E2E 脚本
├── doc/                # 17+ 设计文档
├── datas/              # 原始 CSV 数据
├── Cargo.toml          # workspace root(engine/* + desktop/src-tauri + modules/*)
├── Makefile            # 跨语言统一入口
├── README.md
├── CLAUDE.md
└── .gitignore
```

### 选择说明

| 名 | 改自 | 理由 |
|---|---|---|
| `engine/` | (新增) | Rust 内核,workspace 容纳 8 个 crate |
| `desktop/` | `frontend/` | 名字反映物质形态(桌面 app 不是 Web 前端) |
| `modules/` | (新增) | WASM workspace,与 engine 隔离 target |
| `specs/` | (新增) | 跨语言契约单一真相源 |
| `reference/` | `backend/` | 显式标 read-only,完成 Rust 复刻后整个删除 |
| `deploy/` | (新增) | 仅 headless 模式需要;Tauri 桌面无需 |

**`Cargo.toml` 在顶层**:workspace 统一 engine / desktop / modules(modules 由于 target 不同实际分子 workspace,见 §3)。`cargo build` 在顶层一次出 engine + Tauri binary。

## 2. `engine/` — Rust 内核

Cargo sub-workspace:

```
engine/
├── crates/
│   ├── engine-core/        # Fact 总线 + canonical store(Arrow)
│   ├── engine-identity/    # Identity Resolver(DataFusion SQL)
│   ├── engine-wasm/        # wasmtime runtime + capability injection
│   ├── engine-recovery/    # PRD-001 port(Phase 3)
│   ├── engine-changes/     # PRD-002 port(Phase 3)
│   ├── engine-reports/     # PRD-003 port(Phase 4)
│   ├── engine-storage/     # ★ SQLite + Parquet + Neo4j adapter(可选)
│   ├── engine-bindings/    # wasmtime 生成的 host bindings(从 specs/wit)
│   ├── engine-testkit/     # 测试 fixtures + contract runner
│   └── engine-cli/         # ★ headless binary(REST + Arrow Flight)
├── benches/                # criterion 性能 baseline
├── tests/                  # 集成测试(端到端 / contract)
└── README.md
```

**Tauri app 嵌入 engine-core**(不嵌入 engine-cli),通过 `desktop/src-tauri/Cargo.toml` path dep:

```toml
[dependencies]
engine-core = { path = "../../engine/crates/engine-core" }
engine-storage = { path = "../../engine/crates/engine-storage" }
engine-wasm = { path = "../../engine/crates/engine-wasm" }
# 不依赖 engine-cli,因为 Tauri 自己是 binary
```

### 关键设计

| 项 | 选择 | 理由 |
|---|---|---|
| Tauri 嵌入方式 | engine-core 作 lib,Tauri commands 调函数 | 零网络栈,IPC 是 Tauri 自带 |
| engine-cli 单独 binary | 共用 engine-core,只加 REST/Flight 服务 | 团队/SaaS 模式留口子 |
| engine-storage 抽象 | trait + 3 实现:SQLite / Parquet / Neo4j | Tauri 默认 SQLite,headless 可选 Neo4j |
| WIT bindings 生成 | engine-bindings 单 crate,统一处 | wit 变更只重建一个 crate |
| 异步运行时 | tokio | 与 wasmtime / arrow-flight / sqlx 同源 |
| 内存分配器 | mimalloc | 桌面跨平台优;jemalloc Windows 弱 |

### `engine/Cargo.toml`(sub-workspace)

```toml
[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.package]
edition = "2021"
rust-version = "1.83"
license = "MIT OR Apache-2.0"

[workspace.dependencies]
# Internal
engine-core      = { path = "crates/engine-core" }
engine-identity  = { path = "crates/engine-identity" }
engine-wasm      = { path = "crates/engine-wasm" }
engine-recovery  = { path = "crates/engine-recovery" }
engine-changes   = { path = "crates/engine-changes" }
engine-reports   = { path = "crates/engine-reports" }
engine-storage   = { path = "crates/engine-storage" }
engine-bindings  = { path = "crates/engine-bindings" }
engine-testkit   = { path = "crates/engine-testkit" }

# External(见 doc/15 §7.1)
tokio        = { version = "1.40", features = ["full"] }
arrow        = "54"
arrow-flight = "54"
datafusion   = "44"
wasmtime     = "23"
wasmtime-wasi = "23"
axum         = "0.7"
tonic        = "0.12"
sqlx         = { version = "0.8", features = ["sqlite", "runtime-tokio"] }
parquet      = "54"
neo4rs       = { version = "0.8", optional = true }
serde        = { version = "1", features = ["derive"] }
serde_json   = "1"
tracing      = "0.1"
anyhow       = "1"
thiserror    = "1"
mimalloc     = "0.1"

[profile.release]
lto = "thin"
codegen-units = 1
strip = true
```

## 3. `desktop/` — Tauri 桌面应用

Tauri 2.x 标准布局:

```
desktop/
├── src/                    # React + TypeScript 源(可从 frontend/ 迁 ~90% 代码)
│   ├── components/         # 6 视图 + 4 PRD 视图(沿用现有)
│   │   ├── Graph/         # GraphCanvas / NodeDetailPanel / LayerToggle
│   │   ├── Views/         # 各视图组件
│   │   ├── Recovery/      # 审批 / 执行 / 链
│   │   └── Layout/        # MainLayout
│   ├── api/
│   │   ├── client.ts       # ★ axios → tauri invoke 替换
│   │   └── generated.ts    # ★ tauri-specta 自动生成的 TS 类型
│   ├── hooks/             # useGraphData 等
│   ├── utils/             # graphStyles / layers / resourceIcons
│   ├── App.tsx
│   └── main.tsx
├── src-tauri/              # Rust 部分(Tauri backend)
│   ├── Cargo.toml          # depends on engine-core / engine-wasm / engine-storage
│   ├── tauri.conf.json     # 应用元数据 / 窗口配置 / 权限
│   ├── build.rs
│   ├── src/
│   │   ├── main.rs
│   │   ├── commands/       # tauri commands(按领域分文件)
│   │   │   ├── mod.rs
│   │   │   ├── topology.rs    # invoke('topology_view', ...)
│   │   │   ├── recovery.rs    # invoke('execute_recovery', ...)
│   │   │   ├── change_events.rs
│   │   │   ├── reports.rs
│   │   │   └── connectors.rs
│   │   ├── state.rs        # AppState(engine handle)
│   │   └── menu.rs         # 系统菜单 / tray
│   └── icons/
├── package.json            # React + AntD + Cytoscape + tauri-apps/api
├── vite.config.ts
├── tsconfig.json
└── README.md
```

### 关键设计

| 项 | 选择 | 理由 |
|---|---|---|
| Tauri 版本 | **2.x**(2024 GA) | 1.x EOL,2.x 是长期方向 |
| 前端框架 | **React 18 + TS + Vite** | 沿用现 frontend/ |
| UI 组件 | **Ant Design 5** | 沿用 |
| 图可视化 | **Cytoscape.js + dagre** | 沿用 |
| 状态 | **TanStack Query**(替代部分手动 hook) | 自动缓存 / 失效 |
| TS 类型生成 | **tauri-specta 2.x** | 从 Rust struct 自动出 TS |
| 系统集成 | **tauri-plugin-{notification,tray,updater,fs}** | 桌面 app 标配 |
| 包大小目标 | < 30 MB(无 NodeJS runtime) | Tauri 默认 |

详细 Tauri 架构(commands / IPC / 安全模型 / 打包)见 [`17-tauri-desktop-architecture.md`](./17-tauri-desktop-architecture.md)。

## 4. `modules/` — WASM 插件

独立 Cargo workspace(避免与 host 编译目标冲突):

```
modules/
├── Cargo.toml              # workspace,target = wasm32-wasip2
├── rust-toolchain.toml     # wasm32-wasip2 target
├── manifest.toml           # 模块清单(引擎启动读)
├── sdk/                    # guest 端 WIT bindings + 通用工具
├── connectors/
│   ├── k8s/
│   ├── prometheus/
│   ├── jaeger/
│   ├── flagd/
│   └── coderepo/
├── rules/
│   ├── threshold-check/
│   ├── slo-evaluation/
│   └── outbound-http-detection/
└── handlers/
    └── examples/
        └── custom-restart/
```

### `modules/Cargo.toml`

```toml
[workspace]
resolver = "2"
members = ["sdk", "connectors/*", "rules/*", "handlers/*"]

[workspace.package]
edition = "2021"

[workspace.dependencies]
wit-bindgen = "0.30"
serde       = { version = "1", default-features = false, features = ["derive"] }
serde_json  = { version = "1", default-features = false, features = ["alloc"] }
module-sdk  = { path = "sdk" }

[profile.release]
opt-level = "s"
lto       = true
strip     = true
panic     = "abort"
```

### `modules/manifest.toml`(引擎启动读)

```toml
schema_version = "1"

[[modules]]
name = "k8s-connector"
type = "connector"
wasm_path = "connectors/k8s/target/wasm32-wasip2/release/k8s_connector.wasm"
version = "0.1.0"
capabilities = ["kubernetes-readonly"]
sync_interval_seconds = 30
wasi_version = "p2"          # 默认 p2;async-native 模块切 "p3"
sha256 = "..."

[[modules]]
name = "threshold-rule"
type = "rule"
wasm_path = "rules/threshold-check/target/wasm32-wasip2/release/threshold_rule.wasm"
version = "0.1.0"
capabilities = []
applies_to = ["MetricSnapshot"]
sha256 = "..."
```

### 4.x WASI ABI 演进策略(p2 → p3)

**当前事实**(2026-06 锁定):

| 维度 | 状态 |
|---|---|
| **wasmtime host** | v46.0.0+ 默认支持 WASI 0.3.0 + `component-model-async` |
| **`wasm32-wasip2`** | Tier 2 stable rustc target,`rustup target add` 直接装,完整 std |
| **`wasm32-wasip3`** | **Tier 3** — 不在 stable rustup,需 nightly + `-Z build-std` + wasi-sdk 22+;rustc 标准库实际尚未切到 wasip3 API;libc 需 `[patch]` 注入 |
| **WASIp3 spec** | 仍在 WASI subgroup 终批,预期 2026 年内定稿 |

**因此双轨**:

1. **默认 = p2**(生产路径)
   - `modules/manifest.toml` 每个模块 `wasi_version = "p2"`
   - CI `modules-wasip2` job 用 stable rustc,产物 `hello_world.wasm` 上传 artifact
   - 引擎 `engine-wasm::ModuleManifest::wasi_version` 默认值 `P2`(`#[serde(default)]`)

2. **opt-in = p3**(async-native 模块)
   - 模块在自己的 `manifest.toml` 条目里写 `wasi_version = "p3"`
   - 编译指令:`cargo +nightly build --target wasm32-wasip3 -Z build-std=std,panic_abort`
   - CI `modules-wasip3` job 跑 nightly,`continue-on-error: true`(不阻塞 PR)
   - 引擎启动时检测 `wasi_version=P3` → 校验当前 wasmtime build 是否含 `component-model-async`(46.0+ 全有)

**升 default 到 p3 的触发条件**(任一满足):
- `wasm32-wasip3` 升 Tier 2 stable,`rustup target add` 直接能装
- rustc std 完成 wasip3 切换(`std::os::wasi::p3::*` 真用 WASIp3 syscall)
- 当前能落地的 WASM 模块里 ≥1 个真需要 async/future(例:Jaeger streaming pull)

满足后:
- 改 `engine-wasm::WasiVersion::default` 到 `P3`
- CI `modules-wasip3` 删 `continue-on-error`,toolchain 换 stable
- `specs/version.toml` `[wasi].default = "p3"`
- 现有模块按需迁(p2 模块在 wasip3 runtime 仍跑 — 向后兼容)

## 5. `specs/` — 中立契约

```
specs/
├── wit/                    # Component Model
│   ├── types.wit
│   ├── host.wit            # ★ host-capabilities world(host 端 bindgen 用)
│   ├── connector.wit       # connector-world + 3 capability interface(共享)
│   ├── rule.wit            # rule-world
│   ├── handler.wit         # handler-world
│   └── README.md           # WIT 演化规则
├── arrow/                  # Arrow schema(机读 JSON)
│   ├── fact_v1.json
│   └── metric_snapshot_v1.json
├── tauri-commands/         # ★ Tauri commands 类型定义
│   ├── topology.ts         # 自动生成(specs-generate-tauri-types)
│   ├── recovery.ts
│   └── ...
├── openapi/                # ★ 仅 engine-cli headless 用
│   └── engine.json         # 自动生成
└── version.toml            # 跨组件版本锁
```

### Host vs Guest WIT bindgen 分工

参考 ntx/show 的 `hostnet` world 模式(`/home/cc/Desktop/code/ntx/show/ntxdemo/component/wit/host/world.wit`):

| 端 | 用谁 bindgen | bindgen 哪个 world | 目的 |
|---|---|---|---|
| **host** | `wasmtime::component::bindgen!` 宏(`engine-bindings` crate) | `sre:inspection/host-capabilities@0.1.0`(`specs/wit/host.wit`)+ 每个 guest world(connector-world / rule-world / handler-world)| 一次 `HostCapabilities::add_to_linker(linker, ...)` 接全 capability,再用 `XxxWorld::instantiate_async(...)` 强类型调 guest exports |
| **guest** | `wit_bindgen::generate!` 宏(各 module crate 内) | 仅它要 export 的那个 world(connector-world / rule-world / handler-world)— 同时声明它 import 的 capability interfaces 子集 | 给本 module 生成 host import stubs + guest export trait |

**互不冲突**:同一个 `.wit` 文件被两端解析两次,产物分别是 host glue 和 guest stubs,wit-bindgen / wasmtime-bindgen 各自处理。

`host-capabilities` world 是**纯聚合**(无 export),专为 host 端"一次接全部 capability 到 linker"服务。新增 capability(如 `metric-emit` / `k8s-readonly`)在 `host.wit` 加一行 `import`,engine-bindings 自动跟进。

### `specs/version.toml`

```toml
schema_version = "0.1.0"

[components]
wit              = "0.1.0"
arrow_schema     = "0.1.0"
tauri_commands   = "0.1.0"
openapi          = "0.1.0"   # 仅 headless

[crate_baseline]
arrow            = "54"
arrow-flight     = "54"
datafusion       = "44"
wasmtime         = "23"
wit-bindgen      = "0.30"
tokio            = "1.40"
axum             = "0.7"
tauri            = "2.0"
tauri-specta     = "2.0"
sqlx             = "0.8"

[reference_python_baseline]   # reference/ 仓的依赖,不再升级,锁定
fastapi          = "0.110"
pydantic         = "2.6"
```

## 6. `tests/` — 单一 Rust 测试栈

```
tests/
├── README.md
├── contract/               # ★ 按 PRD 组织
│   ├── prd-001-recovery/
│   │   ├── scale_deployment.rs
│   │   ├── restart_pod.rs
│   │   └── ...
│   ├── prd-002-changes/
│   ├── prd-005-facts/
│   └── prd-006-coderepo/
├── e2e/                    # 跨 crate 端到端
│   ├── fact_to_node.rs     # connector → fact bus → resolver → store
│   ├── wasm_connector.rs   # 加载 .wasm,sync,验证产出
│   └── recovery_chain.rs
└── fixtures/
    ├── mock_k8s_responses/
    ├── sample_traces.json
    └── ...
```

**Supervised 模式下 contract test 是单 Rust runner**,不双跑。Python reference 在本地 dev 时手动跑 + curl 对比,**不进 CI**。

### Contract test 写法(简化)

```rust
// tests/contract/prd-001-recovery/scale_deployment.rs
use engine_testkit::contract::{run_contract, ContractCase};

#[test]
fn scale_deployment_low_risk_sync() {
    run_contract(ContractCase {
        name: "scale_deployment_low_risk_sync",
        action_id: "scale_deployment",
        target_id: "deploy:vm-cluster:otel-demo:cart",
        params: serde_json::json!({"replicas_delta": 1}),
        expected_status: "succeeded",
        expected_dss_change: |dss| {
            assert_eq!(dss.get_node("deploy:vm-cluster:otel-demo:cart").replicas, 4);
        },
    });
}
```

## 7. `reference/` — Python read-only 仓

`backend/` 改名而来,加 README 明确状态:

```
reference/
├── README.md               # ★ DO NOT DEPLOY,只本地 dev 跑做 oracle
├── MIGRATION_STATUS.md     # ★ 各模块复刻状态表
├── app/                    # 原 backend/app/
├── tests/                  # 原 472 测试
├── pyproject.toml
└── uv.lock
```

### `reference/README.md` 内容要点

```markdown
# Reference Python Implementation (read-only)

This directory contains the **legacy Python implementation** of the SRE
Inspection Graph platform. As of T+0 of doc/14 v0.2, it is **read-only**:

- ❌ Do NOT deploy this code anywhere
- ❌ Do NOT add features
- ❌ Do NOT fix bugs (unless blocking dev verification)
- ✅ Run locally with `uv run uvicorn ...` for oracle/diff purposes
- ✅ Update MIGRATION_STATUS.md as Rust modules complete

This directory will be `git rm`'d at T+12mo when Rust complete.
```

### `reference/MIGRATION_STATUS.md`(每复刻一个模块更新一次)

```markdown
| Module | Python File | Rust Replacement | Status |
|---|---|---|---|
| K8s connector | `app/datasource/connectors/k8s_connector.py` | `modules/connectors/k8s/` | ✅ Phase 1 |
| Prometheus connector | `app/datasource/connectors/prometheus_connector.py` | `modules/connectors/prometheus/` | ✅ Phase 2 |
| Identity Resolver | (新功能) | `engine/crates/engine-identity/` | ✅ Phase 2 |
| Recovery engine | `app/recovery/execution.py` | `engine/crates/engine-recovery/` | 🚧 Phase 3 |
| ChangeEvent service | `app/changes/event_service.py` | `engine/crates/engine-changes/` | 🚧 Phase 3 |
| Report engine | `app/reports/generator.py` | `engine/crates/engine-reports/` | ⏳ Phase 4 |
| ...
```

## 8. `deploy/` — 仅 headless 模式

Tauri 桌面不需要 docker-compose;只有想跑 engine-cli 作 SaaS 后端时用:

```
deploy/
├── README.md               # 标 "仅用于 headless / 团队部署模式"
├── docker-compose.yml      # engine-cli + 可选 Neo4j
├── Dockerfile.engine       # 仅 engine-cli
├── k8s/                    # 未来 k8s manifests
└── README.md
```

**Tauri 桌面用户根本看不到这个目录的存在**。

## 9. 顶层 `Cargo.toml`(virtual workspace)

```toml
[workspace]
resolver = "2"
members = [
    "engine/crates/*",
    "desktop/src-tauri",
    # modules/* 是独立 workspace(target 不同),不挂这里
]
exclude = ["modules", "reference"]

[workspace.package]
edition = "2021"
rust-version = "1.83"

# 共享依赖在子 workspace 各自管(engine/Cargo.toml + modules/Cargo.toml)
```

**单一 `cargo build` 出 engine binaries + Tauri binary**;`cd modules && cargo component build` 出 WASM 模块。

## 10. 顶层 Makefile

```makefile
# ── 老的(在 reference/ 跑) ──────────────
ref-up:
	cd reference && uv run uvicorn app.main:app --reload --port 8000

ref-test:
	cd reference && uv run pytest -p no:asyncio

# ── 新的 Rust + WASM + Tauri ────────────
# Engine
engine-build:
	cargo build --release --workspace

engine-test:
	cargo test --workspace --exclude tauri-app

engine-clippy:
	cargo clippy --workspace --all-targets -- -D warnings

engine-fmt:
	cargo fmt --all

engine-cli-dev:
	cargo run --bin engine-cli -- serve

# Desktop (Tauri)
desktop-dev:
	cd desktop && npm run tauri dev

desktop-build:
	cd desktop && npm run tauri build

desktop-test:
	cd desktop && npm test

# WASM modules
modules-build:
	cd modules && cargo component build --release --workspace

modules-build-one:
	cd modules/$(MOD) && cargo component build --release

# Specs / type generation
specs-validate:
	wasm-tools component wit specs/wit/
	cargo run --bin specs-validator       # 自建小工具

specs-generate-tauri-types:
	cargo run --bin export-tauri-types --features=specta

specs-generate-openapi:
	cargo run --bin engine-cli -- --dump-openapi > specs/openapi/engine.json

# Contract test (single Rust runner)
contract-test:
	cargo test --test contract

# Everything
test-all: engine-test desktop-test modules-build contract-test
build-all: engine-build modules-build desktop-build

# Clean
clean:
	cargo clean
	cd modules && cargo clean
	cd desktop && rm -rf node_modules dist
```

## 11. CI(GitHub Actions)

```
.github/workflows/
├── engine.yml          # cargo test + clippy + fmt(paths: engine/**, specs/**)
├── modules.yml         # cargo component build(paths: modules/**, specs/wit/**)
├── desktop.yml         # npm + tauri build(paths: desktop/**)
├── specs.yml           # wit 校验 + arrow schema 校验(paths: specs/**)
├── contract.yml        # 单 Rust runner(paths: engine/**, tests/contract/**)
├── release.yml         # 跨平台 tauri build(tag-triggered)
└── docs.yml            # markdown lint(paths: doc/**)
```

**`reference/` 不进 CI**(只本地 dev 用);如果开发过程中要跑,本地手动 `make ref-up`。

## 12. Bootstrap 步骤(Phase 0 末尾执行)

```bash
# Step 1 — 重命名既有目录(保留 git history)
git mv backend reference
git mv frontend desktop          # 留作 Tauri 集成壳
echo "# Reference Python Implementation (read-only)" > reference/README.md
# ↑ 加完整 README 内容(§7)
echo "# Migration Status" > reference/MIGRATION_STATUS.md

# Step 2 — 顶层目录骨架
mkdir -p engine/{crates,benches,tests} \
         modules/{sdk,connectors,rules,handlers} \
         specs/{wit,arrow,tauri-commands,openapi} \
         tests/{contract,e2e,fixtures} \
         deploy

# Step 3 — Cargo workspace 骨架
cat > Cargo.toml <<'EOF'
[workspace]
resolver = "2"
members = ["engine/crates/*", "desktop/src-tauri"]
exclude = ["modules", "reference"]
EOF

# Step 4 — engine crates
cd engine
cargo new --lib crates/engine-core
cargo new --lib crates/engine-identity
cargo new --lib crates/engine-wasm
cargo new --lib crates/engine-storage
cargo new --lib crates/engine-bindings
cargo new --lib crates/engine-testkit
cargo new --bin crates/engine-cli
# Phase 3 时再加 engine-recovery / engine-changes / engine-reports
cd ..

# Step 5 — modules workspace
cd modules
cat > Cargo.toml <<'EOF'
[workspace]
resolver = "2"
members = ["sdk", "connectors/*", "rules/*"]
EOF
cargo new --lib sdk --vcs none
cd ..

# Step 6 — Tauri 应用骨架(Phase 1 开始时填,这里只占位)
# cd desktop && npm create tauri-app@latest .
# 选 React + TypeScript

# Step 7 — WIT 文件
# 从 doc/15 §1 拷贝完整 wit 内容到 specs/wit/

# Step 8 — Makefile + .gitignore 扩展
# Step 9 — git add + commit
git add .
git commit -m "feat: bootstrap Rust+WASM+Tauri layout per doc/16 v0.2"
```

预计 1-2 小时跑完。

## 13. `.gitignore` 扩展

```gitignore
# ── 既有 ──
__pycache__/
*.pyc
.pytest_cache/
node_modules/
.env

# ── Rust ──
target/
**/*.rs.bk

# ── WASM 产物 ──
modules/**/target/
modules/**/*.wasm

# ── Tauri ──
desktop/src-tauri/target/
desktop/dist/
desktop/.vite/

# ── Specs 自动生成 ──
specs/openapi/engine.json    # 自动生成,但 commit 当快照
specs/tauri-commands/*.ts    # 自动生成,commit 当快照
# ↑ 策略:commit,CI 校验"重新生成与 commit 一致"

# ── 本地 dev ──
reference/.uv-cache/
*.db
*.sqlite
*.parquet
```

**Cargo.lock 策略**:
- 顶层 `Cargo.lock` → commit(workspace 锁)
- `modules/Cargo.lock` → commit
- `desktop/src-tauri/Cargo.lock` → 由顶层 workspace 管,不单独存

## 14. 命名约定

| 范畴 | 约定 | 示例 |
|---|---|---|
| Rust crate(engine) | `engine-{role}` | `engine-core`, `engine-storage` |
| Rust crate(WASM 模块) | `{type}-{name}` | `k8s-connector`, `threshold-rule` |
| WASM 产物文件名 | `{snake_case}.wasm` | `k8s_connector.wasm` |
| WIT package | `sre:topology@{semver}` | `sre:topology@0.1.0` |
| WIT interface | kebab-case 复数 | `fact-emitter`, `rule` |
| Arrow schema 文件 | `{entity}_v{n}.json` | `fact_v1.json` |
| Tauri command | snake_case 动词 | `execute_recovery`, `list_executions` |
| Tauri events | snake_case 名词 | `fact_emitted`, `connector_synced` |

## 15. 多语言协作日常工作流

### 场景 A:加一个新视图(纯 desktop)

```
1. cd desktop && touch src/components/Views/NewView.tsx
2. 写 React 组件,调 invoke('existing_cmd') 拿数据
3. npm test
4. make desktop-dev 看效果
```

无需碰 Rust。

### 场景 B:加一个新 Tauri command(UI 要新数据)

```
1. desktop/src-tauri/src/commands/topology.rs 加 #[tauri::command] 函数
2. 函数内调 engine-core 已有 API,或新加 engine-core 函数
3. make specs-generate-tauri-types 重新生成 TS 类型
4. desktop/src/api/client.ts 加 invoke wrapper
5. 视图调用
6. cargo test + npm test
```

### 场景 C:复刻 reference 一个 Python 模块到 Rust

```
1. cd reference && code app/changes/event_service.py(读懂行为)
2. cd reference && uv run uvicorn ...     # 终端 1
3. cd engine && cargo run --bin engine-cli -- serve   # 终端 2
4. 写 engine-changes crate 实现等价逻辑
5. 写 tests/contract/prd-002-changes/*.rs(覆盖关键行为)
6. curl 对比两侧输出
7. cargo test --test contract
8. 更新 reference/MIGRATION_STATUS.md 标记 ✅
```

### 场景 D:新加 WASM 模块(如 Cloud API connector)

```
1. cd modules/connectors && cargo new --lib aws-cloud
2. 改 Cargo.toml: [lib] crate-type = ["cdylib"];depends on module-sdk
3. src/lib.rs 实现 Guest trait(从 specs/wit/connector.wit 生成)
4. cargo component build --release
5. 加 modules/manifest.toml entry,填 sha256
6. make engine-cli-dev 验证加载
```

## 16. 反模式(不要做)

| ❌ 别做 | ✅ 该做 |
|---|---|
| `reference/` 改任何 feature | 只本地 dev 跑,纪律性 read-only |
| Tauri 里又起一个 HTTP server | 直接 invoke,IPC 是 Tauri 自带优势 |
| 在 desktop/ 写业务逻辑 | 业务逻辑在 engine-core;Tauri commands 是薄包装 |
| WASM 模块直接 syscall | host 注入 capability,deny by default |
| `protoc` / protobuf | 决策已定:不用 |
| Cargo.lock 不 commit | binary 项目必须 commit |
| WIT 改字段名而不升 major | 走演化规则 |
| `npm install` 安装 Tauri 依赖之外的 native module | Tauri webview 跑 JS,Rust 那边的 native 走 src-tauri |
| 跨平台路径硬编码 | 用 `dirs` crate / Tauri path API |
| WASM 模块产物 .wasm 入库 | build artifact 不入库,release 时单独签名分发 |

## 17. 现有目录清理(Bootstrap 时一并)

| 现有 | 操作 |
|---|---|
| `backend/` | `git mv` 到 `reference/`,加 README |
| `frontend/` | `git mv` 到 `desktop/`(留作 Tauri 集成壳) |
| `data/` | 删(与 `datas/` 重复,留 `datas/`) |
| `node_modules/`(顶层) | 删(应在 desktop/ 内) |
| `.pytest_cache/`(顶层) | 删 |
| `product/` | 看一眼,挪到 `doc/archive/` 或删 |
| 顶层 `docker-compose.yml` | `git mv` 到 `deploy/`(仅 headless 模式) |

## 18. 相关文档

- 上游战略:[`14-long-term-tech-strategy.md`](./14-long-term-tech-strategy.md)
- 数据契约:[`15-data-contract-spec.md`](./15-data-contract-spec.md)
- Tauri 架构详细:[`17-tauri-desktop-architecture.md`](./17-tauri-desktop-architecture.md)
- 导航:[`00-README.md`](./00-README.md)

---

**版本**:v0.2.0 — 2026-06-23,Supervised Rewrite + Tauri 桌面化后的物理结构。v0.1 Strangler Fig 双语言并存设计已废弃。
