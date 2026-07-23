# Phase 2-5 复盘:从「计划 7 块」到与 Python 旧栈 feature parity

> 日期:2026-07-23
> 项目:SRE Inspection Graph / 云原生巡检图谱平台
> 阶段:Phase 2 → 5(实数据 + 持久化 → 4 PRD 复刻 → 6 巡检视图 → feature parity)
> 关键词:Identity Resolver v0,Mutable Twin,单机确认门,poll-diff watch,Tera,subgraph 原语,resource_type 词表漂移
> 承接:[Part 1 — Phase 1 重写决策](./01-phase1-rewrite-decisions.md)

## 0. 起点:Part 1 末尾的那张「Phase 2 七块」清单

Part 1 结束时,Phase 1 把架构闭环画清楚了:

```text
WIT 契约 → WASM module → wasmtime host → canonical Fact → Arrow → Tauri IPC → React → Cytoscape
```

当时 Phase 2 列了 7 块:FactBus、Identity Resolver、K8s/Prometheus/Jaeger/flagd/k8s-events connector WASM 化、SQLite+Parquet storage、视图迁移。

实际走下来,这 7 块**没有一块是照原计划做的**。不是计划错,是走着走着发现真正的难点和真正的捷径都不在那张清单上。这篇复盘讲的是 Phase 2-5 的**关键取舍**,不是增量 changelog(CLAUDE.md 里已有完整进度表)。

走完这四个 Phase,新栈与 Python `reference/`(4 个 PRD、12.7k 行、472 测试)达到 **feature parity**。`reference/` 作为 read-only 行为 oracle 的对照职责基本完成。

---

## 1. Phase 2:实数据 + 持久化,三个取舍

### 1.1 取舍一:SQLite materialized topology,而不是 FactBus + DataFusion

Part 1 计划里 Identity Resolver 写的是「DataFusion / SQLite UPSERT 组合」。真做的时候发现:**桌面单机场景下,根本不需要 FactBus 那套多生产者 MPSC**。所有 connector 在一次 `sync_all` 里串行跑完,产出一批 Fact,直接落 SQLite。

于是数据路径收敛成:

```text
sync_all_now
  → WasmRuntime::sync_all          (N 个 connector 串行,聚合一坨 Fact)
  → engine_core::facts_to_graph    (Fact → GraphResponse,去重/派生边/悬空过滤)
  → engine_identity::resolve       (平移成持久化形态 Topology)
  → engine_identity::health_merge  (prometheus metric → health,合进节点)
  → engine_identity::diff          (上次 materialized → 本次,产 ChangeSet)
  → storage::apply_change_set      (单 tx upsert + delete stale)
```

`topology_nodes` / `topology_edges` 两张物化表是 **single source of truth**。`get_graph` 直接读物化拓扑反建 `GraphResponse`,不再每次从 raw Fact 重算。

FactBus + DataFusion 留给真正需要它的场景:多源实时合并(那是 PRD-005 Identity Resolver v1 的事,见 §10)。**没有流量压力时,进程内一次同步 + SQLite 物化表就是最简单的正确答案。**

### 1.2 取舍二:Identity Resolver v0 直接拿 resource_id 当 canonical 键

这是整段重写里**最显式的 defer**。

PRD-005(doc/11)设计的 Identity Resolver 要做 correlation-key 合并:trace 里看到 `tcp://10.0.1.5:3306` + Cloud API 说 RDS 的 internal IP 是 `10.0.1.5` → 同一节点。这需要一套 correlation key 规范(`ip:` / `endpoint:` / `arn:` / `cluster_dns:` …)+ 冲突仲裁规则。

但 Phase 2 只有 K8s 一个真数据源。K8s connector 自己产出的 `resource_id` 已经是 canonical 的(`pod:{cluster}:{ns}:{name}`),**单源场景下根本没东西可合并**。

于是 v0 的规则极简:

```rust
// v0:resource_id 直接当 canonical 身份键
// 不做 correlation-key 模糊合并 / 冲突仲裁
// (那是 PRD-005 完整版,见 doc/11 §4-5)
```

这个 defer 的价值在于:**它让 Phase 2-5 的所有上游(Persistence、Recovery、Changes、Reports、Views)都能在一个稳定的身份键假设上往前跑**,而不用等最难设计的合并算法先就位。等 Phase 6 真接第二个异构源(Cloud API / Trace)时,合并算法才有真实的冲突数据来验证。

**v0/v1 分层是这个项目的核心节奏控制** —— 把难设计点显式 defer 到文档写明的地方,先用最简版本把骨架立起来,让下游不阻塞。health_merge、watch、handler 都是同款打法。

### 1.3 取舍三:kubectl proxy 架构 —— WASM 不碰凭据

Phase 2.6b 接真 K8s API 时,本能反应是「给 WASM 一个 kube capability,让它读 kubeconfig 直连 apiserver」。但这条路有三个麻烦:

- kubeconfig 是敏感凭据,塞进 WASM guest 的 capability 边界违反 deny-by-default 的初衷
- TLS + token 刷新 + cert 轮换,这套在 WASI p2 下 async/network 抽象很别扭
- 多集群时每个 guest 一份 kubeconfig,隔离反而更难

最后选了 **kubectl proxy 架构**:

```text
desktop 托管 `kubectl proxy --port=8001`(子进程,TCP 就绪探测,RunEvent::Exit kill 防孤儿)
        ↓ 明文 HTTP 到 127.0.0.1:8001
WASM k8s connector 只用 http-client capability(GET /api/v1/...)
```

**TLS + 认证全部留在 proxy 进程**(它读宿主机的 kubeconfig),WASM guest 永远只见到明文 HTTP localhost。凭据不进 capability,不需要新 capability 类型,真集群 otel-demo 一次拉 71 个 Fact 跑通。

代价是依赖宿主机装了 kubectl + 有合法 context。对「SRE 工作站」这个目标用户来说,这是合理假设 —— 他们本来就在用 kubectl。

---

## 2. Phase 2.7:health 合并的 field-ownership v0 —— 最严重胜出

Prometheus 接进来后,拓扑节点同时有两个 health 来源:K8s connector 从 pod phase/ready 推的结构性 health,和 Prometheus metric 推的观测性 health。冲突时谁赢?

PRD-005 doc/11 §4.3 设计了完整的 field-ownership 表(每个字段指定哪些 source 能写)。但 v0 先上最朴素的规则:

```rust
// field-ownership v0:最严重胜出
// critical > warning > normal
```

不区分字段、不查 ownership 表,任何来源只要说 critical,节点就是 critical。简单、保守、可预测 —— 对一个巡检工具来说,**宁可误报不可漏报**。

完整 field-ownership 表(每个 attr 谁能写)同样 defer 到 Identity v1。

---

## 3. Phase 3:复刻 4 PRD,两个最大的语义简化

Phase 3 是体量最大的一段:把 reference 的 PRD-001(Recovery)/ 002(Changes)/ 003(Reports)逐字复刻。这里有两个**主动偏离 reference** 的设计决策,都记在 doc/14 §9。

### 3.1 审批语义:桌面单机确认门,丢掉多人审批

reference 的 Recovery 有完整审批实体:`ApprovalRequest` + `approver_team` + 24h TTL + 多人 approve/reject。这是给「团队协作的 Web 控制台」设计的。

新栈是桌面单机工具。一个人在本地操作,搞一套多人审批流水线纯属过度设计。于是砍成**单机确认门**:

```text
low risk      → 同步执行(200 语义)
medium/high   → awaiting_approval,本机点 Confirm / Cancel
rollback      → 跳过二次审批(原始动作已审,反向是"撤销"不是新风险)
```

保留的是 risk → status 的映射逻辑(这是真正有价值的部分),丢掉的是多人协作状态机。**复刻行为规约,不复刻部署形态带来的负担。**

### 3.2 Mutable Twin:让 mock handler 真的"生效",verifier 真能验

reference 的 recovery 真模式调 K8s/MySQL/Redis handler 改真集群,成功后才更新内存孪生。Phase 3.2 一开始只做了 mock handler,但 mock 怎么"生效"?

答案是 **mutable twin 架构**:

```text
orchestration 层读 materialized_topology 的 owned clone 作 &mut Topology(源不污染)
   ↓
handler 拿 &mut ResolvedNode,写回 attributes_json 模拟动作生效
  (scale_deployment 写 desired/available_replicas;restart_pod 写 restart_count + health_status;…)
   ↓
verifier 读 handler 写回的 mutated attrs 验 predicate
   ↓
verify_failed + auto_rollback → 反向 exec(读 post-action 状态做正确反转)
```

关键点是 **rollback 要读 post-action 状态才能正确反转** —— 比如 scale 4→6 后 rollback,得知道动作前的 4,而不是硬编码回 1。twin 让 handler / verifier / rollback 三者共享同一份 mutated 视图。

这套架构的回报在 Phase 3.9a-3b2 接 real handler 时兑现:把 mock handler 换成 `WasmHandlerExecutor`(WASM 调 K8s API 真改集群),**mutable twin + verifier 管线零改动** —— 只是 handler 的实现从"写 attrs"变成"发 PATCH 请求 + 写 attrs"。

---

## 4. Phase 3.7-3.9:k8s 边富化让下游算法自动生效

Phase 3.7 之前,k8s connector 只产 Deployment/Pod/Service/Node + 派生的 CONTAINS 边。Recovery 的 propagation BFS、Changes 的 derive_propagation 都只沿 CONTAINS 反向走 —— 爆炸半径算出来很窄,跟 reference 的语义对不上。

Phase 3.7-3.9 是一连串**纯 mapper 增强**:connector 产出更多 edge_type fact(`topology-edge` kind),`facts_to_graph` 加分流逻辑把 node fact 和 edge fact 分别去重/过滤。

```text
3.7  SCHEDULED_ON(pod→node) / ROUTES_TO(svc→pod) / USES(pod→cm/secret)
3.8  Application/Component/Middleware 层 + CONTAINS / DEPLOYED_AS / BELONGS_TO
3.9  RUNS(pod→container) / EXPOSES(svc→deploy)
3.9b ContainerImage 节点 + USES_IMAGE(container→image)
```

**最爽的一点**:Recovery 和 Changes 的算法早就写成了「吃 8 种白名单边的 I/O-free 纯函数」。connector 富化产了新边类型,这些算法**一行没改就自动沿新边传播**。真集群从 47 edge 一路涨到 328 edge,blast radius / config-impact 越来越接近 reference Cypher 的语义。

这是「领域逻辑与数据采集分离」的直接收益 —— 算法认 edge_type 白名单,不认具体 connector。

---

## 5. PRD-003/002 后续:poll-diff watch,而不是 streaming

reference 的 K8s watcher 是真 streaming watch。新栈 defer 了它,理由很实在:**真 watch 需要 WIT stream + WASI p3(async-native),这套 ABI 还没稳定**。

退而求其次做的是 **poll-diff**:

```text
后台 tick_loop(interval 30s,env 可配)
  → 每次 sync 拉 current topology
  → detect_changes(current, next)  纯函数,对比信号字段
  → 只看 ConfigMap/Secret/Deployment 的信号字段(current_revision / images / data_keys)
  → compute_yaml_diff 剥 10 个噪声字段(managedFields/resourceVersion/uid/…)
  → record_change(source=k8s_api)
```

首次 sync 抑制(`first_sync_done` AtomicBool),避免重启 burst 误录一堆"变更"。

代价是延迟 ≤30s,以及 ConfigMap/Secret 的 value-only 变更漏检(不存 data 值,只存 data_keys)。对巡检场景,30s 延迟完全可接受;value-only 漏检是显式 trade-off(存 secret 值本身就有合规问题)。

**真集群验证**:手动 `kubectl rollout restart` emailservice(revision 1→2),后台 tick 10s 后 `detect_changes` 录入 `deployment_rolled`,yaml_diff 精确只显 `current_revision: 1 -> 2`。poll-diff 的语义正确性靠真集群证明,不是靠合成测试。

---

## 6. Phase 4:Reports —— Tera 替 Jinja2,自写 cron loop

reference 的报告用 Jinja2 + APScheduler。新栈两个偏离:

**Tera 替 Jinja2**:Tera 是 Rust 原生模板引擎,语法接近 Jinja2 但能编译进二进制,不需要 Python 运行时。迁移时踩了一个坑:Tera 1.20.1 对中文 key 的 bracket 索引(`rating_counts["健康"]`)会落到 UTF-8 边界崩 —— 改成 ASCII 字段名的 struct + `{% for k,v in obj %}` 迭代绕过。

**自写 cron loop,不用 tokio-cron-scheduler 库**:`cron` crate 解析 5-field 表达式(prepend "0 " 转 6-field)+ tokio `time::interval` 60s tick。自己写的原因是 scheduler 要访问 AppState 里的 registry(recovery/change/alert)做报告采集,第三方库的 task 抽象反而绕。而且要精确控制 **no-catch-up 语义**:

```text
next_fire <= now 且 now-next <= grace(300s) → Fire
> grace → MissedAdvance(只推进 last_run_at,不补跑)
```

对齐 reference 的 `misfire_grace_time=300` + `coalesce`。关机漏发 >5min 不补跑,<5min 重开补跑 1 次 —— 桌面工具的合理语义。

3 个模板(application_health / cluster_overview / incident_report)全可生成、订阅、邮件发送。SMTP 凭据走 env(对齐 reference,非 keychain),空配置回退 InMemory sender 开箱即用。

---

## 7. Phase 5:6 巡检视图 + subgraph 原语 + 一个词表 bug

Phase 5 是收尾段:把 reference 的 6 个巡检视图(topology / access-link / node-impact / config-impact / image-risk / alert-aggregation)迁过来。

### 7.1 核心洞察:5 个视图共用一个 BFS 原语

读到第 3 个视图时发现:除了 alert-aggregation(起点是告警不是拓扑节点),剩下 5 个视图(reference view2-5 + topology)**共用同一个图遍历原语** —— 从起点 BFS、depth 限深、edge_type 白名单过滤、有向(forward/reverse/both)、返 induced subgraph。

于是只写了一个 ~35 行的 `subgraph` 函数:

```rust
pub fn subgraph(
    topo: &Topology,
    start: &str,
    max_depth: usize,
    allowed: &[&str],      // edge_type 白名单
    dir: TraversalDir,     // Forward / Reverse / Both
) -> Topology              // induced subgraph(节点+边)
```

4 个视图只是不同的配置:

| 视图 | 起点 | 方向 | edge 白名单 |
|---|---|---|---|
| node-impact | Node | Reverse | SCHEDULED_ON, CONTAINS, … |
| config-impact | Secret/ConfigMap | Reverse | USES, ROUTES_TO, … |
| access-link | Application | Both | CONTAINS, BELONGS_TO, … |
| image-risk | ContainerImage | Reverse | USES_IMAGE, RUNS, … |

command 层薄包装,前端复用已有的 `<TopologyView>`。**5 个视图的代码量 ≈ 1 个原语 + 4 张配置表 + 4 个薄 page。**

### 7.2 一个 bug,和它暴露的债

Node Impact 视图第一次真集群验证时,用户报告「看不到节点」。

根因:前端选择器查的是 `KubernetesNode`(从 reference Cypher 的 Neo4j label 抄来的),但 Rust connector 产的 `resource_type` 是 `Node`。**词表漂移** —— 合成测试用的都是手写的类型名,跟 connector 真实产出对不上,所以测试全绿也抓不到。

修一行(`["KubernetesNode"]` → `["Node"]`)就好,但它暴露了一个结构性问题:**`resource_type` / `edge_type` 全程是 stringly-typed 散落字符串,没有中央注册表**。connector 产出、视图白名单、前端 `SHAPE_BY_TYPE`、recovery/changes 白名单,各写各的字面量,任一处改名都没人提醒。

事后做了三层防回归:

1. **realistic-fixture 集成测试** —— 用 connector 真实产出的类型名(Node / Pod / ContainerImage / USES_IMAGE …)跑 4 视图,锁住类型契约
2. **memory** 记录 reference Neo4j label ≠ Rust resource_type 这个坑
3. **项目级 skill**(`port-reference-testing`)把 canonical 词表写进移植约定

但这只是补丁。**根治方案是建中央 `resource_type` 注册表**(一个 enum / const 模块,所有消费方引用同一源)—— 这是 Phase 6 技术债的头号项。

### 7.3 image 富化:让 image-risk 真集群非空

Phase 5.2 给 k8s mapper 加 ContainerImage 节点 + USES_IMAGE 边,让 image-risk 不再是空图。验证时用新写的 headless 工具 `inspect_views`(镜像 desktop 命令的 exact code path:`materialized_topology` → `subgraph` → `topology_to_graph`)在真集群 SQLite 上跑一遍:

```text
topology: 163 nodes / 328 edges
node-impact  : start=`node:vm1`            -> 2 nodes / 1 edges
config-impact: start=`cm:kube-root-ca.crt`  -> 3 nodes / 2 edges
access-link  : start=`app:otel-demo`        -> 141 nodes / 300 edges
image-risk   : start=`image:flagd:v0.10.1`  -> 6 nodes / 6 edges   ← Phase 5.2 富化确认
alert-agg    : empty AlertRegistry          -> 0 nodes              ← 无 live 源(已知 gap)
```

6/6 视图功能验证通过(5 个有真数据,alert-aggregation 等接入 live 告警源)。

---

## 8. 里程碑:feature parity 达成

```text
Phase 1  纵切片 + 拓扑视图 + Blog Part 1          ✅
Phase 2  持久化 + Identity v0 + 真集群 + health merge  ✅
Phase 3  recovery + changes + reports + 边富化 + real handler  ✅
Phase 4  3 报告模板 + 调度 + SMTP + 持久化        ✅
Phase 5  6 巡检视图全迁 + image 富化              ✅
```

`reference/` 作为 read-only oracle 的对照职责基本完成。新栈全绿 gate:**282 Rust 测试 + clippy `-D warnings` + 前端 tsc/vitest/build**,真集群 163 节点 / 328 边稳定 sync。

这并不意味着 reference 可以删 —— 它仍是冷备份和行为规约。但**「把 reference 行为迁到新栈」这个阶段性目标,完成了**。

---

## 9. 三条值得记住的经验

**1. v0/v1 分层是副业项目的节奏控制器。** Identity Resolver、health_merge、watch、handler 全部先做 v0,把完整版显式 defer 到文档写明的地方。这让 Phase 2-5 能连续推进,不被最难的合并算法/真 streaming/真审批流水线卡住。代价是技术债累积 —— 但债是显式记账的(都在 CLAUDE.md 偏差栏),不是偷偷欠下的。

**2. I/O-free 纯领域逻辑 + orchestration 层做 I/O,是可测性的根基。** Recovery 的 handler/verifier/rollback、Changes 的 propagation/frequency、Views 的 subgraph 全设计成吃 `&Topology` / `&Registry` 的纯函数。这让它们能逐字移植 reference 的 contract test,行为偏差在 commit msg 里逐条记账。mutable twin 让 mock 和 real handler 共用同一管线。

**3. stringly-typed 词表是最大的隐性债。** Node Impact bug 只是一个症状。`resource_type` / `edge_type` 散落字面量,合成测试抓不到漂移,真集群验证才发现。教训:**跨模块共享的枚举集合,必须有中央注册表**,哪怕只是个 const 模块。

---

## 10. 下一步:技术债,还是 feature gap?

feature parity 之后是一个真正的方向选择。两条路:

- **A. 技术债(架构可演进性)**:Identity Resolver v1(correlation-key 合并 + field-ownership 完整表,doc/11 §4-5)+ resource_type 中央注册表(根治词表漂移)+ Parquet 归档 + 顶层 Makefile。
- **B. feature gap**:fault injection(Rust 无,incident_report / alert-aggregation 缺锚点)+ 真 k8s watch streaming(WIT stream + WASIp3)+ PRD-004 剩余 connector(jaeger/flagd/k8s_events)。

选了 **A → 然后 Identity v1**。理由:刚到 parity,先冻结一个可演示里程碑,再啃最值钱的架构债。Identity v1 会让后续所有异构源(Cloud API / Trace)的去重和合并受益 —— 它是 PRD-005 那套「N connector → Fact → Identity Resolver → Canonical Graph」愿景的地基。fault injection 虽然缺口大,但它是自成一体的较大子项目,适合作为冻结后的独立 Phase 启动。

Phase 1 的关键词是「边界清楚」。Phase 2-5 的关键词是「**用最简 v0 把骨架立起来,让下游不阻塞,把难设计点显式 defer**」。这套节奏让一个副业项目能在不卡死的前提下,从「命令行能跑」一路走到「真集群 6 视图全绿」。

下一篇,大概是 Identity v1 落地后的「合并算法 + 冲突仲裁」复盘 —— 如果那时候真的接了第二个异构数据源的话。
