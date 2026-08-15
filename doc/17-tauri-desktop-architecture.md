# 17 — Tauri 桌面架构

## 0. 上下文

本平台产品形态为 **Tauri 2.x 桌面应用**(不走 SaaS Web 默认路径);仓库布局见根 `README.md`。本文是 **Tauri 桌面层的具体技术规约**,覆盖 webview ↔ Rust ↔ engine-core 的边界设计、安全模型、打包发布。

> 这是一篇技术规约文档,所有写 Tauri commands、调 invoke()、配置 Tauri plugin、做跨平台打包的代码都应对照本文。

## 1. 决策摘要

| 项 | 选择 |
|---|---|
| Tauri 版本 | **2.x**(2024-09 GA) |
| Webview 引擎 | macOS WKWebView / Linux WebKitGTK / Windows WebView2(WebView2 自动安装) |
| 前端栈 | React 18 + TS + Vite + AntD 5 + Cytoscape.js + dagre(沿用 frontend/) |
| Rust ↔ TS 类型生成 | **tauri-specta 2.x** |
| 状态管理 | TanStack Query(server state)+ Zustand(UI state,可选) |
| 系统集成 | tauri-plugin-{notification, tray, updater, fs, dialog, shell} |
| 本地存储 | engine-storage crate(SQLite + Parquet,见 §6) |
| 打包目标 | macOS .dmg / Linux .AppImage + .deb / Windows .msi |
| 自动更新 | tauri-plugin-updater + GitHub Releases |
| 签名 | macOS notarization + Windows Authenticode(Phase 4 必做) |
| 包大小目标 | < 30 MB(壳)+ 20-50 MB engine binaries 嵌入 |

## 2. 进程与边界模型

```
┌─────────────────────────────────────────────────────────────┐
│  Tauri Main Process(Rust)                                  │
│                                                             │
│   ┌─────────────────────────────────────────────┐          │
│   │  Webview Process(隔离进程,操作系统提供)    │          │
│   │   • React 应用(JS runtime)                 │          │
│   │   • Cytoscape.js 渲染                       │          │
│   │   • 所有 UI 逻辑                            │          │
│   │   • 通过 window.__TAURI__.invoke 与 main 通信│          │
│   └────────────────┬────────────────────────────┘          │
│                    │ Tauri IPC(异步 JSON,序列化在 Rust)  │
│                    ▼                                       │
│   ┌─────────────────────────────────────────────┐          │
│   │  Tauri Backend(Rust 主进程)                │          │
│   │   • #[tauri::command] 函数                  │          │
│   │   • AppState(engine handle,Arc<RwLock<>>) │          │
│   │   • Plugin 注册(notification/tray/...)    │          │
│   │   • 系统菜单 / 托盘                          │          │
│   └────────────────┬────────────────────────────┘          │
│                    │ 同进程 Rust 函数调用                  │
│                    ▼                                       │
│   ┌─────────────────────────────────────────────┐          │
│   │  engine-core(Rust lib)                     │          │
│   │   • 业务逻辑,见 doc/16 §2                  │          │
│   │   • 启动时通过 AppState 初始化              │          │
│   └────────────────┬────────────────────────────┘          │
│                    │ wasmtime engine                       │
│                    ▼                                       │
│   ┌─────────────────────────────────────────────┐          │
│   │  WASM Modules                                │          │
│   └─────────────────────────────────────────────┘          │
└─────────────────────────────────────────────────────────────┘
```

**关键不变量**:

1. Webview 是**沙箱**,不能直接访问文件系统 / 网络 / OS API,只能调 Tauri commands
2. Tauri commands 是**白名单**,在 `tauri.conf.json` 配置 allowlist
3. engine-core **不知道**自己跑在 Tauri 里;只暴露 Rust API,Tauri commands 包装它
4. WASM 模块**不知道**自己跑在 Tauri 里;通过 WIT 与 host 交互
5. 所有 IPC 数据走 JSON 序列化;**Arrow RecordBatch 不跨 IPC**(保留在 Rust 侧,IPC 传 query 结果的 JSON 投影)

## 3. Tauri Commands 设计

### 3.1 命名约定

- snake_case 动词开头:`list_executions`, `execute_recovery`, `dry_run_action`
- 一律 async:`async fn list_executions(...) -> Result<Vec<Execution>, String>`
- 参数包成 struct(便于 specta 生成 TS):`#[derive(Serialize, Deserialize, specta::Type)]`
- 错误统一返 `Result<T, AppError>`,AppError 实现 `Serialize`

### 3.2 commands 模块组织

```rust
// desktop/src-tauri/src/commands/mod.rs
pub mod topology;
pub mod recovery;
pub mod change_events;
pub mod reports;
pub mod connectors;
pub mod fault_simulation;
pub mod system;       // 系统级:获取版本 / 配置 / 路径

// 在 main.rs 注册
tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![
        topology::get_topology_view,
        topology::get_node_detail,
        recovery::list_actions,
        recovery::dry_run_action,
        recovery::execute_action,
        recovery::rollback_execution,
        // ... 完整清单见各 commands 模块
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
```

### 3.3 标准 command 模板

```rust
// desktop/src-tauri/src/commands/recovery.rs
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::state::AppState;
use crate::error::AppError;

#[derive(Deserialize, Type)]
pub struct ExecuteActionArgs {
    pub action_id: String,
    pub target_id: String,
    pub params: serde_json::Value,
    pub verify: Option<bool>,
}

#[derive(Serialize, Type)]
pub struct ExecutionResult {
    pub execution_id: String,
    pub status: String,
    pub verify_status: Option<String>,
    pub approval_id: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn execute_action(
    args: ExecuteActionArgs,
    state: State<'_, AppState>,
) -> Result<ExecutionResult, AppError> {
    let engine = state.engine.read().await;
    let exec = engine.recovery
        .execute(&args.action_id, &args.target_id, args.params, args.verify.unwrap_or(true))
        .await?;
    Ok(ExecutionResult {
        execution_id: exec.execution_id,
        status: exec.status,
        verify_status: Some(exec.verify_status),
        approval_id: exec.approval_id,
    })
}
```

### 3.4 错误处理

```rust
// desktop/src-tauri/src/error.rs
use serde::Serialize;
use specta::Type;

#[derive(Debug, thiserror::Error, Serialize, Type)]
#[serde(tag = "kind", content = "message")]
pub enum AppError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("engine error: {0}")]
    Engine(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("approval required: {0}")]
    ApprovalRequired(String),
    #[error("internal: {0}")]
    Internal(String),
}

impl From<engine_core::Error> for AppError {
    fn from(e: engine_core::Error) -> Self {
        AppError::Engine(e.to_string())
    }
}
```

前端 TS 拿到的就是结构化对象:`{ kind: "ApprovalRequired", message: "..." }`,可 switch 分支。

### 3.5 AppState

```rust
// desktop/src-tauri/src/state.rs
use std::sync::Arc;
use tokio::sync::RwLock;

use engine_core::Engine;
use engine_storage::Storage;

pub struct AppState {
    pub engine: Arc<RwLock<Engine>>,
    pub storage: Arc<Storage>,
}

impl AppState {
    pub async fn init() -> anyhow::Result<Self> {
        let storage = Arc::new(Storage::open_default().await?);   // ~/.config/sre-graph/db.sqlite
        let engine = Arc::new(RwLock::new(
            Engine::builder()
                .storage(storage.clone())
                .with_wasm_runtime()
                .load_modules_from_manifest("modules/manifest.toml")?
                .build()
                .await?,
        ));
        Ok(Self { engine, storage })
    }
}
```

## 4. TS 类型自动生成(tauri-specta)

在每个 command 加 `#[specta::specta]`,然后一个 binary 跑导出:

```rust
// engine/crates/engine-cli/src/bin/export-tauri-types.rs
fn main() -> anyhow::Result<()> {
    use tauri_specta::{Builder, ExportLanguage};

    Builder::new()
        .commands(tauri_specta::collect_commands![
            crate::commands::topology::get_topology_view,
            crate::commands::recovery::execute_action,
            // ... 全部 command
        ])
        .export(
            specta_typescript::Typescript::default()
                .header("// Auto-generated by tauri-specta. DO NOT EDIT.\n"),
            "../../specs/tauri-commands/index.ts",
        )?;

    Ok(())
}
```

跑 `make specs-generate-tauri-types` → `specs/tauri-commands/index.ts` 出文件:

```typescript
// Auto-generated, do not edit
export type ExecuteActionArgs = {
    action_id: string;
    target_id: string;
    params: any;
    verify?: boolean | null;
};

export type ExecutionResult = {
    execution_id: string;
    status: string;
    verify_status?: string | null;
    approval_id?: string | null;
};

export type AppError =
    | { kind: "NotFound"; message: string }
    | { kind: "InvalidInput"; message: string }
    | { kind: "Engine"; message: string }
    // ...
```

前端封装:

```typescript
// desktop/src/api/client.ts
import { invoke } from '@tauri-apps/api/core';
import type { ExecuteActionArgs, ExecutionResult } from '../../specs/tauri-commands';

export async function executeAction(args: ExecuteActionArgs): Promise<ExecutionResult> {
    return invoke('execute_action', { args });
}
```

**`desktop/src/api/client.ts` 是 frontend 老代码迁移的唯一改点** — 把 `axios.post('/api/v1/recovery/execute', ...)` 改成 `invoke('execute_action', ...)`。其他组件代码基本不动。

## 5. Tauri 事件(server push)

某些场景需要 backend 主动推 UI(如 connector 同步完成、fact 新入图):

```rust
// Rust 侧 emit
use tauri::Emitter;

pub async fn run_connector_sync(app: &tauri::AppHandle, name: &str) {
    let result = engine.sync(name).await;
    app.emit("connector_synced", serde_json::json!({
        "connector": name,
        "result": result,
    })).ok();
}
```

```typescript
// TS 侧 listen
import { listen } from '@tauri-apps/api/event';

useEffect(() => {
    const unlisten = listen<{connector: string, result: any}>('connector_synced', (event) => {
        queryClient.invalidateQueries(['connector-status', event.payload.connector]);
    });
    return () => { unlisten.then(fn => fn()); };
}, []);
```

**事件命名**:snake_case 名词(`connector_synced`, `fact_emitted`, `execution_completed`)。

## 6. 本地存储设计

### 6.1 engine-storage crate

```rust
// engine/crates/engine-storage/src/lib.rs
pub trait Storage: Send + Sync {
    async fn save_execution(&self, exec: &Execution) -> Result<()>;
    async fn load_execution(&self, id: &str) -> Result<Option<Execution>>;
    async fn list_executions(&self, filter: ExecutionFilter) -> Result<Vec<Execution>>;
    // ... change_events / approvals / reports / facts(归档)
}

pub struct SqliteStorage { pool: sqlx::SqlitePool, ... }
pub struct ParquetArchive { base_dir: PathBuf, ... }

impl Storage for SqliteStorage { ... }
```

### 6.2 数据分层

| 数据 | 存哪 | 理由 |
|---|---|---|
| Canonical Graph(节点/边) | **内存**(Arrow RecordBatch) | 热路径,查询 < 10ms |
| Executions / Approvals / ChangeEvents | **SQLite** | 关系结构,事务安全,SQL 查询 |
| Fact 历史归档(可选) | **Parquet 文件**(按日分文件) | 列存,DuckDB 直接查,长期分析 |
| Connector 配置 / kubeconfig 引用 | **TOML 配置**(`~/.config/sre-graph/config.toml`) | 用户可手编辑 |
| Inspection findings | **SQLite** | 同 executions |
| 报告产物(.md 文件) | **本地文件**(`~/.local/share/sre-graph/reports/`) | 用户可直接打开 |
| 图数据库(可选) | **headless 模式**或用户显式启用 | 团队共享需求 |

### 6.3 数据库 schema(SQLite)

```sql
-- ~/.local/share/sre-graph/db.sqlite

CREATE TABLE executions (
    execution_id TEXT PRIMARY KEY,
    action_id TEXT NOT NULL,
    target_id TEXT NOT NULL,
    cluster_id TEXT,
    status TEXT NOT NULL,
    verify_status TEXT,
    initiated_by TEXT,
    initiated_at INTEGER NOT NULL,   -- unix ms
    completed_at INTEGER,
    params_json TEXT NOT NULL,
    result_json TEXT,
    rollback_id TEXT,
    chain_id TEXT,
    chain_step_index INTEGER
);
CREATE INDEX idx_exec_target ON executions(target_id);
CREATE INDEX idx_exec_status ON executions(status);
CREATE INDEX idx_exec_chain ON executions(chain_id);

CREATE TABLE change_events (
    change_event_id TEXT PRIMARY KEY,
    change_type TEXT NOT NULL,
    target_resource_id TEXT NOT NULL,
    severity_estimate TEXT NOT NULL,
    occurred_at INTEGER NOT NULL,
    -- ... diff_summary_json, propagated_to_json, commit_sha 等
);
CREATE INDEX idx_change_target ON change_events(target_resource_id);
CREATE INDEX idx_change_time ON change_events(occurred_at);

CREATE TABLE approvals (...);
CREATE TABLE inspection_findings (...);
CREATE TABLE report_subscriptions (...);
CREATE TABLE alert_events (...);
```

### 6.4 文件系统布局

```
~/.config/sre-graph/         # 配置
├── config.toml              # 主配置
├── kubeconfigs/             # k8s connector 用的 kubeconfig
└── manifest.toml            # WASM 模块清单(初始从 app bundle 拷)

~/.local/share/sre-graph/    # 数据
├── db.sqlite                # SQLite metadata
├── facts/                   # Parquet 归档
│   ├── 2026-07-15.parquet
│   └── ...
├── reports/                 # 生成的 .md 报告
├── modules/                 # 用户安装的额外 WASM 模块
└── cache/

~/.cache/sre-graph/          # 临时
└── ...
```

## 7. 安全模型

### 7.1 webview CSP(`tauri.conf.json`)

```json
{
  "app": {
    "security": {
      "csp": "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'",
      "freezePrototype": true,
      "dangerousDisableAssetCspModification": false
    }
  }
}
```

- 禁止外部脚本注入
- AntD 必须用 `'unsafe-inline'` style(可接受;Phase 4 评估改 CSS-in-JS hash)
- 不允许加载外部图片(除 `data:` base64)

### 7.2 Tauri allowlist(`tauri.conf.json`)

```json
{
  "tauri": {
    "allowlist": {
      "all": false,
      "fs": {
        "all": false,
        "readFile": true,
        "writeFile": true,
        "scope": ["$APPCONFIG/**", "$APPDATA/**"]
      },
      "shell": {
        "all": false,
        "open": true       // 仅允许打开外部 URL / 文件管理器
      },
      "dialog": {
        "all": false,
        "open": true,
        "save": true
      },
      "notification": { "all": true },
      "http": { "all": false }   // ★ 禁用,所有 HTTP 走 Rust 侧
    }
  }
}
```

**所有网络请求由 Rust 发,不让 webview 直接 fetch** — 这是 Tauri 关键安全优势。

### 7.3 WASM 模块沙箱

`engine-wasm` crate 实现 capability 注入:

```rust
// wasmtime 配置
let mut config = wasmtime::Config::new();
config.wasm_component_model(true);
config.async_support(true);

let engine = wasmtime::Engine::new(&config)?;
let mut linker = wasmtime::component::Linker::new(&engine);

// 注入 capability(白名单)
wasmtime_wasi::add_to_linker_async(&mut linker, |s: &mut WasmCtx| &mut s.wasi)?;

// 自定义 host 函数 — 比如 K8s API capability
linker.instance("sre:capabilities/kubernetes")?.func_wrap_async(
    "list_pods",
    |ctx, (namespace,): (String,)| { /* 调真实 K8s API */ },
)?;

// WASM 模块在 manifest.toml 声明 capabilities,host 启动时根据 manifest 决定是否 link
```

未在 manifest 声明的 capability → 不 link → WASM 调时 trap → 安全失败。

### 7.4 签名 / notarization(Phase 4 必做)

| 平台 | 工具 | 备注 |
|---|---|---|
| macOS | `codesign` + Apple Notary | 需要 Apple Developer 账号($99/年) |
| Windows | Authenticode | 需要代码签名证书 |
| Linux | GPG 签名 .AppImage | 可选 |

**没签名 macOS 用户运行会 Gatekeeper 拦截**,Linux 较宽松。Phase 1-3 可以不签(只自己 + 团队用),v1.0 release 必签。

### 7.5 自动更新签名

`tauri-plugin-updater` 用 minisign 签名 update manifest。Tauri config:

```json
{
  "plugins": {
    "updater": {
      "active": true,
      "endpoints": ["https://github.com/<user>/sre-graph/releases/latest/download/latest.json"],
      "dialog": true,
      "pubkey": "ABCDEF..."         // ★ minisign 公钥,build 时填
    }
  }
}
```

私钥保管在 GitHub secrets / 本地加密(`tauri signer generate`)。

## 8. 启动流程

```
1. Tauri main() ──────────────────────────────────────────┐
   • 读 ~/.config/sre-graph/config.toml                    │
   • 初始化 tracing-subscriber(写 ~/.local/share/.../logs)│
   • AppState::init() async ─────────────┐                 │
                                          │                 │
2. AppState::init() ◄─────────────────────┘                 │
   • Storage::open_default() ── 打开 SQLite,migrate schema │
   • Engine::builder() ─────────────────────┐              │
                                              │              │
3. Engine 启动 ◄─────────────────────────────┘              │
   • wasmtime engine 初始化                                  │
   • 读 modules/manifest.toml,校验 sha256,加载 .wasm      │
   • 启动 connector polling tasks(tokio spawn)            │
   • 启动 scheduler(报告订阅 / TTL 清理)                 │
                                                              │
4. Tauri Webview 启动 ◄───────────────────────────────────┘
   • 加载 dist/index.html(打包时 vite build 产物)
   • React 应用初始化
   • Query 第一个数据(invoke 'get_topology_view')
   • Cytoscape 渲染
```

启动时间目标:**冷启动 < 1.5s,热启动 < 0.5s**(macOS M 系列)。

## 9. 跨平台打包

### 9.1 macOS

```bash
cd desktop
npm run tauri build              # 默认产 .app + .dmg
# 产物:src-tauri/target/release/bundle/dmg/sre-graph_0.1.0_aarch64.dmg
```

- Universal binary:`--target universal-apple-darwin`(同时打 intel + arm64)
- notarization:`tauri.conf.json` 配 `macOSPrivateApi` + 签名身份

### 9.2 Linux

```bash
npm run tauri build              # 产 .deb + .rpm + .AppImage
# 产物:src-tauri/target/release/bundle/{deb,rpm,appimage}/...
```

- AppImage 是首选(无需 root,跨发行版)
- 依赖:`webkit2gtk-4.1`(Tauri 2.x)— Ubuntu 24.04 / Fedora 40+ 默认有

### 9.3 Windows

```bash
# 需在 Windows 上 build,或 GitHub Actions windows-latest runner
npm run tauri build
# 产物:src-tauri/target/release/bundle/msi/sre-graph_0.1.0_x64_en-US.msi
```

- WebView2 runtime 自动安装(Windows 11 自带,Windows 10 首次启动下载)
- MSI 签名 + UAC manifest

### 9.4 GitHub Actions release workflow

```yaml
# .github/workflows/release.yml
name: Release

on:
  push:
    tags: ['v*']

jobs:
  build:
    strategy:
      matrix:
        platform: [macos-latest, ubuntu-latest, windows-latest]
    runs-on: ${{ matrix.platform }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: actions/setup-node@v4
        with: { node-version: 20 }
      - name: Install Linux deps
        if: matrix.platform == 'ubuntu-latest'
        run: |
          sudo apt update
          sudo apt install -y libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev
      - name: Build modules
        run: cd modules && cargo component build --release --workspace
      - name: Tauri build
        uses: tauri-apps/tauri-action@v0
        env:
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
        with:
          tagName: ${{ github.ref_name }}
          releaseName: 'SRE Graph ${{ github.ref_name }}'
          releaseBody: 'See CHANGELOG.md'
          releaseDraft: true
```

## 10. 开发 / 调试

### 10.1 启动 dev 环境

```bash
make desktop-dev     # 等价 cd desktop && npm run tauri dev
```

- Vite HMR(React 改了即刻热更新)
- Tauri Rust 改了需要重启(`r` enter)
- DevTools:右键 → Inspect Element(Tauri 2 默认开启 webview devtools)
- Rust 日志:终端直接看(tracing 输出)

### 10.2 与 设计 对照验证

```bash
# 终端 1 — 对照基线
make ref-up

# 终端 2 — Tauri 桌面 app
make desktop-dev

# 终端 3 — 对比
curl http://localhost:8000/api/v1/topology > /tmp/py.json
# 在 desktop app 调相同视图,DevTools console 截 invoke 返回
diff <(jq -S . /tmp/py.json) <(jq -S . /tmp/rust.json)
```

### 10.3 测试

```bash
# Rust 单元 + 集成
make engine-test

# Rust Tauri commands 测试(用 mock AppState)
cd desktop/src-tauri && cargo test

# 前端
cd desktop && npm test

# Contract
make contract-test

# 全量
make test-all
```

## 11. 性能基线

| 指标 | 目标 | 测量方式 |
|---|---|---|
| 冷启动到首屏 | < 1.5s (M-series) / < 3s (Intel) | 手测 |
| invoke RTT(空 command) | < 1ms | bench |
| invoke RTT(典型 query) | < 10ms | bench |
| Cytoscape 渲染 200 节点 | < 200ms | browser perf |
| 1000 fact 入 store | < 50ms | criterion |
| BFS depth=4 on 10k 节点 | < 100ms | criterion |
| 包体积(macOS arm64) | < 30 MB(壳)+ engine 20-50 MB | 量产物 |
| 内存空载 | < 100 MB | activity monitor |
| 内存满载(10k 节点) | < 500 MB | 同上 |

Phase 1 收尾时定第一版基线,后续 Phase 不许回退超 10%。

## 12. 前端迁移路径(从 frontend/ → desktop/)

`frontend/src/` 90% 可迁,主要改 3 处:

### 12.1 API client 替换

```diff
- // frontend/src/api/client.ts
- import axios from 'axios';
- const api = axios.create({ baseURL: '/api/v1' });
- export const executeRecovery = (args) => api.post('/recovery/execute', args);

+ // desktop/src/api/client.ts
+ import { invoke } from '@tauri-apps/api/core';
+ import type { ExecuteActionArgs, ExecutionResult } from '../../specs/tauri-commands';
+ export const executeRecovery = (args: ExecuteActionArgs) =>
+     invoke<ExecutionResult>('execute_action', { args });
```

### 12.2 路由可选保留

React Router 沿用,Tauri 默认 hash router(避免 file:// 协议问题):

```tsx
// desktop/src/App.tsx
import { HashRouter, Routes, Route } from 'react-router-dom';
// 老 BrowserRouter 改 HashRouter,其他不变
```

### 12.3 文件下载

老代码用 `<a download>` blob:Tauri 用 `tauri-plugin-fs` + `tauri-plugin-dialog`:

```typescript
import { save } from '@tauri-apps/plugin-dialog';
import { writeTextFile } from '@tauri-apps/plugin-fs';

export async function downloadReport(content: string, filename: string) {
    const path = await save({ defaultPath: filename });
    if (path) {
        await writeTextFile(path, content);
    }
}
```

## 13. 系统集成功能(增量价值)

Tauri 解锁的功能 Web 版做不到:

| 功能 | 实现 | 价值 |
|---|---|---|
| 系统托盘 | `tauri-plugin-tray` | 后台跑 connector 同步 |
| 桌面通知 | `tauri-plugin-notification` | high_risk 动作待审批弹通知 |
| 全局快捷键 | `tauri-plugin-global-shortcut` | Cmd+Shift+I 打开主窗 |
| 文件拖入 | webview 原生 dragdrop event | 拖 kubeconfig 直接加 cluster |
| 深度链接 | `sre-graph://` URL scheme | 从 Slack 链接直接打开某 execution |
| 多窗口 | `tauri::WindowBuilder` | 主窗 + 报告独立窗 |
| 离线工作 | 默认 | 飞机上看历史归档 |

这些 **不是 Phase 1 必做**,但 Phase 2-3 逐步加,作为产品差异化。

## 14. 不做(本架构)

| 能力 | 理由 |
|---|---|
| 在 webview 跑 WASM 模块 | wasmtime 在 Rust 侧跑;webview 跑 WASM 性能差且无 capability 隔离 |
| webview 直接 fetch 外部 URL | 全走 Rust 侧 reqwest;CSP 拒绝 |
| 嵌入 Node.js runtime | Tauri 卖点就是无 Node;包体积优势 |
| 自己写 webview(Servo / CEF) | 用系统自带,跨平台差异 OS 处理 |
| webview 持久化(localStorage) | 用 SQLite,统一 |
| 多用户同机切换 | 单用户 / 一份配置;团队场景走 engine-cli |
| 安卓 / iOS(Tauri Mobile) | 不在 v1.0 范围;evaluate Phase 5+ |

## 15. 相关文档

- 仓库结构:见根 `README.md`「顶层结构」
- 数据契约(WIT/Arrow):[`15-data-contract-spec.md`](./15-data-contract-spec.md)
- 导航:[`00-README.md`](./00-README.md)

---

**版本**:v0.1.0 — 2026-06-23 初稿。Phase 1 实施时升 v0.2.0 加入实测数据。
