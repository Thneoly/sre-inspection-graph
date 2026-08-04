# Case Study — SRE 巡检图谱平台

> 面试/深读用工程叙事。README 是一分钟版,这份是「为什么这么设计 + 怎么权衡 + 结果」的完整版。定位:**全栈 / 平台通才**。

## 一句话

一个人从 Rust 内核到 WebAssembly 沙箱、Tauri 桌面端、React UI **全栈设计与实现**的云原生「感知 → 定位 → 恢复」控制面:把分散的 K8s 拓扑、调用链、变更、代码、指标汇成一张资源图谱,让 SRE 在一个桌面端上完成故障定位与恢复编排。

**规模**:~32k 行手写代码(Rust ~28.7k + TS ~3.3k),517 个 Rust 测试 + 21 前端测试,8 个 engine crate + 6 个 WASM connector。

---

## 问题:为什么需要这个东西

真实 SRE 定位一次故障,要在五六个割裂的系统之间来回跳:拓扑看 Grafana、调用链看 Jaeger、变更看 Argo/Git、指标看 Prometheus、恢复手敲 kubectl。**它们之间没有一张共享的资源图谱**,于是这三个问题全靠人脑拼接:

- 「这个挂掉的 Pod 影响了哪些业务?」
- 「这次变更和这个告警有没有关系?」
- 「恢复一个 Deployment 会炸到谁?」

本项目的核心论点:**给这些割裂的信号一个共享的 canonical 资源身份**,任何一处的信号(拓扑/调用/变更/代码/指标)都能在图上找到同一位置,并据此行动(dry-run → 审批 → 恢复 → 自动验证)。

---

## 我做了什么(全栈纵切,一个人)

```
React 18 + AntD + Cytoscape          ← 前端 6 视图 + 恢复/变更/报告页
        ↕ Tauri 进程内 IPC(无 HTTP server)
Tauri 2.x Rust 后端                  ← 薄命令层 + AppState + 托管 kubectl proxy + 调度/SMTP
        ↕
Rust engine(8 业务 crate)           ← identity resolution / recovery / changes / reports / views
        ↕ facts
wasmtime host + WASM connector       ← 多 connector 编排 + capability 注入(deny-by-default)
        ↕ WIT 契约
K8s API / Jaeger / Prometheus / 代码仓
SQLite(latest 拓扑)+ Parquet(归档)+ Arrow(批契约)
```

业务面覆盖 4 个 PRD:恢复动作引擎(PRD-001,8 action + dry-run/审批/回滚/自动验证/链)、变更追踪(PRD-002,传播 BFS + yaml diff + 频率告警 + 告警关联)、自检报告(PRD-003,3 模板 + cron 订阅 + SMTP)、数据源 connector(PRD-004,5 个)+ 代码仓源(PRD-006)+ 6 个图遍历巡检视图。

---

## 四个架构决策(STAR)

> 这四个是面试会被追问的判断点。每个都是「有多个选项 → 选了一个 → 因为 → 结果」。

### 决策 1:canonical `Fact` 作为唯一数据契约

- **情境**:6 个 connector(K8s / Jaeger / Prometheus / code-repo / k8s-events / flagd)各自产出形状不同的数据,下游(storage / identity resolve / graph build / UI)都要消费。若让下游 per-数据源 适配,耦合数 = N(connector)× M(下游),爆炸。
- **选项**:A) 每下游适配每数据源;B) 引入一个 canonical 中间态,所有数据源压平成它,所有下游只认它。
- **决策**:B。所有 connector 产出统一 7 字段 canonical `Fact`(`id/kind/source/resource_id/resource_type/timestamp/attributes_json`);一个 `engine-core::fact_schema()` Arrow Schema 把契约焊死;下游(storage / identity / graph)只认 Fact。
- **为什么**:解耦。新增数据源只需写一个产 Fact 的 WASM connector,内核 + 下游零改 —— 这是整个平台可扩展的支点。
- **结果**:后续加 jaeger / code-repo / k8s-events / flagd 等 5 个 connector,**内核与全部下游零改**,只新增 connector 本身。Arrow Schema 同时是 SQLite 落库、Parquet 归档、批传输的单一 schema。

### 决策 2:Tauri 桌面优先,而不是 SaaS Web

- **情境**:SRE 巡检工具要碰生产集群凭据 + 读敏感拓扑。SaaS 形态意味着多租户、认证、数据上云、入站 webhook server。
- **选项**:A) SaaS(后端 REST + 前端);B) 桌面 app(对照 k9s / Lens)。
- **决策**:B,Tauri 2.x。数据不出本机;UI ↔ Rust 走进程内 IPC,**不起 HTTP server**。
- **为什么**:① SaaS 的多租户/认证/隔离复杂度对单作者项目不抵;② 数据不出本机对 SRE 工具是**特性**(对照 k9s / Lens 的定位);③ 进程内 IPC 比 REST 轻,无序列化往返开销。
- **代价 / 结果**:由此砍掉 webhook(需入站连接)→ 变更入口改 **poll-diff + 手动录入**。代价诚实记录:多人协作 / 远程访问留后续。这是个「为了一致性主动放弃一个能力」的权衡,不是疏漏。

### 决策 3:WebAssembly Component Model + deny-by-default capability

- **情境**:connector 是「会跑用户指定代码 + 访问生产集群凭据」的**不可信插件**。直接给 raw WASI preopens = 完整读写删权限,危险。
- **选项**:A) raw WASI(读写删);B) host 自定义 capability 注入 + deny-by-default。
- **决策**:B。wasmtime host 加载 wasm32-wasip2 guest;`http-client` / `fs-read` 由 host 注入并按 allow-list **逐次放行**;`fs-read` 第一天就强制 **path-root canonicalize 校验**(`canonicalize` + `starts_with`,防 `../../etc/passwd` 目录穿越 + 符号链接逃逸)。三层数据契约固化边界:WIT(WASM 边界)/ Tauri commands(UI 边界)/ Arrow+SQLite+Parquet(存储)。
- **为什么**:安全默认(deny-by-default)。capability 查表是 **call-time**(非 link-time)—— 简单,且后续加 URL / path allow-list 平滑。
- **结果**:6 connector 各自申明最小 capability;`fs-read` 的 path-root 校验有单测覆盖目录穿越 / 符号链接逃逸;新增 capability 只改 WIT + 给 host State 加一个 impl。

### 决策 4:Identity Resolver 延后到「有真数据冲突」才落地

- **情境**:多源拓扑合并(K8s API 与代码仓对同一资源用不同 ID 描述)是平台的核心难点。早期(Phase 6)本可以用合成数据演示合并。
- **选项**:A) 用合成数据早点 demo(有演示价值);B) 延后到有真实冲突源再做。
- **决策**:B,延后。合成冲突是**假的** —— 会让整套仲裁逻辑对着不存在的问题空转。直到后期 code-repo 给出**真实冲突源**(repo 的 `BUILDS` 边指向的镜像 `image-ref:<ref>` 与 K8s 部署镜像 `image:{c}:{ns}:{ref}`,同一镜像两端用不同 key),才落地 correlation-key 合并。
- **为什么**:**不为演示造合成问题**。这是项目里最显工程判断的一处 —— 知道什么时候 NOT to build。
- **结果**:落地后 = correlation-key BFS 聚簇(支持传递合并)+ source-priority winner(runtime 源 > 声明源)+ 边端点 remap + canonical attr 合并;**零 schema 改**(correlation_keys 走 attributes_json,WIT/Arrow/SQLite 全不动);真集群验证 repo→image→container→pod 联通。决定性(输入顺序无关,diff-stable)是 load-bearing 不变量,有单测守护。

---

## 工程亮点(除架构外)

- **I/O-free 纯领域核**:identity `resolve`/`diff`、recovery `cascade::dry_run`、changes `derive_propagation` 全是吃 `&Topology` / `&ChangeRegistry` 的纯函数,不碰 I/O → 单测覆盖,持久化是独立一层。这是「逻辑可测、I/O 边界薄」的纪律。
- **mutable twin 架构(recovery)**:handler mock 动作时 mutate `&mut Topology` 孪生的 attrs,verifier 读 mutated attrs 验 predicate,rollback 读 post-action 状态做正确反转 —— 动作生效/验证/回滚全在内存模型上自洽。
- **行为契约测试**:每个领域函数配 fixture-based 契约测试(合成拓扑 + 期望),517 Rust 测试 + 21 前端测试守住行为。
- **零跨进程 RPC 桌面架构**:webview ↔ Rust 进程内 IPC,无 HTTP server(刻意反模式规避)。

---

## 量化

| 维度 | 数 |
|---|---|
| 手写代码 | ~32k 行(Rust 28.7k + TS 3.3k) |
| Rust 测试 | 517(+ 前端 21) |
| engine crate | 8 业务 + 2 基建 |
| WASM connector | 6 数据源 + 2 handler |
| 巡检视图 | 6 个图遍历(BFS + edge 白名单 + induced subgraph) |
| recovery action | 8(dry-run/审批/回滚/自动验证/链) |
| 真集群验证 | 169 节点 / 350 边(otel-demo on kubeadm) |

---

## 诚实的边界

- **不是规模故事**:刻意桌面单机、数据不出本机(对照 k9s/Lens)。规模不是卖点,**架构深度与工程判断**才是。otel-demo 是「真实 polyglot 微服务拓扑」的验证手段,不是生产流量。
- **单作者**:没有团队 / 用户 / 线上事故故事。靠**深度 + 决策质量 + 纪律**打,不假装有规模。
- **延后项**(诚实):Unknown Dependency Queue 需指向有真实外部依赖的集群;real handler 仅 K8s(MySQL/Redis 留 mock);多人协作留后续。

---

## 深入探讨(面试可能追问)

**Q:为什么用 WASM 隔离 connector,不用容器/进程?**
connector 要在桌面进程内被 host 编排(N 个顺序 sync)。WASM 比 container 轻 —— 毫秒级加载、共享 host 凭据注入、无进程开销;Component Model + capability 给最小权限沙箱。容器隔离适合「不可信长跑服务」,不适合「进程内被编排的短任务插件」。

**Q:为什么 Rust 做 engine,不用 Go / Python?**
性能 + 类型安全 + `wasmtime`/`wasm32-wasip2` 生态原生支持;Rust 的 trait + 强类型 enum 适合固化三层数据契约(把 schema 错误挡在编译期)。Python 在重写前作过第一版,但桌面 + WASM + 进程内 IPC 的形态下,Rust 的零成本抽象 + 沙箱宿主能力更合身。

**Q:Identity Resolver 的 correlation-key 合并怎么保证决定性?**
winner = max source-priority,routine 平局 lex-min resource_id;attr 合并用 BTreeMap-backed serde Map → `to_string()` 产 canonical 有序 key 串。纯函数 + 顺序无关 → `diff` 按 attributes_json 字符串相等比节点,稳定。有「输入乱序也产出同节点集 + 字节一致 attrs」的单测。

**Q:最难的一个 bug?**
real handler 接入时:WASM handler 只返动作生效字段(`{desired_replicas}`),host 若整体替换 `attributes_json` 会**擦掉 connector 写入的字段**(`cluster`/`name`/...)→ verifier 全 fail 误触 auto-rollback。修法:host 读 target 现有 attrs,**overlay** WASM 返字段后返合并全量;再据 action 从合并 attrs 合成 verifier 期望字段。这是个「跨层(连接器字段 + handler 字段)合并语义」的隐性契约 bug。

**Q:桌面之外怎么扩?**
已有 headless `engine-cli`(tick 单次/循环);再扩:engine 作服务 + Arrow Flight 跨网(三层契约已留 Flight 口子)+ 多集群 per-kubeconfig + 真集群 watch(需 WIT stream + WASIp3)。架构没把自己锁死在桌面。

**Q:connector 是不可信插件,你怎么信它产出的事实?**
deny-by-default capability 限制了它**能做什么**(网络/fs 访问);产出的事实经 `resolve`(去重/派生边/悬空过滤)+ identity correlation 合并后才物化进图 —— 不直接信任 connector 的图谱断言,而是以其事实为输入由内核重算。
