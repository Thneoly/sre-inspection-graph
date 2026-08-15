# 一个人从 Rust 内核做到 React 前端:我做了一个云原生 SRE 巡检图谱桌面工具

> 这是系列的入口篇。后面六篇会分别拆开讲 WASM 插件沙箱、数据契约、多源拓扑合并、恢复动作引擎、变更追踪、巡检视图 —— 那些是这个项目里最有意思的部分,但你得先看见全貌。

## 从一个真实的痛点说起

你定位过生产故障吗?那大概率经历过这样的流程:

```
告警响了 → 打开 Grafana 看哪个图表飘了
        → 打开 Jaeger 拉调用链,看卡在哪一跳
        → 想起来昨天有个变更 → 翻 Argo / Git 提交记录
        → 指标/日志/链路各说各话 → 回到 kubectl describe 手敲命令
```

五六个系统来回跳,**因为它们之间没有一张共享的资源图谱**。于是这几个问题全靠人脑拼接:

- 这个挂掉的 Pod,影响了哪些业务?
- 这次变更和这个告警,有没有因果关系?
- 我要重启这个 Deployment,会炸到谁?

我做过很多次这样的拼接之后决定自己动手:做一个**把拓扑、调用链、变更、代码、指标汇成一张图**的工具,并且让 SRE 能直接在图上行动 —— dry-run 预演、审批、执行恢复、自动验证、一键回滚。

## 它长什么样

一个桌面应用。打开后是当前集群的资源图谱(节点 = 资源,边 = 关系),配 6 个图遍历视图:

| 视图 | 回答的问题 |
|---|---|
| 应用拓扑 | 全局长什么样 |
| 节点影响 | 这台 Node 挂了,爆炸半径多大 |
| 配置影响 | 这个 Secret 变了,谁受影响 |
| 访问链路 | 流量从入口怎么到这个 Pod |
| 镜像风险 | 这个镜像被哪些服务在用 |
| 告警聚合 | firing 的告警各自挂在哪 |

点击任何节点,能直接发起恢复动作 —— 8 种(scale / restart_pod / rollback_deployment / refresh_secret / drain_node …),带完整的生命周期:

```
pending → dry_run_ok → awaiting_approval → executing → succeeded → (verify) → rolled_back
                ↑ 干跑爆炸半径          ↑ 风险门        ↑ 出问题自动反向回滚
```

所有数据来自真实的 K8s 集群和 Jaeger —— 不掺 mock(Prometheus connector 也在,但我这套测试集群里的 Prometheus 常驻 OOM,指标维度暂空,正好当「数据源缺席时系统不崩」的演练)。

## 为什么是桌面工具,而不是 Web 服务?

这是第一个重大选型。做 SaaS 的话,意味着多租户、认证、数据上云、入站 webhook —— 对一个个人项目,这些复杂度不抵。而对照 **k9s 和 Lens** 的成功:面向单个使用者的运维工具,「数据不出本机」不是妥协,是特性 —— SRE 的集群拓扑和凭据本来就不该交给一个第三方 SaaS。

所以技术形态定为:**Tauri 2.x 桌面壳 + Rust 引擎进程内嵌**。UI 和后端之间没有 REST,没有 HTTP server,只有进程内 IPC:

```
┌────────────────────────────────────────────────┐
│ React 18 + AntD + Cytoscape(前端)             │
├──────────────── Tauri IPC(进程内)──────────────┤
│ Tauri 后端(Rust,薄命令层)                    │
├────────────────────────────────────────────────┤
│ engine 内核(Rust,8 个业务 crate)             │
│   identity / recovery / changes / reports ...  │
├────────────────────────────────────────────────┤
│ wasmtime host + WASM connector(沙箱插件)      │
├────────────────────────────────────────────────┤
│ K8s API · Jaeger · Prometheus · 本地代码仓      │
└────────────────────────────────────────────────┘
   存储:SQLite(最新拓扑)+ Parquet(归档)+ Arrow(批契约)
```

代价也诚实记录:砍掉了 webhook(需要入站连接),变更事件改用轮询 diff + 手动录入。**为一个一致性主动放弃一个能力,这是权衡不是疏漏。**

## 一个人打穿全栈是什么体验

技术栈从下到上:

- **Rust engine**(8 个业务 crate):`engine-core`(canonical Fact + Arrow Schema)· `engine-identity`(多源拓扑合并)· `engine-recovery`(动作引擎)· `engine-changes`(变更追踪)· `engine-reports`(报告)· `engine-wasm`(wasmtime host)· `engine-storage`(SQLite + Parquet)· `engine-cli`(headless 验证入口)
- **WASM connector**(6 个,`wasm32-wasip2`):k8s / prometheus / jaeger / k8s-events / flagd / code-repo
- **Tauri 后端**:`#[tauri::command]` 薄命令层 + AppState
- **前端**:React 18 + TypeScript + AntD 6 + Cytoscape + TanStack Query

规模:~32k 行手写代码(Rust 28.7k + TS 3.3k),403 个 Rust 测试。关键路径都量过(仓库带两个可复跑的 bench example):connector 实例化 **6–24ms**、全图 resolve(含多源合并)**0.77ms**、稳态增量判定 **0.05ms** —— 各篇有完整数字。

最深的感受是**契约的价值**。单人项目最大的风险不是写不完,是改不动 —— 三个月前的自己就是最陌生的协作者。所以我从第一天就定了三层数据契约(WIT / Tauri IPC / Arrow),后面每加一个 connector、每加一个视图,内核几乎不用动。这个展开是[下一篇](./03-canonical-fact-data-contract.md)的主题。

另外几个值得说的实践,各自成篇:

- **connector 是不可信插件**,怎么让它们安全地访问集群?→ [WASM 沙箱 + capability 模型](./02-wasm-capability-sandbox.md)
- **K8s API 和代码仓对同一个镜像用不同 ID**,怎么合并成同一个节点?→ [Identity Resolution](./04-identity-resolution.md)
- **恢复动作怎么让人敢按下去**?dry-run / 审批门 / 自动回滚 / mutable twin → [恢复动作引擎](./05-recovery-action-engine.md)
- **「最近改了什么」怎么回答**?传播 BFS / YAML diff 去噪 / 自动录入 → [变更追踪](./06-change-tracking-timeline.md)
- **六个巡检视图**其实是一个图遍历原语 —— 顺带一个词表漂移的教训 → [subgraph 与视图](./07-subgraph-views.md)

## 一条完整的链路(真实数据)

拿真实集群验证过一次完整的 SRE 价值链:本地 kubeadm 集群跑 OpenTelemetry Demo(~20 个微服务,Go/Java/Python/Node 混布):

1. **sync** → connector 拉 K8s API,产出 169 节点 / 350 边的真实拓扑
2. **trace** → jaeger connector 从跨服务 span 引用聚合出 CALLS 调用边
3. **变更** → 对一个服务做 rollout,后台轮询 diff 检测到 `current_revision 1 → 2`,自动录一条变更事件,YAML diff 只显示真正变化的信号字段
4. **恢复** → 图上点节点 → dry-run 看爆炸半径 → 执行 → verifier 验证 → 不通过自动反向回滚
5. **报告** → 按模板生成 Markdown 巡检报告,cron 订阅邮件发送

每一步都是真数据,没有一步是写死的演示。

## 复现

```bash
git clone https://github.com/Thneoly/sre-inspection-graph && cd sre-inspection-graph
make modules-build                 # 构建 6 个 WASM connector

# 实测沙箱成本(本系列数字的来源之一,可直接复跑)
cargo run --manifest-path engine/Cargo.toml --release \
  -p engine-wasm --example bench_load -- modules/target/wasm32-wasip2/release 20

# 连真实集群(本地 kubectl proxy --port=8001 后,单次 headless 同步)
cargo run --manifest-path engine/Cargo.toml --release -p engine-cli -- tick
```

桌面端与完整 quickstart 见仓库 README。

## 写在最后

这个项目教给我的,比「学会 Rust」多得多:

- **架构是关于边界的**:WIT 边界、IPC 边界、存储边界 —— 每层契约清晰,扩展才便宜。
- **可测性是设计出来的**:领域逻辑全部写成吃 `&Topology` 的纯函数,不碰 I/O,403 个测试才写得动。
- **桌面工具被低估了**:不是所有东西都要长成 SaaS。

仓库(含完整架构文档和 case study):**https://github.com/Thneoly/sre-inspection-graph**

> 系列下一篇:[用 WebAssembly 给不可信插件上镣铐:capability 沙箱模型实践](./02-wasm-capability-sandbox.md)
