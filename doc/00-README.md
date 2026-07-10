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
