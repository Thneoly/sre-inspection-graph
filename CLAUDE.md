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
│       ├── engine-identity/    # ✅ Identity Resolver v0(resolve/diff/topology_to_graph + Phase 2.7 health_merge)
│       ├── engine-recovery/    # ✅ Phase 3.1-3.3:action_defs + dry_run + execution/approval/rollback + verifiers/auto-rollback + chains
│       ├── engine-changes/     # ✅ Phase 3.4-3.5:ChangeEvent + record_change + propagation + frequency + alert 关联 + yaml_diff + correlated_changes + suggest_for_change 桥
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
│       ├── k8s-mini/           # ✅ 第二条(多 connector 编排验证)
│       ├── prometheus/         # ✅ 第三条(首个消费 http-client capability,GET /api/v1/query)
│       └── k8s/                # ✅ 第四条(真实 K8s API via kubectl proxy,真集群验证)
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
| 2.2 | Tauri `AppState { runtime, storage, proxy }`,`sync_all_now` sync 后 upsert 到 SQLite | ✅ |
| 2.3 | `get_topology` command + 前端启动从 SQLite 恢复拓扑(重启不 sync 也能渲染)| ✅ |
| 2.4 | GraphResponse DTO(engine-core `facts_to_graph` + Tauri `get_graph` + 前端改吃 `{nodes,edges,summary}`,对齐 reference `GraphResponse`)| ✅ |
| 2.5 | engine-identity ChangeSet resolver v0(`resolve`/`diff`/`topology_to_graph`)+ materialized `topology_nodes`/`topology_edges` 表 | ✅ |
| 2.6 | Prometheus connector WASM 化(首个消费 `http-client` capability:GET `/api/v1/query` → 解析 Prom JSON → metric Fact)| ✅ |
| 2.6b | 真实 K8s API connector WASM 化(`modules/connectors/k8s` via 本地 kubectl proxy;Deployment/Pod/Service/Node → topology Fact + owner 链 + health;真集群 otel-demo 验证)| ✅ |

| 2.7 | metric->topology health 合并(engine-identity `health_merge`,doc/11 §4.3 field-ownership v0:最严重胜出)+ desktop 托管 kubectl proxy(`commands/proxy.rs`)+ per-connector manifest config(`select_config` + `enabled`)+ Tauri 真集群拓扑(shape=type/fill=health/border=risk 真色);真集群 headless 验证:engine-cli tick k8s 经 manifest api_base 拉 71 fact | ✅ |

### Phase 3 进展

> PRD-001(recovery)/ PRD-002(changes)复刻。审批语义已定(doc/14 §9):**桌面单机确认门**(保留 risk->status,丢 reference 的 `approver_team`/24h TTL/多人 approve-reject)。

| 增量 | 内容 | 状态 |
|---|---|---|
| 3.1 | engine-recovery `action_defs`(8 action 元数据 + `propagation` 规则 + rule/change 推荐)+ `cascade::dry_run`(BFS blast radius,I/O-free 吃 `&Topology`)+ contract test(移植 reference `TestActionDefs`/`TestDryRun`,合成 9 节点 11 边拓扑) | ✅ |
| 3.2 | engine-recovery `execution` 管线(validate->dry_run->approval gate->mock handler)+ `approval`(单机确认门 confirm/cancel)+ rollback(skip re-approval)+ 8 mock handler + SQLite `recovery_executions` 表 | ✅ |
| 3.3 | engine-recovery `verifiers`(8,读 mutated twin attrs)+ auto-rollback(verify_failed -> 反向 exec,marker 防递归)+ `chains`(3 模板 × 3 on_failure:Stop/RollbackAll/Continue)+ **mutable twin**(handler `&mut ResolvedNode` 写回 attributes_json) | ✅ |
| 3.4 | engine-changes `ChangeEvent` 模型(18 字段)+ `record_change`(校验/ID/propagated_to/severity/commit_sha 回填)+ propagation 反向 BFS(`derive_propagation`/`find_propagation_path`/`find_descendants`,I/O-free 吃 `&Topology`)+ 内存 `ChangeRegistry` CRUD | ✅ |
| 3.5 | engine-changes frequency(过频 low->medium + `[过频变更]` tag)+ alert 关联(`correlate_alerts`/`correlate_changes_for_alert`)+ yaml_diff(`compute_yaml_diff` + `summarize_diff`,剥 10 噪声字段)+ `correlated_changes`(direct/propagated + 时间窗)+ `get_recovery_suggestion` 桥(接 `engine_recovery::suggest_for_change`,direct/propagated/unresolved)+ `record_change` 接入频率检查 + `serialize` | ✅ |
| 3.6 | Tauri commands(recovery/change_events/alerts ~31 条)+ AntD 视图(全量移植 reference Recovery/Changes)+ SQLite 持久化(change_events/recovery_chains/alert_events 3 新表 + 启动 from_* 载入)+ 接 sync 管线(拓扑消费 only) | ✅ |
| 3.7 | k8s connector 边富化(`topology-edge` fact:SCHEDULED_ON/ROUTES_TO/USES + ConfigMap/Secret node;`facts_to_graph` 分流 node/edge;`latest_topology_facts` SQL 含 edge;真集群 47 edge:22 SCHEDULED_ON+22 ROUTES_TO+3 USES)| ✅ |

| 3.8 | k8s connector Application/Component/Middleware 层(从 deploy 派生:normalize_component_name + detect_middleware + is_infra;CONTAINS[app->comp 派生]/DEPLOYED_AS[comp/mw->deploy]/BELONGS_TO[deploy->comp,comp->app 反向] edge;真集群 1 app+16 comp+1 Redis+1 Kafka + 97 edge[32 BELONGS_TO+18 DEPLOYED_AS+22 SCHEDULED_ON+22 ROUTES_TO+3 USES])| ✅ |

### 关键 crate 入口

- **engine-core**(`engine/crates/engine-core/src/`):`Fact`(WIT `connector.fact` 的 host 规范型,7 字段)+ `fact_schema()`(Arrow Schema)+ `FactBatch`(→ `RecordBatch` 零拷贝转储)。所有下游(storage / query / Arrow)只认它。`graph.rs` — `GraphResponse { nodes, edges, summary }`(对齐 reference `app/models/graph.py`)+ `facts_to_graph(&[Fact])`:topology-node 去重(newest)、`parent_resource_id` 派生 `CONTAINS` 边、悬空过滤;**Phase 3.7** 加 `topology-edge` fact 分流(显式 USES/ROUTES_TO/SCHEDULED_ON 边,各自 dedup,悬空+自环过滤,与 CONTAINS 合并);+ `summarize(&[GraphNode], &[GraphEdge])`(risk/health 固定桶统计的**唯一入口**,`facts_to_graph` 与 engine-identity `topology_to_graph` 共用,不漂移)。**领域逻辑在此,Tauri command 只薄包装**
- **engine-identity**(`engine/crates/engine-identity/src/`):Identity Resolver **v0**(`resource_id` 直接当 canonical 身份键,不做 correlation-key 合并 / 仲裁 —— 见 doc/11 §4-5 完整版)。`topology.rs` — `Topology { nodes: ResolvedNode[], edges: ResolvedEdge[] }`(持久化形态;`attributes_json` 存 canonical 字符串)+ `resolve(&[Fact])`(复用 `engine_core::facts_to_graph` 派生,再平移)+ `topology_to_graph(&Topology)`(反建前端 `GraphResponse`,summary 复用 `engine_core::summarize`)。`changeset.rs` — `ChangeSet { nodes_upserted, nodes_removed, edges_upserted, edges_removed }` + `ChangeSummary`(计数)+ `diff(current, next)`(身份键 + 内容相等判增删改)。**I/O-free 纯领域逻辑,可单测**;持久化在 engine-storage。**Phase 2.7** `health_merge.rs` - `HealthThresholds`(内置 prometheus 3 metric 阈值)+ `derive_metric_health(&Fact)`(metric value -> health)+ `merge_metric_health(&Topology, &[metric Fact])`(doc/11 §4.3 field-ownership **v0:最严重胜出** critical>warning>normal,把 prometheus metric health 合进 topology 节点;orchestration 在 resolve 后、diff 前调)
- **engine-storage**(`engine/crates/engine-storage/src/`):`Storage` trait + `sqlite::SqliteStorage`(feature `sqlite`)。`connect` / `connect_in_memory` / `migrate` / `upsert_facts`(按 `Fact.id` 幂等)/ `latest_topology_facts`(按 `resource_id` 取最新 `topology-node`/`topology-edge`,**Phase 3.7** SQL `kind IN ('topology-node','topology-edge')`);**Phase 2.5** 加 `topology_nodes`/`topology_edges` materialized 表 + `materialized_topology()`(读当前拓扑)+ `apply_change_set(&ChangeSet)`(单 tx upsert + delete stale)。**Phase 3.2** 加 `recovery_executions` 表 + `upsert/get/list_recovery_execution`。**Phase 3.6** 加 `change_events`/`recovery_chains`/`alert_events` 3 表 + `upsert/get/list_*`(enum 列存 snake_case JSON 文本,Vec/Value 存 JSON 文本;round-trip 测试)。`StorageError` 统一错误;`examples/dump_topology.rs` 是 GUI-less 验证 `get_graph` 读路径的小工具。Parquet/Neo4j 仍待后续
- **engine-wasm**(`engine/crates/engine-wasm/src/`):
  - `runtime.rs` — `WasmConnector`(单 connector,持 wasmtime Store)+ host trait impls(`LoggingHost` / `ClockHost` / `HttpClientHost` for `State`)+ `load(path, capabilities)` / `load_with_http(client)` / `sync` / `health_check`
  - `http_host.rs` — `http-client` capability 纯函数实装(`http_get` + `HostHttpResponse`/`HostHttpError`,刻意与 WIT binding 解耦,可单测)
  - `multi.rs` — `WasmRuntime`(N 个 `ConnectorEntry`)+ `from_manifest`(跳过 `enabled=false`)/ `sync_all` / `tick_loop` + `SyncSummary` + `select_config`(Phase 2.7:per-connector `config` 优先,无则回退全局 broadcast)。**保持 storage-agnostic**,持久化在 orchestration 层(Tauri/CLI)做
  - `lib.rs` — `ModuleManifest` / `ManifestFile`(manifest.toml schema;Phase 2.7 加 `enabled: bool` + `config: Option<serde_json::Value>`)+ `WasiVersion`(p2/p3 enum)
- **engine-cli**(`engine/crates/engine-cli/src/main.rs`):headless binary。`tick` 单次;`tick --loop --interval=30` 持续。`MODULES_ROOT` env 覆盖 manifest 根
- **engine-recovery**(`engine/crates/engine-recovery/src/`):PRD-001 复刻。**Phase 3.1** `action_defs.rs` - 8 个命名 `static ActionDef`(restart_pod/scale_deployment/rollback_deployment/refresh_secret/drain_node/kill_query/restart_service/clear_cache;元数据 + `propagation` 规则 + rule/change 推荐;`kill_query` 例外 medium 不审批)+ `get_action`/`list_actions_filtered`/`suggest_for_rule`/`suggest_for_change`。`cascade.rs` - `dry_run(action_id, target, input_params, &Topology) -> DryRunResult`(I/O-free BFS blast radius)。**Phase 3.2** `models.rs` - `RecoveryExecution`(全 24 字段)+ `RecoveryStatus`/`VerifyStatus`/`ExecutionContext`/`ExecutionError`(单机确认门:无 ApprovalRequest 实体/TTL/approver_team)。`handlers.rs` - 8 mock handler(`fn(&mut ResolvedNode,&Value,&ExecutionContext)->Value`,**3.3 起 mutate twin `attributes_json`** 模拟动作生效:desired/available_replicas、restart_count+health_status、current_revision、secret_version、cordoned、endpoints_refresh_count)+ `HANDLERS`/`is_executable`。`execution.rs` - `ExecutionRegistry`(in-memory)+ `execute`(low 同步 / medium·high awaiting_approval)+ `confirm_execution`/`cancel_execution`(单机确认门)+ `rollback`(skip re-approval,反向 execution)。engine-storage 加 `recovery_executions` 表 + `upsert/get/list_recovery_execution`。**逐字对齐 reference,contract test 移植 `TestActionDefs`/`TestDryRun` + risk->status + rollback 矩阵**。**Phase 3.3** `verifiers.rs` - 8 verifier(`fn(&ResolvedNode,&Value,&Value,&ExecutionContext)->VerifierVerdict`,读 handler 写回的 mutated attrs 验 predicate;`kill_query`/`clear_cache` 无可观测副作用 -> `not_supported` passed=true)+ `VERIFIERS`/`get_verifier`/`run_verifier`(verifier set 可注入,测试 fake failing verifier 触发 auto-rollback)。`execution.rs` **3.3 重写**:全管线 `&mut Topology`;`run_handler`(handler mutate twin + 若 succeeded+verify 跑 verifier)+ `verify_and_maybe_rollback`(verify_failed + auto_rollback + 有 rollback_action_id -> `do_rollback(marker=true)`,无则 warn)+ `do_rollback`(反向 exec `verify=!marker` 防递归)+ `reverify`(手动重验)。**mutable twin 架构**:handler mutate `&mut Topology` twin,verifier 读 mutated attrs,rollback 读 post-action 状态做正确反转(orchestration 层 3.6 应传 clone 避免污染源拓扑)。`chains.rs` - `ChainRegistry`/`ChainStep`/`ChainTemplate` + 3 模板 + `execute_chain`(链级审批:任一步非 low/requires_approval -> awaiting_approval,全 low 直接跑)/`confirm_chain`/`cancel_chain`(->Failed)/`abort_chain`(->Aborted)+ `run_chain_steps`(Stop/RollbackAll/Continue;RollbackAll 反向 do_rollback 已成 prior step)+ `run_single_step`(auto_rollback=false,verify=step.verify_required,失败交 chain on_failure)。`models.rs` 加 `RecoveryChain`/`ChainStatus`/`OnFailureStrategy`。**clippy + 61 测试绿**
- **engine-changes**(`engine/crates/engine-changes/src/`):PRD-002 复刻。**Phase 3.4** `models.rs` - `ChangeEvent`(18 字段,对齐 reference dataclass)+ `ChangeType`(4:configmap_updated/secret_rotated/deployment_rolled/image_pushed)/`Source`(6)/`Severity`(3)强类型枚举(snake_case 序列化,reference 用 plain str + 集合校验)+ `ChangeRequest`(14 字段入参结构体,避免 too_many_arguments;`Default` 复刻 source=manual/diff_summary={})+ `ChangeFilter`(type/target/source/since/until 闭区间)+ `ChangeError`(message+code,对齐 `ChangeEventError`)。`propagation.rs` - `PROPAGATION_EDGES`(8 白名单:USES/CONTAINS/DEPLOYED_AS/BELONGS_TO/RUNS/SCHEDULED_ON/EXPOSES/ROUTES_TO,**不含 USES_IMAGE**)+ `derive_propagation(target,&Topology,max_depth=4,edge_types)`(反向 BFS,incoming[edge.target]->edge.source,排除自身,target 不在拓扑 -> [])+ `find_propagation_path(source,affected,&Topology,max_depth=4)`(反向 BFS 最短路径 + parents 回溯,source==affected/不可达 -> [])+ `find_descendants(start,&Topology,max_depth=6)`(镜像 forward BFS)。全 I/O-free 吃 `&Topology`(reference 读全局 DSS `store`)。`event_service.rs` - `ChangeRegistry`(in-memory,add/get/list(filter)/clear,插入序 + ISO8601 字典序闭区间过滤)+ `estimate_severity(count)`(>=10 high/>=5 medium/else low)+ `record_change(&mut ChangeRegistry,&Topology,&ChangeRequest)`(校验 type/source -> 400,gen `ce-<12 hex>`,查 target_resource_type 或 "",propagated_to 反向 BFS,severity,commit_sha 回填 related_commit)。**偏差**:丢 Neo4j dual-write + alert 关联(3.5);不调 `_apply_frequency_check`(3.5);v0 拓扑边由 facts_to_graph 派生(**3.7 k8s 边富化已接入**:USES/ROUTES_TO/SCHEDULED_ON,生产传播沿白名单边反向)。contract test 移植 reference `TestChangeEventModel`/`TestStore`/`TestPropagation`/`TestRecordChange`(10 节点 10 边 fixture),**clippy + 29 测试绿**。**Phase 3.5** `iso.rs` - `now_iso`/`parse_iso_utc`/`shift_iso` 共享时间工具。`yaml_diff.rs` - `NOISE_KEYS`(10:managedFields/resourceVersion/uid/creationTimestamp/generation/selfLink/etag/last-applied-configuration/annotations/managedVersion)+ `strip_noise`(递归)+ `select_keys` + `compute_yaml_diff(old,new,keys,name)`(自写确定性 block-style 发射器排序键,**不引 serde_yaml**;Value 结构相等短路 -> "";否则 `similar` unified_diff)+ `summarize_diff`->{added,removed,changed_keys}。`alerts.rs` - `AlertEvent`(13 字段)+ `AlertSeverity`(warning/critical)/`AlertStatus`(firing/resolved)+ `AlertRegistry`(in-memory,add/get/list(fired_at 闭区间))。`frequency.rs` - `check_target_frequency`/`detect_frequent_changes`(默认 3600s/5,严格 `>`)+ `apply_frequency_check`(`record_change` add 后调,只升 low->medium + `[过频变更:N次/Ws]` tag)。`alert_correlation.rs` - `correlate_alerts`(affected={target}∪propagated_to,窗 [changed±window],resource_ref 命中)+ `correlate_changes_for_alert`(反向);**丢 Neo4j `CORRELATED_WITH` 边 + `persist_correlation`**,`neo4j_available` 恒 false。`event_service.rs` **3.5 扩展**:`record_change` 接入 `apply_frequency_check`(add 后调,计数含当前,回写 severity/description via `get_mut`)+ `serialize`(全字段 + propagated_count)+ `correlated_changes(target,window=300,since,until,include_propagated,&ChangeRegistry,&Topology)`(direct/propagated + 三种时间窗模式 + propagation_distance=max(path-1,1) + changed_at 倒序)+ `get_recovery_suggestion(event_id,&ChangeRegistry,&Topology)`(404;桥 `engine_recovery::suggest_for_change`,嵌套 ActionSuggestion -> 扁平 RecoverySuggestion,target_match direct/propagated/unresolved)。contract test 移植 reference `TestYamlDiff`/`TestFrequencyAlert`/`TestCorrelatedQuery`/`TestRecoverySuggestion`/`TestAlertCorrelation`(8 节点 7 边 phase2 fixture + 10 节点 sprint1 fixture),**clippy + 66 测试绿**
- **desktop/src-tauri**:`lib.rs::run()` 启动 `WasmRuntime` + 在 `setup` 里初始化 `SqliteStorage`(路径取 `SRE_GRAPH_DB_PATH` 或 app data dir,migrate)→ `.manage(AppState { runtime, storage })`。command:`list_connectors` / `sync_all_now`(sync → upsert raw facts → **resolve+merge_metric_health+diff+apply_change_set** 维护 materialized 拓扑,返回 `changes` 增量计数)/ `get_topology`(读 latest topology facts,raw `FactDto[]`,留诊断用)/ `get_graph`(**Phase 2.5 起**读 materialized 拓扑 → `engine_identity::topology_to_graph` → `GraphResponse`,前端拓扑渲染走这条)。Phase 2.7 加 `commands/proxy.rs`(`start_kubectl_proxy`/`stop_kubectl_proxy`/`proxy_status` 托管 `kubectl proxy --port=8001`,TCP 就绪探测;`AppState.proxy: Mutex<Option<Child>>` 持子进程,`RunEvent::Exit` kill 防孤儿)。**Phase 3.6** `AppState` 加 4 `Mutex` registry(`recovery_executions`/`recovery_chains`/`change_events`/`alerts`),`setup` 启动从 storage `from_executions`/`from_chains`/`from_events`/`from_alerts` 载入(重启恢复)。新 command 模块:`commands/recovery.rs`(18 条:actions/dry_run/execute/executions CRUD/confirm/cancel/rollback/reverify + chains templates/execute/confirm/cancel/abort/list + suggestions)+ `commands/change_events.rs`(8 条:record/list/get/correlated/frequent/impact/recovery_suggestion/alerts)+ `commands/alerts.rs`(5 条:record/list/get/resolve/correlate_changes_for_alert)。**mutable twin**:mutation 命令读 `materialized_topology()` 的 owned clone 作 `&mut Topology`(handler mock 模拟,**不写回** materialized 表;真实态只由 sync 更新),engine 调用同步在锁内,storage upsert 异步在锁外(`Mutex`Guard 不跨 await)。**DTO 偏差**:直接返 engine `Serialize` 类型(RecoveryExecution/RecoveryChain/ChangeEvent/AlertEvent/DryRunResult/CorrelatedResult/RecoverySuggestionResult),非手写 DTO(区别于 wasm.rs Fact 模式);`ChainTemplate`/`ChainStep` 未 Serialize -> `ChainTemplateDto`。错误 `ExecutionError`/`ChangeError` 的 `Display`=`"[code] msg"` -> `.map_err(|e| e.to_string())`
- **desktop/src/views/TopologyView.tsx**:Phase 2.4 视图,吃 `GraphResponse`。`graphToElements(graph)` 把 `{nodes,edges}` 纯映射成 Cytoscape elements(去重/连边/悬空过滤已在 Rust 完成,前端不再解 JSON);有 Vitest 覆盖。Phase 2.7 起 fill=health / border=risk 真色(shape=type 不变,对齐 reference graphStyles),经 `healthFill`/`riskBorder` helper + `data(fill)`/`data(borderColor)` mapper 上色。`App.tsx` 启动 + sync 后均调 `get_graph` 拉成图渲染;sync 后 header 显示 `changes` 增量(`Δ +Nn/Me −Kn/Le`)。**Phase 3.6** 前端迁 AntD(`antd`6 + `react-router-dom`6 + `@tanstack/react-query`5):`main.tsx` 包 `QueryClientProvider`+`HashRouter`+`ConfigProvider`;`App.tsx` 嵌套布局路由(父 `MainLayout` Sider 菜单 + Outlet,子路由 Topology/Recovery/Changes);`api/client.ts` invoke-based 封装(全 ~31 命令 + TS 类型,axios->invoke)。视图:`pages/TopologyPage`(原 App 拓扑功能 + react-query + 节点点击 -> `NodeDetailPanel` Drawer 集成 `RecoveryActionsSection`+`ChangeTimelineSection`)+ `pages/RecoveryPage`(`ExecutionsView` Table+Drawer+折叠审批 confirm/cancel/rollback/reverify + `RecoveryChainsView` + `DryRunModal`)+ `pages/ChangesPage`(`ChangeTimelineView` Table+Drawer impact/alerts/suggestion + 记录表单 + `AlertsView`)。TopologyView 加 `onSelectNode` prop(cy tap -> ref 防 stale)。vitest 14 测试(7 既有 graphToElements + 7 api/client invoke 包装)

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
| C | **Arrow RecordBatch** + Parquet + SQLite | engine 内部 | 🔨 engine-core Arrow Schema ✅;engine-storage SQLite raw Fact backend ✅(Phase 2.1)+ materialized `topology_nodes`/`topology_edges` 表 ✅(Phase 2.5);Parquet 归档待后续 |

**反模式(不做)**:Tauri 里又起 HTTP server(用 invoke 直接 IPC);desktop/ 写业务逻辑(逻辑在 engine-core,Tauri command 是薄包装);WASM 模块直接 syscall(host 注入 capability,deny by default)。

### Capability 设计(http-client 实例,Phase 1 G)

- **deny by default**:`manifest.toml` 每个模块 `capabilities = [...]` 显式申明;host 调用时 allow-list 查表,缺则返 `HostHttpError::Unauthorized`
- **call-time 拒绝(非 link-time)**:共享 Linker,`http_get` 每次查 `HashSet<String>`,简单 + 后续加 URL allow-list 平滑
- **host 类型与 WIT binding 解耦**:`HostHttpResponse` / `HostHttpError` 在 `http_host.rs` 定义,可单测;`runtime.rs::HttpClientHost::get` 是薄适配做类型平移
- **状态码映射**:401/403 → `Unauthorized`;404 → `NotFound`;timeout → `Timeout`;其它(含 5xx)透状态码 + body 给 guest 自决
- **首个消费方(Phase 2.6)**:`modules/connectors/prometheus` —— `bindings::sre::inspection::http_client::get(url, [])` 发 GET,manifest `capabilities=["logging","clock","http-client"]`。缺 `http-client` 时 host 返 `Unauthorized`,guest 整轮 0 fact + error note(`tests/prometheus_http_e2e.rs` 两个 case:mock Prom server 放行 + deny-by-default 拒绝,均覆盖)

### 待办

- [x] Phase 1 收官:最小拓扑视图(打开 app 看到 mock 拓扑图)+ Blog Part 1 + GUI verifier + Option A 首屏 polish
- [x] Phase 2 第一刀:SQLite-backed mock topology persistence(`sync_all_now` 写入 SQLite,重启后 `get_topology` 从库恢复;GUI X11 验证通过)
- [x] Phase 2.4:GraphResponse DTO(engine-core `facts_to_graph` + Tauri `get_graph` + 前端改吃 `{nodes,edges,summary}`;boot 从 SQLite 恢复并成图渲染,GUI X11 验证通过)
- [x] Phase 2.5:Identity Resolver v0(engine-identity `resolve`/`diff`/`topology_to_graph` + engine-storage materialized `topology_nodes`/`topology_edges` + Tauri sync 维护 + `get_graph` 改读 materialized;headless `dump_topology` 验证 + 全栈 cargo/vitest 绿)
- [x] Phase 2.6:Prometheus connector WASM 化(`modules/connectors/prometheus` 首个消费 http-client capability;GET `/api/v1/query` → Prom JSON → metric Fact;mock-server e2e + deny-by-default 验证)
- [x] Phase 2.6b:真实 K8s API connector WASM 化(`modules/connectors/k8s` 经本地 `kubectl proxy` 明文 HTTP 拉 API;纯 mapper 把 Deployment/ReplicaSet/Pod/Service/Node 映射成 topology Fact —— owner 链 Pod→RS→Deploy、health 由 phase/ready 推导、parent 层级;真集群 otel-demo 验证:71 fact / GraphResponse nodes=71 edges=70 health{critical:1,warning:4})。**架构**:WASM 只用 http-client,TLS+认证留 kubectl proxy,不碰凭据、不加 capability
- [x] Phase 2.7(可选):metric→topology health 合并(需 Identity Resolver field-ownership,见 doc/11 §4.3)+ desktop 托管 kubectl proxy 生命周期 + Tauri 视图迁真集群拓扑。✅ 完成:health_merge/storage/proxy/select_config/vitest 单测全绿 + 真集群 headless(engine-cli tick k8s 经 manifest api_base 拉 71 fact;prometheus OOM 0 fact 符合预期,merge no-op)
- [x] Phase 3.1:engine-recovery `action_defs`(8 action 元数据 + propagation + rule/change 推荐)+ `cascade::dry_run`(BFS blast radius,I/O-free 吃 `&Topology`)+ contract test(移植 reference `TestActionDefs`/`TestDryRun`,合成 9 节点 11 边拓扑);逐字对齐 reference,clippy + 20 测试绿。**审批语义已定**(doc/14 §9):桌面单机确认门,3.2 落地
- [x] Phase 3.2:engine-recovery `execution` 管线(execute:low 同步 / medium·high awaiting_approval)+ `approval`(单机确认门 confirm/cancel)+ rollback(skip re-approval)+ 8 mock handler + engine-storage `recovery_executions` 表;contract test 移植 risk->status + rollback 矩阵,clippy + 39+11 测试绿
- [x] Phase 3.3:engine-recovery `verifiers`(8,读 mutated twin attrs)+ auto-rollback(verify_failed -> 反向 exec,marker 防递归;无 rollback_action_id 则 warn)+ `chains`(3 模板 × 3 on_failure:Stop/RollbackAll/Continue)+ **mutable twin 架构**(handler `&mut ResolvedNode` 写回 `attributes_json`,verifier 读 mutated attrs,rollback 读 post-action 状态做正确反转);contract test 移植 reference verify/auto-rollback + chain on_failure 矩阵,clippy + 61 测试绿
- [x] Phase 3.4:engine-changes `ChangeEvent` 模型(18 字段)+ `record_change`(校验 type/source 400 + `ce-<12hex>` ID + target_resource_type 查拓扑 + propagated_to 反向 BFS + severity + commit_sha 回填 related_commit)+ propagation 反向 BFS(`derive_propagation`/`find_propagation_path`/`find_descendants`,I/O-free 吃 `&Topology`,8 种白名单边不含 USES_IMAGE)+ 内存 `ChangeRegistry` CRUD(add/get/list 闭区间过滤/clear);contract test 移植 reference `TestChangeEventModel`/`TestStore`/`TestPropagation`/`TestRecordChange`(10 节点 10 边 fixture),clippy + 29 测试绿。**偏差**:丢 Neo4j dual-write + alert 关联(3.5)、不调 frequency check(3.5)、v0 拓扑只 CONTAINS 边生产传播只沿 CONTAINS 反向
- [x] Phase 3.5:engine-changes frequency(过频 low->medium + `[过频变更:N次/Ws]` tag,`record_change` add 后调,计数含当前,回写 via `get_mut`)+ alert 关联(`correlate_alerts`/`correlate_changes_for_alert`,affected={target}∪propagated_to,窗 [changed±window])+ yaml_diff(`compute_yaml_diff` + `summarize_diff`,剥 10 噪声字段,自写确定性 block-style 发射器**不引 serde_yaml**)+ `correlated_changes`(direct/propagated + 三种时间窗 + propagation_distance)+ `get_recovery_suggestion` 桥(接 `engine_recovery::suggest_for_change`,嵌套 ActionSuggestion -> 扁平 RecoverySuggestion,direct/propagated/unresolved)+ `serialize`;contract test 移植 reference `TestYamlDiff`/`TestFrequencyAlert`/`TestCorrelatedQuery`/`TestRecoverySuggestion`/`TestAlertCorrelation`(8 节点 7 边 phase2 + 10 节点 sprint1 fixture),clippy + 66 测试绿。**偏差**:丢 Neo4j dual-write + `CORRELATED_WITH` 边 + `persist_correlation`(`neo4j_available` 恒 false);I/O-free 吃 `&Topology`/`&ChangeRegistry`/`&AlertRegistry`(reference 读全局 DSS `store`);丢 K8s watch + webhook(Phase 3 延后)
- [x] Phase 3.6:Tauri commands(recovery 18 / change_events 8 / alerts 5 = ~31 条)+ AntD 视图全量移植(react-router 嵌套布局 + react-query;ExecutionsView 折叠 ApprovalsView 单机确认门 confirm/cancel;RecoveryChainsView+DryRunModal;ChangeTimelineView drawer impact/alerts/suggestion;NodeDetailPanel 集成 actions+changes;TopologyView 加 onSelectNode)+ SQLite 持久化(change_events/recovery_chains/alert_events 3 新表,AppState 4 Mutex registry 启动 from_* 载入 + mutation 后 upsert)+ 接 sync 管线(拓扑消费 only:命令读 materialized topology 作 &mut twin,不写回;自动录 change event 留 k8s-watch)。mutable twin 传 owned clone;DTO 直接返 engine Serialize 类型(ChainTemplate 用 DTO);errors `[code] msg` 字符串。cargo test(clippy -D warnings;storage 14 + changes 66 + recovery 61)+ desktop tsc/vite build/vitest 14 绿;X11 启动 smoke 通(3 表 migrate + 4 registry 载入无 panic)。**偏差**:sync 拓扑消费 only(不自动录 change event)、alerts 无 live 源(仅手动 record_alert)、ApprovalsView 折叠进 ExecutionsView、真 handler 留 write-capability WIT 延后
- [x] Phase 3.7:k8s connector 边富化(`topology-edge` fact:SCHEDULED_ON[pod->node]/ROUTES_TO[svc->pod selector 匹配]/USES[pod->cm/secret,volumes+envFrom]+ ConfigMap/Secret node[只存 data_keys 不存值]);`facts_to_graph` 分流 topology-node/topology-edge(各自 dedup,edge 悬空+自环过滤,与派生 CONTAINS 合并);`latest_topology_facts` SQL `kind IN ('topology-node','topology-edge')`;engine-identity/engine-storage 零改(diff/apply 已支持任意 edge_type);engine-recovery/changes 零改(算法认全 8 白名单边,富化后自动生效)。真集群 headless:engine-cli tick k8s -> 78 node(含 4 cm+3 secret)+ 47 edge(22 SCHEDULED_ON+22 ROUTES_TO+3 USES)。**偏差**:不产 BELONGS_TO/DEPLOYED_AS/RUNS/EXPOSES(需 application/component 层);USES 只解析 volumes+envFrom(对齐 reference,不解析 env.valueFrom);Secret 只存 data_keys;edge id=`{src}->{tgt}`(K8s 同对资源无多关系不撞)。clippy+测试绿(engine-core 20/identity 17/storage 15/k8s mapper 11/changes 66/recovery 61)
- [x] Phase 3.8:k8s connector Application/Component/Middleware 层(从 deploy 派生:`normalize_component_name`[strip release prefix + 砍 service + 拆混淆名] + `detect_middleware`[valkey/redis/kafka/postgres/mysql] + `is_infra`[loadgenerator/otelcol/prometheus-server/jaeger/opensearch/grafana/kibana];`CONTAINS`[app->comp,comp.parent 派生] + `DEPLOYED_AS`[comp/mw->deploy] + `BELONGS_TO`[deploy->comp, comp->app 反向边,action BELONGS_TO forward 规则需要] edge fact;`ClusterInput` 加 `release_prefix`[默认 otel-demo];engine-core/identity/storage/changes/recovery 零改(3.7 已支持任意 edge_type + node type)。真集群 headless:engine-cli tick k8s -> 1 Application + 16 ApplicationComponent + 1 Redis + 1 Kafka + 97 edge(32 BELONGS_TO + 18 DEPLOYED_AS + 22 SCHEDULED_ON + 22 ROUTES_TO + 3 USES)。**偏差**:产 BELONGS_TO 反向边(reference k8s_mapper 不产,action BELONGS_TO forward 需要);Application parent=ns(reference 无 parent);component 名=normalize(deploy_name)不用 labels component;infra deploy 不挂 application。clippy+测试绿(k8s mapper 15[+4 3.8]/engine 回归 全绿)
- [ ] Phase 3 延后:真 handler(write-capability WIT)+ k8s-watch connector + webhook(桌面架构冲突,可能跳过)+ RUNS/EXPOSES 边(需 trace/ingress 层)
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
