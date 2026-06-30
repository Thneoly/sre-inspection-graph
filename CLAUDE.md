# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

> **重要(2026-06)**:本仓已完成 **Python → Rust + WASM + Tauri 重写**的 Phase 1 纵切片(A→G + 最小拓扑视图 + Blog Part 1),准备进入 Phase 2(实数据 + 持久化 + Identity)。原 Python 实现(PRD-001/002/003/004,~12.7k LOC + 472 测试)已 100% 完成,**降为 `reference/` read-only oracle**,不接受 feature 改动 —— 本地可跑 FastAPI 对照 Rust 行为。**新代码看「活跃栈」章节**;旧栈细节看「Reference(Python,read-only)」章节,深入查 `reference/` 源码 + `doc/01-13` PRD。

## Project Overview

SRE 云原生巡检图谱平台 — cloud-native resource inspection graph platform。Phase 1 重写后核心:**Tauri 2.x 桌面应用** +  **Rust engine + WASM connectors** + **WIT/Arrow 契约**。原 Python 栈覆盖 4 层 Neo4j 图模型 + 故障模拟 + 恢复动作引擎(PRD-001)+ 变更追踪(PRD-002)+ 自检报告(PRD-003)+ 5 个 OTel-Demo connector(PRD-004),现作行为参考。

---

## 活跃栈(Rust + WASM + Tauri,新代码以此为准)

### 战略决策(详见 doc/14 v0.2)

- **Supervised Rewrite**,否决 Strangler Fig(副业不需要零停机,桥接代价不抵)
- **Tauri 2.x 桌面优先**,不走 SaaS Web 默认(对照 k9s / Lens;数据不出本机)
- **三层数据契约**:WIT(WASM 边界)+ Tauri commands(UI↔Rust 进程内 IPC)+ Arrow/SQLite/Parquet(存储)。**无 REST / Flight 跨进程 RPC**(headless engine-cli 才用)
- WASI ABI:**默认 p2**(`wasm32-wasip2` Tier 2 stable),p3(async-native)opt-in 留口子,详见 doc/16 §4.x

### 顶层结构

```
graph_data/
├── engine/             # Rust workspace — engine 内核 + CLI(10 crate)
│   └── crates/
│       ├── engine-core/        # ✅ canonical Fact + Arrow Schema(7 列)+ FactBatch→RecordBatch + GraphResponse(facts_to_graph)
│       ├── engine-wasm/        # ✅ wasmtime host + 多 connector 编排 + capability 注入(含 http-client)
│       ├── engine-bindings/    # ✅ wasmtime bindgen 出来的 host glue
│       ├── engine-storage/     # ✅ Storage trait + SqliteStorage(raw Fact 落库,Phase 2.1)
│       ├── engine-cli/         # ✅ headless binary(tick 子命令)
│       ├── engine-testkit/     # 骨架
│       ├── engine-identity/    # 骨架(Phase 2.5:Identity Resolver)
│       ├── engine-recovery/    # 骨架(Phase 3:PRD-001 复刻)
│       ├── engine-changes/     # 骨架(Phase 3:PRD-002 复刻)
│       └── engine-reports/     # 骨架(Phase 4:PRD-003 复刻)
├── desktop/            # Tauri 2.x 桌面(React 18 + AntD + Cytoscape)
│   └── src-tauri/src/
│       ├── lib.rs              # ✅ 启动 WasmRuntime + SqliteStorage → .manage(AppState)
│       └── commands/           # ✅ wasm.rs(list_connectors / sync_all_now)+ topology.rs(get_topology / get_graph)+ system.rs
├── modules/            # 独立 WASM workspace(target 隔离,wasm32-wasip2)
│   ├── manifest.toml           # 引擎启动读的模块清单
│   ├── sdk/                    # guest 端 WIT bindings
│   └── connectors/
│       ├── hello-world/        # ✅ 第一条 connector(WIT 端到端)
│       └── k8s-mini/           # ✅ 第二条(多 connector 编排验证)
├── specs/wit/          # ✅ 中立契约:host / connector / rule / handler 4 个 world
├── reference/          # ★ 旧 Python,read-only oracle(DO NOT DEPLOY)
└── doc/                # 18 份文档(00-17 + blog/1 篇)
```

### Phase 1 进展

| 增量 | 内容 | 状态 |
|---|---|---|
| A-B | WIT + toolchain:host-capabilities world + cargo aliases + cargo-component metadata | ✅ |
| 第一刀 | host wasmtime 真加载 hello_world.wasm 端到端跑通 | ✅ |
| C+D | WasmRuntime 多 connector 编排 + canonical Fact + Arrow RecordBatch + engine-cli tick | ✅ |
| E | k8s-mini 第二条 WASM connector + multi-connector 编排验证 | ✅ |
| F | Tauri ↔ engine-wasm 桥接(`list_connectors` + `sync_all_now` invoke) | ✅ |
| **G** | **http-client capability host 实装**(reqwest GET + capability allow-list) | ✅ |
| 收官 | 最小拓扑视图 + GUI verifier + Blog Part 1 + Option A polish | ✅ |

### Phase 2 进展

| 增量 | 内容 | 状态 |
|---|---|---|
| 2.1 | engine-storage SqliteStorage(raw Fact 落库 + `latest_topology_facts` 去重)| ✅ |
| 2.2 | Tauri `AppState { runtime, storage }`,`sync_all_now` sync 后 upsert 到 SQLite | ✅ |
| 2.3 | `get_topology` command + 前端启动从 SQLite 恢复拓扑(重启不 sync 也能渲染)| ✅ |
| 2.4 | GraphResponse DTO(engine-core `facts_to_graph` + Tauri `get_graph` + 前端改吃 `{nodes,edges,summary}`,对齐 reference `GraphResponse`)| ✅ |
| 2.5 | engine-identity ChangeSet resolver v0 + materialized topology_nodes/edges | ⏳ |
| 2.6 | 真实 K8s / Prometheus connector WASM 化 | ⏳ |

### 关键 crate 入口

- **engine-core**(`engine/crates/engine-core/src/`):`Fact`(WIT `connector.fact` 的 host 规范型,7 字段)+ `fact_schema()`(Arrow Schema)+ `FactBatch`(→ `RecordBatch` 零拷贝转储)。所有下游(storage / query / Arrow)只认它。`graph.rs` — `GraphResponse { nodes, edges, summary }`(对齐 reference `app/models/graph.py`)+ `facts_to_graph(&[Fact])`:topology-node 去重(newest)、`parent_resource_id` 派生 `CONTAINS` 边、悬空过滤、risk/health summary 统计。**领域逻辑在此,Tauri command 只薄包装**
- **engine-storage**(`engine/crates/engine-storage/src/`):`Storage` trait + `sqlite::SqliteStorage`(feature `sqlite`)。`connect` / `connect_in_memory` / `migrate` / `upsert_facts`(按 `Fact.id` 幂等)/ `latest_topology_facts`(按 `resource_id` 取最新 `topology-node`)。`StorageError` 统一错误。Parquet/Neo4j 仍待后续
- **engine-wasm**(`engine/crates/engine-wasm/src/`):
  - `runtime.rs` — `WasmConnector`(单 connector,持 wasmtime Store)+ host trait impls(`LoggingHost` / `ClockHost` / `HttpClientHost` for `State`)+ `load(path, capabilities)` / `load_with_http(client)` / `sync` / `health_check`
  - `http_host.rs` — `http-client` capability 纯函数实装(`http_get` + `HostHttpResponse`/`HostHttpError`,刻意与 WIT binding 解耦,可单测)
  - `multi.rs` — `WasmRuntime`(N 个 `ConnectorEntry`)+ `from_manifest` / `sync_all` / `tick_loop` + `SyncSummary`。**保持 storage-agnostic**,持久化在 orchestration 层(Tauri/CLI)做
  - `lib.rs` — `ModuleManifest` / `ManifestFile`(manifest.toml schema)+ `WasiVersion`(p2/p3 enum)
- **engine-cli**(`engine/crates/engine-cli/src/main.rs`):headless binary。`tick` 单次;`tick --loop --interval=30` 持续。`MODULES_ROOT` env 覆盖 manifest 根
- **desktop/src-tauri**:`lib.rs::run()` 启动 `WasmRuntime` + 在 `setup` 里初始化 `SqliteStorage`(路径取 `SRE_GRAPH_DB_PATH` 或 app data dir,migrate)→ `.manage(AppState { runtime, storage })`。command:`list_connectors` / `sync_all_now`(sync 后 upsert 到 SQLite)/ `get_topology`(读 latest topology facts,raw `FactDto[]`,留诊断用)/ `get_graph`(读 latest topology facts → `facts_to_graph` → `GraphResponse`,前端拓扑渲染走这条)
- **desktop/src/views/TopologyView.tsx**:Phase 2.4 视图,吃 `GraphResponse`。`graphToElements(graph)` 把 `{nodes,edges}` 纯映射成 Cytoscape elements(去重/连边/悬空过滤已在 Rust `facts_to_graph` 完成,前端不再解 JSON);有 Vitest 覆盖。`App.tsx` 启动 + sync 后均调 `get_graph` 拉成图的 `GraphResponse` 渲染

### 常用命令

```bash
# Engine
cargo build --workspace                      # 一次出 engine binaries + Tauri binary
cargo test --workspace                       # 全 Rust 单测
cargo clippy --workspace --all-targets -- -D warnings
engine-cli tick                              # 加载 manifest + 跑一次 sync_all
engine-cli tick --loop --interval=30         # 持续 sync(Ctrl-C 退)

# WASM modules(独立 workspace,target 隔离)
cd modules && cargo wasi-build               # 出 wasm32-wasip2 产物
MODULES_ROOT=/abs/path engine-cli tick       # 用指定 manifest 根跑

# Desktop(Tauri)
cd desktop && npm run tauri dev              # webview + Rust backend HMR
cd desktop && npm run tauri build            # 出 .app/.AppImage/.msi
cd desktop && npm test                       # 前端 vitest
SRE_GRAPH_DB_PATH=/tmp/x.sqlite npm run tauri dev   # 指定 SQLite 路径(默认 app data dir)
```

> 注:`make` 顶层入口(doc/16 §10 设计)尚未落 Makefile,当前用裸 cargo/npm。

### 三层数据契约(写新代码前对照 doc/15)

| 层 | 协议 | 边界 | 现状 |
|---|---|---|---|
| A | **WIT**(Component Model) | WASM ↔ host | ✅ `specs/wit/` 4 world;host 用 `wasmtime::component::bindgen!`,guest 用 `wit_bindgen::generate!` |
| B | **Tauri commands**(JSON IPC) | webview ↔ Rust | ✅ `commands/wasm.rs` + `commands/topology.rs`(get_topology / get_graph→`GraphResponse`);Phase 2+ 继续拆 `commands/{recovery,...}.rs` |
| C | **Arrow RecordBatch** + Parquet + SQLite | engine 内部 | 🔨 engine-core Arrow Schema ✅;engine-storage SQLite raw Fact backend ✅(Phase 2.1);Parquet 归档 + materialized topology 表待 Phase 2.5 |

**反模式(不做)**:Tauri 里又起 HTTP server(用 invoke 直接 IPC);desktop/ 写业务逻辑(逻辑在 engine-core,Tauri command 是薄包装);WASM 模块直接 syscall(host 注入 capability,deny by default)。

### Capability 设计(http-client 实例,Phase 1 G)

- **deny by default**:`manifest.toml` 每个模块 `capabilities = [...]` 显式申明;host 调用时 allow-list 查表,缺则返 `HostHttpError::Unauthorized`
- **call-time 拒绝(非 link-time)**:共享 Linker,`http_get` 每次查 `HashSet<String>`,简单 + 后续加 URL allow-list 平滑
- **host 类型与 WIT binding 解耦**:`HostHttpResponse` / `HostHttpError` 在 `http_host.rs` 定义,可单测;`runtime.rs::HttpClientHost::get` 是薄适配做类型平移
- **状态码映射**:401/403 → `Unauthorized`;404 → `NotFound`;timeout → `Timeout`;其它(含 5xx)透状态码 + body 给 guest 自决

### 待办

- [x] Phase 1 收官:最小拓扑视图(打开 app 看到 mock 拓扑图)+ Blog Part 1 + GUI verifier + Option A 首屏 polish
- [x] Phase 2 第一刀:SQLite-backed mock topology persistence(`sync_all_now` 写入 SQLite,重启后 `get_topology` 从库恢复;GUI X11 验证通过)
- [x] Phase 2.4:GraphResponse DTO(engine-core `facts_to_graph` + Tauri `get_graph` + 前端改吃 `{nodes,edges,summary}`;boot 从 SQLite 恢复并成图渲染,GUI X11 验证通过)
- [ ] Phase 2.5:Identity Resolver(ChangeSet + SQLite UPSERT + materialized topology_nodes/edges)
- [ ] Phase 2.6:真实 K8s / Prometheus connector WASM 化(对照 `reference/backend/app/datasource/connectors/`)+ Tauri 视图迁
- [ ] Phase 3:engine-recovery(PRD-001)/ engine-changes(PRD-002)复刻 —— **PRD-001 审批流桌面语义需在此 Phase 明确决策**(doc/14 §9 风险)
- [ ] Phase 4:engine-reports(PRD-003)复刻

---

## Reference(Python,read-only oracle —— DO NOT MODIFY)

> 重写期作行为规约参考。详细实现看 `reference/` 源码 + `doc/01-13`。**不要改 reference/**;Rust 复刻在 `engine/crates/engine-{recovery,changes,reports}/`(目前是骨架)。

### Python 栈快照

- **Backend**:Python 3.12 + FastAPI + uv,`reference/backend/`(原 `backend/`)
- **Frontend**:React 18 + TypeScript + Cytoscape.js + AntD 5 + Vite,`reference/frontend/`(逻辑复用约 90% 迁 `desktop/`)
- **Graph DB**:Neo4j 5(Docker,bolt://localhost:7687)
- **Deployment**:Docker Compose
- **测试**:472 backend + 71 frontend

### 4 层图模型 + DSS

```
L1 Resource Type Graph    → 14 type nodes + 35 relationships(静态 CSV)
L2 Resource Instance      → application/component/Deployment/Pod/middleware instance
L3 Dynamic Observability  → MetricQuery + MetricSnapshot + AlertEvent + ChangeEvent
L4 Inspection Results     → InspectionRun/Rule/Finding
+ DSS(in-memory)         → 解耦 fault injection 与 Neo4j 写
```

### 4 个 PRD 一句话总结(深入查 doc/01-13)

| PRD | 功能 | 关键文件 |
|---|---|---|
| **PRD-001 Recovery Action Engine** | 8 action(scale/restart_pod/refresh_secret/rollback_deployment/drain_node/kill_query/clear_cache/restart_service)+ dry-run + 审批 + 回滚 + 真实 K8s/MySQL/Redis handler(`RECOVERY_HANDLER_MODE=real`)+ 跨集群编排 + 自动验证 + 动作链 | `reference/backend/app/recovery/` |
| **PRD-002 Change Event** | 4 类变更(configmap/secret/deployment/image)+ correlated query + propagation BFS + Neo4j dual-write + K8s watcher + webhook + YAML diff + 频率告警 + ChangeEvent↔AlertEvent 关联 | `reference/backend/app/changes/` |
| **PRD-003 Self-Inspection Report** | 3 模板(application_health / cluster_overview / incident_report)+ Jinja2 Markdown + APScheduler 订阅 + SMTP + Neo4j 持久化 | `reference/backend/app/reports/` |
| **PRD-004 OTel Demo Connectors** | 5 connector(k8s / prometheus / jaeger / flagd / k8s_events)+ AlertEvent + scenario 接入 + Connector UI | `reference/backend/app/datasource/connectors/` |

### Inspection 视图 6 个

`/topology` `/access-link` `/node-impact` `/config-impact` `/image-risk` `/alert-aggregation` —— 各一个 router + Cypher query + Cytoscape 视图,详见 `doc/05-six-views-design.md`。

### Fault Types(7)

cpu_spike / memory_leak / pod_crashloop / node_disk_pressure / service_no_endpoints / mysql_slow_query / redis_unavailable —— 详见 `doc/08-fault-types-and-timeline.md`。

### 节点视觉规则(Cytoscape)

**Shape = 资源类型,fill = health(green/yellow/red),border = risk(thin green / medium yellow / thick red)**。无 per-type fill 配色。形状参见 `reference/frontend/src/utils/graphStyles.ts`。

### Python 旧栈命令(read-only,跑 reference)

```bash
# 一次性 setup
make setup
# Neo4j + import baseline
make infra
# API hot-reload(8000)+ Frontend HMR(3000)
make dev-api
make dev-frontend
# 测试
make test                                                       # 472 + 71
cd reference/backend && uv run python -m pytest -p no:asyncio   # backend pytest 必须 -p no:asyncio
```

### 几个关键设计决策(老栈,迁 Rust 时对照)

1. **Neo4j 5 路径深度需字符串插值** —— `*1..5` 而非 `*1..$depth`(FastAPI ge/le 校验过的)
2. **节点用 property-based label** —— `ResourceInstance` + `label` property,不是 multi-label
3. **DSS 是 single source of truth** —— fault 写 DSS,DSS 同步 Neo4j;生产走 DSS 端点不走 simulation
4. **HTTP 语义**:low_risk → 200(sync done);medium/high → 202(awaiting_approval)。前端按 status 字段分支,不按 HTTP code
5. **Rollback 跳过二次审批** —— 原始动作已审,反向是"撤销"不是新风险
6. **K8s client 真实模式**:成功后才更新 DSS 孪生,失败 DSS 不动(避免内存图与集群失真)
7. **kubernetes_asyncio config 是全局状态** —— 多集群只能 switch-and-reload(Phase 3 上 per-ApiClient `Configuration` 真并发)

---

## 重写期工作约定

- **新代码进 `engine/` / `desktop/` / `modules/` / `specs/`**;**绝不在 `reference/` 加 feature**
- **行为参考**:Rust 复刻每个 PRD 时,读 reference 对应模块的源码 + 测试(测试是规约),不是 PRD 文档(可能落后于实现)
- **Contract test**:Phase 3+ 复刻时,挑 reference 的代表性测试用例,Rust 端跑等价测试,行为偏差需在 commit msg 中明示
- **doc/14-17** 是技术战略 + 数据契约 + repo 布局 + Tauri 架构 4 份核心文档,写代码前先读
