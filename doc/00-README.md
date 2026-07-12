# 00 — 文档导航

本目录共 18 份文档(含本文件 + blog/1 篇),按"读者动机"分六组。建议新人从对应分组的第一篇看起。

## 📁 分组速查

### A. 入门 / 全局视图(必读)

| # | 文档 | 一句话 | 何时读 |
|---|------|--------|--------|
| 01 | [需求概述与平台目标](./01-requirements-overview.md) | 平台定位、四层模型、技术栈 | **第一篇** |
| 10 | [产品对标分析](./10-product-gap-analysis.md) | MVP 完成度 100% / v2 规划 20%,缺口分布 | 评估当前能力 |
| 13 | [Unknown Dep 端到端剧本](./13-story-unknown-dep-stripe.md) | Stripe trace → Queue → 代码仓 → 节点 | 想直观理解 UTS 怎么工作 |

### B. 数据模型(L1-L4 静态规约,实现稳定)

| # | 文档 | 一句话 |
|---|------|--------|
| 02 | [L1/L2 类型与实例模型](./02-L1-L2-type-and-instance-model.md) | 14 类型节点 + 35 关系 |
| 03 | [L3 动态观测模型](./03-L3-dynamic-metrics-model.md) | MetricQuery / Snapshot / AlertEvent |
| 04 | [L4 巡检结果模型](./04-L4-inspection-results-model.md) | InspectionRun / Rule / Finding |
| 08 | [故障类型与时间线](./08-fault-types-and-timeline.md) | 7 种故障 + 多级级联 + 每类型阈值 |
| 09 | [数据源服务 DSS](./09-data-source-service.md) | 内存孪生层,connector 写入入口 |

### C. 实现交付物(已落地的 PRD)

| 范围 | 文档锚点 | 状态 |
|------|---------|------|
| **PRD-001** 恢复动作引擎 | CLAUDE.md / README.md(本目录暂无独立 PRD 文档,以代码 + 工作文档为准) | ✅ 100% |
| **PRD-002** 变更事件追踪 | 同上 | ✅ 100% |
| **PRD-003** 自检报告 | 同上 | ✅ 100% |
| **PRD-004** OTel Demo 真实接入 | 同上 | ✅ 100% |
| 视图 / API / 前端 | [05](./05-six-views-design.md) / [06](./06-api-specification.md) / [07](./07-frontend-component-tree.md) | ✅ |

> **说明**:PRD-001/002/003/004 因迭代节奏快(每个 PRD 跨多个 Sprint + Phase 2 补丁),设计描述维护在 `CLAUDE.md` 的对应章节里,与代码同步。`doc/` 下的 PRD 文档只在**实施前的规划阶段**写入(见 D 组)。

### D. 规划中的 PRD(未实施,详细设计已就绪)

| # | 文档 | 一句话 | 优先级 |
|---|------|--------|--------|
| 11 | [PRD-005 统一拓扑服务 UTS](./11-PRD-005-universal-topology-service.md) | Fact 总线 + Identity Resolver 横切层,N→1 统一合并 | **下一步** |
| 12 | [PRD-006 代码仓数据源](./12-PRD-006-code-repo-source.md) | 业务规则 / PR-MR 事件 / 构建链路自动抽取,依赖 PRD-005 | 紧随 PRD-005 |

### E. 技术战略 / 架构规约(长期演进)

| # | 文档 | 一句话 | 何时读 |
|---|------|--------|--------|
| 14 | [长期技术战略](./14-long-term-tech-strategy.md) | 12 个月 Supervised Rewrite(Python → Rust+WASM)+ Tauri 桌面化 + 退出条件 | 决策入档,实施前必读 |
| 15 | [数据契约规范](./15-data-contract-spec.md) | 四层契约:WIT + Tauri Commands + Arrow + REST/Flight(headless) | 写代码前对照查 |
| 16 | [仓库与代码目录设计](./16-repo-and-codebase-layout.md) | Mono-repo 物理结构:engine/desktop/modules/specs/reference + Bootstrap 步骤 | Phase 1 开工时必读 |
| 17 | [Tauri 桌面架构](./17-tauri-desktop-architecture.md) | Tauri 2.x commands / IPC / 本地存储 / 跨平台打包 / 安全模型 | 桌面 app 开发前必读 |

### F. 技术博客 / 阶段复盘

| # | 文档 | 一句话 | 何时读 |
|---|------|--------|--------|
| blog/01 | [Phase 1 重写复盘](./blog/01-phase1-rewrite-decisions.md) | Python MVP → Rust+WASM+Tauri 的 A→G+最小拓扑视图决策复盘 | 向外介绍项目 / 新人快速理解重写路线 |

## 🧭 按角色选路

### 你是新加入的 SRE / 平台工程师
**01 → 02 → 09 → 05 → 13** — 看懂平台是什么、数据怎么建模、视图能查什么,最后用一个完整故事串起来。

### 你是来评估"还能做什么"
**01 → 10 → 11 → 12** — 看现状,看差距,看下一步两个 PRD 解决什么。

### 你是即将上手实施 PRD-005 / PRD-006 的开发
**14 → 15 → 16 → 17 → 11 → 12** — 先看长期技术战略 + 三层契约 + 仓库目录 + Tauri 架构,再看具体 PRD,**最后**配合 13 剧本对照,直接照 Sprint Plan 写代码。

### 你是来做技术选型决策 / 长期路线对齐
**14 → 17 → 15 → 16 → 10** — 战略 + Tauri 形态 + 契约 + 仓库 + MVP 起点,理解为什么 **Supervised Rewrite + Tauri 桌面 + Rust+WASM**(放弃 Strangler Fig / 放弃 SaaS Web)。

### 你只想看视图能展示什么
**05 → 07** — 6 视图 + 4 个 PRD 视图(审批中心 / 恢复历史 / 恢复链 / 报告中心 / 变更时间线 / Connector 状态),前端组件树。

## 🔗 PRD 间依赖

```
PRD-001 ──┐
PRD-002 ──┤── 已完成(MVP) ──┐
PRD-003 ──┤                 │
PRD-004 ──┘                 │
                            ▼
                    ┌── PRD-005 (UTS 底座)  ◄── doc/14 战略 + doc/15 契约
                    │     │
                    │     ▼
                    └── PRD-006 (代码仓,消费 Fact 总线 + WASM 规则)
                          │
                          ▼
                  v3 高阶:安全合规图层 / SLO 评分 / AI 活动建模
```

PRD-005 是 PRD-006 的硬前置 — 代码仓 connector 通过 Fact 总线注入,而不是直接写 DSS。
PRD-005 实施时遵循 doc/14 的 Rust+WASM 路径 + doc/15 的三层契约。

## 📜 演进时间线

```
2026 Q1   PRD-001/002/003/004 全部上线(MVP 100% 完成)
2026 Q2   ✅ Phase 0 + Phase 1 完成:A→G + mock 拓扑视图 + Blog Part 1 + GUI verifier + 首屏 polish
  ↓
2026 Q3   ▶ 当前 — Phase 2:2.1 SQLite-backed topology persistence ✅ + 2.4 GraphResponse DTO ✅ + 2.5 Identity Resolver v0(ChangeSet + materialized 表)✅ + 2.6 Prometheus connector(首个 http-client capability 消费方)✅ + 2.6b 真实 K8s connector(via kubectl proxy,真集群验证)✅ + 2.7 metric->topology health 合并(engine-identity field-ownership v0)+ desktop 托管 kubectl proxy + per-connector manifest config + 真集群拓扑真色(fill=health/border=risk)✅
2026 Q4   Phase 2 续:5 connector WASM 化 + SQLite/Parquet 存储 + Tauri 视图迁
2026 Q4   Phase 3 起步 - 3.1 engine-recovery action_defs(8 action 元数据 + propagation + rule/change 推荐)+ cascade dry_run(BFS blast radius,I/O-free 吃 &Topology)✅;审批语义定(桌面单机确认门,doc/14 §9)
2027 Q1   Phase 3:PRD-006 + 复刻 PRD-001/002(⚠️ PRD-001 审批流桌面语义需在此 Phase 定)
2027 Q2   Phase 4:复刻 PRD-003/004 + v1.0 release(macOS/Linux/Windows)
2027 Q3   Buffer / 社区 / 技术分享
```

## ✅ 当前验证基线

- Phase 1 桌面 GUI 验证走 `.claude/skills/verifier-tauri-gui/`:强制 `GDK_BACKEND=x11 npm run tauri dev`,点击/激活 `Sync all now`,观察 `k8s-mini sync: cluster=demo namespaces=2 with_topology=true`,并用截图像素确认 Cytoscape 绿色拓扑节点。
- Phase 2.1 持久化验证:`SRE_GRAPH_DB_PATH=/tmp/x.sqlite` fresh 启动拓扑空 → sync 后写入 SQLite(34 facts / 12 resources)→ 重启同一 DB 不再 sync,`get_topology` 从库恢复拓扑(日志无新 `sync invoked`,绿色节点仍渲染)。
- Phase 2.4 GraphResponse 验证:seed SQLite 一棵 K8s 拓扑(Cluster/Node/2×Namespace/2×Pod/Service)→ 重启 app,boot 调 `get_graph`(`facts_to_graph` → `GraphResponse{nodes:7,edges:6,summary}`)→ 前端 `graphToElements` 成图,Cytoscape 渲染层级拓扑(hexagon→octagon/round-rect→ellipse/diamond,9.2k 绿色像素,header 显示「7 node · 6 edge」),全程不 sync。
- Phase 2.5 Identity Resolver v0 验证:`engine-identity` 8 单测(`resolve` 去重+派生边、attributes canonical 排序、`topology_to_graph` 与 `facts_to_graph` 等价、summary 重算)+ `engine-storage` `materialized_topology_round_trips_resolve_diff_apply`(对真实 SQLite 跑 resolve→diff→apply→回读 + remove 分支);GUI-less 端到端:seed materialized `topology_nodes`/`topology_edges` → `cargo run --example dump_topology` 走 `get_graph` 读路径(`materialized_topology` + `topology_to_graph`)产出 `GraphResponse{nodes:7,edges:6, risk{high:1,low:4,medium:2}, health{critical:1,normal:4,warning:2}}`,字段对齐 reference。注:本环境沙箱拦截 GUI 进程伴随 capture 的启动(exit 144),无法新截图,故新读路径走 headless 验证 + 2.4 GUI 基线传递性覆盖(`topology_to_graph(resolve(f)) == facts_to_graph(f)` 已单测,前端 `graphToElements` 与 2.4 字节一致)。
- `desktop/src/views/TopologyView.test.ts` 覆盖 `graphToElements`(GraphResponse → Cytoscape elements:nodes-first 顺序、type→shape、`type\nlabel` 标签、edge `edgeType` 透传、空图);`engine-core` `graph.rs` 8 个单测覆盖去重/父子边/悬空过滤/label 优先级/risk·health 固定桶统计/非 topology 忽略/serde 字段名;`engine-identity` 8 单测;`engine-storage` 7 个 SQLite 单测覆盖 migrate/upsert/latest_topology/materialized 往返。
- Phase 2.6 Prometheus connector 验证:`modules/connectors/prometheus`(首个消费 `http-client` capability 的 guest)`cargo wasi-build` 出 `prometheus.wasm`;host e2e `engine/crates/engine-wasm/tests/prometheus_http_e2e.rs` 两 case —— (1) mock Prom HTTP server 返 canned JSON + 申明 `http-client` → guest GET `/api/v1/query` 拿 bytes → 解析两条 sample 成 metric Fact(`service:local:otel-demo:cartservice` value=42.5 / `frontend` value=7);(2) 不申明 `http-client` → host deny-by-default 返 `Unauthorized`,guest 0 fact + error note。`engine-cli tick` 确认 3 connector 全加载、prometheus 空 url 优雅跳过。
- Phase 2.6b 真实 K8s connector 验证:`modules/connectors/k8s`(经本地 `kubectl proxy` 明文 HTTP 拉 API server,TLS+认证留 proxy)。纯 mapper 7 个 host 单测(canned K8s JSON:owner 链 Pod→RS→Deploy、悬空 owner 退化 namespace、health normal/warning/critical、Node Ready 条件、全层级 parent);**真集群 e2e**(`tests/k8s_live_e2e.rs`,env `K8S_PROXY_BASE` gated)对真实 otel-demo:`kubectl proxy --port=8001` → sync 拉 **71 fact / 0 error**(Node=3 / Deployment=22 / Pod=22 / Service=22 + cluster + ns),pod parent 全解析无悬空;再走 `engine_core::facts_to_graph` 成 `GraphResponse{nodes:71, edges:70, risk{high:1,medium:4,low:66}, health{critical:1,warning:4,normal:66}}` —— crashloop 的 cartservice 真实显示 critical。

- Phase 2.7 metric->topology health 合并 + kubectl proxy 托管 + 真集群拓扑验证:(1) `engine-identity` `health_merge` 8 单测(metric 阈值边界、worst-severity 合并、不降级 k8s critical、多 metric 取最严重、attributes canonical 重排);(2) `engine-storage` `latest_metric_facts`(最新 sync 的 metric)+ `select_config`(per-connector manifest config 分发)单测;(3) `commands/proxy.rs` 5 单测(kubectl 路径解析 + TCP 就绪探测 mock);(4) TopologyView vitest 7(`healthFill`/`riskBorder` + `data(fill)`/`data(borderColor)` mapper 上色);(5) **真集群 headless**:`kubectl proxy --port=8001` -> `K8S_PROXY_BASE=http://127.0.0.1:8001 cargo test k8s_live_e2e` 71 fact/0 error(Node=3/Deploy=22/Pod=22/Svc=22,health{normal:67,warning:4});`engine-cli tick` 用真实 manifest 经 per-connector config(api_base 从 manifest,非 CLI)拉 k8s **71 fact**,prometheus OOM 0 fact(符合预期,merge no-op)。GUI 渲染(真色)由用户 X11 验。
- Phase 3.1 engine-recovery 验证:`action_defs` 7 单测(8 action 元数据完整性、high_risk 必审批、kill_query 例外、list_actions 过滤、rule/change 推荐)+ `cascade::dry_run` 11 单测(3 invalid 路径 + scale/restart_pod/drain_node/refresh_secret 4 propagation + severity 取 max + rollback 参数 + 排序;合成 9 节点 11 边拓扑,移植 reference `TestActionDefs`/`TestDryRun`)。逐字对齐 reference `action_defs.py`/`cascade.py`;clippy `-D warnings` + 20 测试绿。审批语义定(桌面单机确认门),3.2 落地 execute 管线。
- Phase 3.2 engine-recovery execute/approval/rollback 验证:`models`(RecoveryExecution 全 24 字段 + RecoveryStatus/VerifyStatus/ExecutionContext/ExecutionError)+ `handlers` 8 mock handler(纯函数,校验 target 类型 + 参数,返 flat result dict;只 mock 不 mutate topology)+ `execution`(ExecutionRegistry + execute/confirm/cancel/rollback)。contract test 移植 reference `test_recovery_execute`/`test_recovery_approval`:risk->status(low=succeeded 同步 / medium·high=awaiting_approval)+ confirm 跑 handler->succeeded + cancel->rejected + rollback 矩阵(scale 反向 delta + only-succeeded 409 + idempotent 409 + no-rollback-action 400 + 404)+ 404/400/409 错误码。engine-storage `recovery_executions` 表 + upsert/get/list round-trip(幂等)。clippy + 39(engine-recovery)+ 11(engine-storage)测试绿。偏差:I/O-free(registry 入参非全局 DSS)、无 ApprovalRequest 实体/TTL/approver_team(单机确认门)、mock handler 3.3 起改 mutate twin(见下 3.3)。
- Phase 3.3 engine-recovery verifiers/auto-rollback/chains 验证:**mutable twin 架构**(handler 改 `fn(&mut ResolvedNode,&Value,&ExecutionContext)->Value`,写回 `attributes_json` 模拟动作生效:scale 写 desired/available_replicas、restart_pod 写 restart_count+health_status、rollback 写 current_revision、refresh_secret 写 secret_version、drain_node 写 cordoned、restart_service 写 endpoints_refresh_count)。`verifiers` 8 个(读 mutated attrs 验 predicate;`kill_query`/`clear_cache` 无可观测副作用 -> `not_supported` passed=true)+ `run_verifier`(verifier set 可注入,测试 fake failing verifier 触发 auto-rollback,对齐 reference monkeypatch VERIFIERS)。`execution` 3.3 重写全管线 `&mut Topology`:`run_handler`(mutate twin + succeeded&verify 跑 verifier)+ `verify_and_maybe_rollback`(verify_failed + auto_rollback + 有 rollback_action_id -> `do_rollback(marker=true)` 反向 exec;无则 warn)+ `do_rollback`(反向 exec `verify=!marker` 防递归)+ `reverify`(手动重验)。auto-rollback 测试:fake failing verifier -> status=rolled_back + result.auto_rollback.triggered=true;无 rollback_action_id -> warn 不回滚。scale rollback 读 post-action mutated state 反转 delta(new_replicas==3 正确)。`chains` 3 模板 + `execute_chain`(链级审批:任一步非 low/requires_approval -> awaiting_approval,全 low 直接跑)/`confirm_chain`/`cancel_chain`(->Failed)/`abort_chain`(->Aborted)+ `run_chain_steps`(on_failure:Stop->Partial / RollbackAll->RolledBack 反向 do_rollback 已成 prior step / Continue 记录并续跑)+ `run_single_step`(auto_rollback=false,verify=step.verify_required,失败交 chain on_failure)。contract test 移植 reference verify/auto-rollback + chain on_failure 矩阵。clippy + 61(engine-recovery)测试绿。偏差:twin 显式 `&mut Topology` 入参而非全局 DSS(reference 读 DSS properties);orchestration 层(3.6)应传 clone 避免污染源 materialized 拓扑;real verifier(查真 K8s/MySQL/Redis)留 write-capability WIT 延后。
- Phase 3.4 engine-changes ChangeEvent/record_change/propagation 验证(PRD-002 复刻起手):`models`(`ChangeEvent` 18 字段对齐 reference dataclass + Phase 2 字段一次性全收;`ChangeType` 4 / `Source` 6 / `Severity` 3 强类型枚举 snake_case 序列化,reference 用 plain str + `VALID_*` 集合校验;`ChangeRequest` 14 字段入参结构体避免 too_many_arguments;`ChangeFilter` type/target/source/since/until;`ChangeError` message+code 对齐 `ChangeEventError`)。`propagation`(`PROPAGATION_EDGES` 8 白名单 USES/CONTAINS/DEPLOYED_AS/BELONGS_TO/RUNS/SCHEDULED_ON/EXPOSES/ROUTES_TO,**不含 USES_IMAGE**)+ `derive_propagation`(反向 BFS,incoming[edge.target]->edge.source,max_depth=4,排除自身,target 不在拓扑 -> [])+ `find_propagation_path`(反向 BFS 最短路径 + parents 回溯,source==affected/不可达 -> [])+ `find_descendants`(镜像 forward BFS,max_depth=6)。全 I/O-free 吃 `&Topology`(reference 读全局 DSS `store`)。`event_service`(`ChangeRegistry` in-memory add/get/list/clear,插入序 + ISO8601 字典序闭区间过滤;`estimate_severity` >=10 high/>=5 medium/else low;`record_change` 校验 type/source 400 + `ce-<12hex>` ID + 查 target_resource_type 或 "" + propagated_to 反向 BFS + severity + commit_sha 回填 related_commit)。contract test 移植 reference `TestChangeEventModel`/`TestStore`(add/get/filter type/target/time-window)/`TestPropagation`(configmap->pods 1 跳 / secret->app 4 跳 / orphan / max_depth=1 截断 / USES_IMAGE 跳过 / unknown / propagation_path)/`TestRecordChange`(basic / severity 边界 / real propagation / target 不在 dss / invalid type / invalid source),合成 10 节点 10 边 fixture(app-contain-comp-deployed_as-deploy-contain-pods-uses-{cm,secret} + svc-routes_to-pod1 + deploy-uses_image-img + orphan)。clippy + 29(engine-changes)测试绿。偏差:丢 Neo4j dual-write + alert 关联(best-effort 副本,3.5 接 alert 关联)、不调 `_apply_frequency_check`(3.5)、v0 拓扑只有 CONTAINS 边故生产传播只沿 CONTAINS 反向(算法认全 8 种白名单边,待 Phase 3 延后的 k8s connector 边富化 USES/ROUTES_TO 接入)。
- Phase 3.5 engine-changes frequency/alert/yaml_diff/correlated/suggestion 验证(PRD-002 复刻续):`iso`(共享 `now_iso`/`parse_iso_utc`/`shift_iso`,抽离原 event_service 私有 `now_iso`)。`yaml_diff`(`NOISE_KEYS` 10 managedFields/resourceVersion/uid/creationTimestamp/generation/selfLink/etag/last-applied-configuration/annotations/managedVersion + `strip_noise` 递归 + `select_keys` + `compute_yaml_diff(old,new,keys,name)` **自写确定性 block-style 发射器排序键,不引 serde_yaml**(离线风险规避);Value 结构相等短路 -> "";否则 `similar` unified_diff + `summarize_diff`->{added,removed,changed_keys})。`alerts`(`AlertEvent` 13 字段 + `AlertSeverity` warning/critical + `AlertStatus` firing/resolved + `AlertRegistry` in-memory add/get/list(fired_at 闭区间))。`frequency`(`DEFAULT_WINDOW_SECONDS=3600`/`DEFAULT_THRESHOLD=5`,严格 `>`;`check_target_frequency` 单 target + `detect_frequent_changes` 全 registry 桶聚合 count>threshold desc + `apply_frequency_check` 只升 low->medium + append `[过频变更:N次/Ws]` tag)。`alert_correlation`(`correlate_alerts(affected={target}∪propagated_to,窗 [changed±window],resource_ref 命中)`+ 反向 `correlate_changes_for_alert`;`CorrelatedChangeForAlert` 字段用 ChangeType/Source/Severity 枚举而非 Debug 字符串)。`event_service` 3.5 扩展:`record_change` add 后调 `apply_frequency_check`(计数含当前,回写 severity/description via `get_mut`,对齐 reference add-then-check)+ `serialize`(全字段 + propagated_count)+ `correlated_changes(target,window=300,since,until,include_propagated,&ChangeRegistry,&Topology)`(direct/propagated + 三种时间窗模式 since/until/window + propagation_distance=max(path-1,1) + changed_at 倒序)+ `get_recovery_suggestion(event_id)`(404;桥 `engine_recovery::suggest_for_change`,嵌套 ActionSuggestion -> 扁平 RecoverySuggestion,target_match direct/propagated/unresolved)。contract test 移植 reference `TestYamlDiff`(strip_noise/相等短路/diff 内容/summarize)/`TestFrequencyAlert`(过频升 medium+tag/阈值下不变/单 target/全局 detect)/`TestCorrelatedQuery`(direct+propagated/time-window/include_propagated/distance)/`TestRecoverySuggestion`(direct/propagated/unresolved)/`TestAlertCorrelation`(正向 resource_ref 命中/反向/窗内外/404),8 节点 7 边 phase2 fixture + 10 节点 sprint1 fixture。clippy + 66(engine-changes)测试绿(29 from 3.4 + 37 new)。偏差:丢 Neo4j dual-write + `CORRELATED_WITH` 边 + `persist_correlation`(`neo4j_available` 恒 false);I/O-free 吃 `&Topology`/`&ChangeRegistry`/`&AlertRegistry`(reference 读全局 DSS `store`);丢 K8s watch + webhook(Phase 3 延后);自写 YAML 发射器不引 serde_yaml(规避离线编译风险,deterministic block-style 排序键足够覆盖 reference diff 契约)。
- Phase 3.6 Tauri commands + AntD 视图 + SQLite 持久化(PRD-001/002 接桌面):**engine-storage** 加 `change_events`/`recovery_chains`/`alert_events` 3 表 + `upsert/get/list_*`(enum 列存 snake_case JSON 文本,Vec/Value 存 JSON 文本;3 round-trip 测试)。**engine-changes/recovery** 加 `from_events`/`from_chains`/`from_alerts`/`get_mut`/`list` 构造器(启动从 storage 载入)。**desktop/src-tauri** `AppState` 加 4 `Mutex` registry,`setup` 启动 `from_*` 载入(重启恢复);3 新 command 模块 ~31 条命令(recovery 18:actions/dry_run/execute/executions CRUD/confirm/cancel/rollback/reverify + chains;change_events 8:record/list/get/correlated/frequent/impact/recovery_suggestion/alerts;alerts 5:record/list/get/resolve/correlate_changes_for_alert)。**mutable twin**:mutation 命令读 `materialized_topology()` owned clone 作 `&mut Topology`(handler mock 模拟不写回 materialized 表),engine 同步调用在锁内 / storage upsert 异步在锁外(`Mutex`Guard 不跨 await);每次 mutation 后 upsert 回 SQLite。**DTO** 直接返 engine `Serialize` 类型(`ChainTemplate`/`ChainStep` 未 Serialize -> `ChainTemplateDto`);错误 `[code] msg` 字符串。**前端**迁 AntD(`antd`6+`react-router-dom`6+`@tanstack/react-query`5):`main.tsx` 包 QueryClientProvider+HashRouter+ConfigProvider;嵌套布局路由(MainLayout Sider 菜单 + Outlet);`api/client.ts` invoke 封装(axios->invoke);`TopologyPage`(react-query + 节点点击 NodeDetailPanel 集成 RecoveryActionsSection+ChangeTimelineSection)+ `RecoveryPage`(`ExecutionsView` Table+Drawer 折叠 ApprovalsView 单机确认门 confirm/cancel/rollback/reverify + `RecoveryChainsView` + `DryRunModal`)+ `ChangesPage`(`ChangeTimelineView` drawer impact/alerts/suggestion + 记录表单 + `AlertsView`)。TopologyView 加 `onSelectNode` prop。验证:cargo test(clippy -D warnings;storage 14 + changes 66 + recovery 61)+ desktop tsc/vite build/vitest 14(7 既有 graphToElements + 7 api/client invoke 包装)+ X11 启动 smoke(3 表 migrate + 4 registry 载入无 panic,wasm 2 connector 加载)。偏差:**接 sync 管线 = 拓扑消费 only**(命令读 materialized topology 作 twin,不自动录 change event -- 留 k8s-watch);**审批 = 单机确认门**(ApprovalsView 折叠进 ExecutionsView,无 ApprovalRequest/TTL/approver_team);alerts 无 live 源(仅手动 record_alert,k8s-watch/webhook 延后);DTO 直接返 engine Serialize 类型(非手写,区别于 wasm.rs Fact 模式);真 handler 留 write-capability WIT 延后。
- Phase 3.7 k8s connector 边富化(USES/ROUTES_TO/SCHEDULED_ON `topology-edge` fact + ConfigMap/Secret node):`facts_to_graph` 分流 topology-node/topology-edge(各自 dedup,edge 悬空+自环过滤,与派生 CONTAINS 合并);`latest_topology_facts` SQL `kind IN ('topology-node','topology-edge')`;engine-identity/engine-storage/engine-recovery/engine-changes 零改(diff/apply 已支持任意 edge_type;cascade/propagation 算法认全 8 白名单边,富化后自动生效)。k8s mapper 产 SCHEDULED_ON(pod->node nodeName)+ ROUTES_TO(svc->pod selector 匹配 labels)+ USES(pod->cm/secret,volumes.configMap/secret + envFrom.configMapRef/secretRef;不解析 env.valueFrom 对齐 reference)+ ConfigMap/Secret node(只存 data_keys 不存值);`edge_fact` helper(resource_id=`edge:{type}:{src}->{tgt}` 含 edge_type 避免撞 latest 去重)。真集群 headless 验证:engine-cli tick k8s(via kubectl proxy 8001,nightly toolchain)-> 78 topology-node(1 cluster+3 node+1 ns+22 deploy+22 pod+22 svc+4 cm+3 secret)+ 47 topology-edge(22 SCHEDULED_ON+22 ROUTES_TO+3 USES)。偏差:不产 BELONGS_TO/DEPLOYED_AS/RUNS/EXPOSES(需 application/component 层);USES 只解析 volumes+envFrom(对齐 reference);Secret 只存 data_keys;edge id=`{src}->{tgt}`(K8s 同对资源无多关系不撞)。clippy+测试绿(engine-core 20[+4 edge]/identity 17[+1]/storage 15[+1]/k8s mapper 11[+5 edge]/changes 66/recovery 61)。
- Phase 3.8 k8s connector Application/Component/Middleware 层(对照 reference k8s_mapper `_make_application`/`normalize_component_name`/`detect_middleware`):从 deployment 派生 Application(1 个 per c:ns:release,parent=ns)+ ApplicationComponent(`normalize_component_name`[strip release prefix + 砍 service + 拆混淆名 frauddetection->fraud-detection 等],parent=app 派生 CONTAINS)+ Middleware(`detect_middleware` valkey/redis/kafka/postgres/mysql -> Redis/Kafka/PostgreSQL/MySQL);infra deploy[loadgenerator/otelcol/prometheus-server/jaeger/opensearch/grafana/kibana]不挂 application)。边:`CONTAINS`[app->comp,comp.parent 派生] + `DEPLOYED_AS`[comp/mw->deploy edge fact] + `BELONGS_TO`[deploy->comp, comp->app 反向边,action_defs 8 个 action 的 BELONGS_TO forward 规则需要;reference k8s_mapper 不产 BELONGS_TO,Rust 补反向]。`ClusterInput` 加 `release_prefix`[默认 otel-demo,config_json 传]。engine-core/identity/storage/changes/recovery 零改(3.7 已支持任意 edge_type + node type)。真集群 headless 验证:engine-cli tick k8s -> 1 Application + 16 ApplicationComponent + 1 Redis + 1 Kafka + 97 topology-edge(32 BELONGS_TO[16 comp->app + 16 deploy->comp] + 18 DEPLOYED_AS[16 comp->deploy + 2 mw->deploy] + 22 SCHEDULED_ON + 22 ROUTES_TO + 3 USES)。偏差:产 BELONGS_TO 反向边(reference 不产,action 规则需要);Application parent=ns(reference 无 parent,层级完整);component 名=normalize(deploy_name)不用 labels component;infra 不挂 application。clippy+测试绿(k8s mapper 15[+4:normalize/detect_middleware/is_infra/application_layer]/engine 回归全绿)。

## 📝 文档约定

- 每份 PRD 文档统一 **14 节**:背景 / 设计原则 / 目标架构 / 核心契约 / 关键算法 / 工业对标 / Sprint 计划 / 验收 / 风险 / 不做 / File Map(其余按需)
- L1-L4 模型文档稳定,改动需在 CHANGELOG 留痕
- 已实施 PRD 的"在做"细节走 `CLAUDE.md`,**不**塞进 `doc/`(避免实施波动污染规划文档)
- 跨文档链接用相对路径 `./11-PRD-005-...md`,不用绝对 URL

## 🔍 找东西

```bash
# 找 PRD-005 / PRD-006 相关
grep -l "PRD-005\|PRD-006" doc/*.md

# 找某个数据模型术语(如 TopologyFact)
grep -rn "TopologyFact" doc/

# 找某个节点类型(如 CodeRepo)
grep -rn "CodeRepo" doc/
```
