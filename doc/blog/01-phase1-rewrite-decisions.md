# 从 Python MVP 到 Rust + WASM + Tauri:一个 SRE 巡检图谱的 Phase 1 重写复盘

> 日期:2026-06-29  
> 项目:SRE Inspection Graph / 云原生巡检图谱平台  
> 阶段:Phase 1(A → G + 最小拓扑视图)  
> 关键词:Supervised Rewrite,Tauri,WebAssembly Component Model,WASI p2,WIT,Capability,Arrow

## 0. 背景:一个已经完成的 Python MVP,为什么还要重写?

这个项目最早是一个 Python + FastAPI + Neo4j + React 的 MVP。它不是半成品:

- PRD-001 Recovery Action Engine:8 个恢复动作、dry-run、审批、回滚、真实 K8s/MySQL/Redis handler、跨集群编排、自动验证、动作链
- PRD-002 Change Event:ConfigMap/Secret/Deployment/Image 变更追踪、传播 BFS、Neo4j dual-write、K8s watcher、webhook、YAML diff、频率告警
- PRD-003 Self-Inspection Report:3 套报告模板、Jinja2 Markdown、APScheduler 订阅、SMTP
- PRD-004 OTel Demo Connectors:K8s / Prometheus / Jaeger / flagd / K8s events 5 个真实 connector
- 约 12.7k 行 Python + 472 个 backend 测试 + 71 个 frontend 测试

也正因为 MVP 已经完整,重写不是为了"补功能",而是为了换技术地基:

1. **部署形态**:从 Web 服务转成桌面优先,像 k9s / Lens 一样本地运行,数据不出本机
2. **插件模型**:connector / rule / handler 需要更强隔离和能力注入,Python import 插件边界太软
3. **数据路径**:Neo4j 很适合探索期,但桌面单机默认更适合 Arrow + SQLite + Parquet 的轻量组合
4. **长期演进**:WASM Component Model + WIT 提供稳定 ABI,比 Python 内部对象边界更适合长期插件生态

于是当前策略是:**Python 旧栈冻结为 `reference/` read-only oracle,Rust 新栈做 supervised rewrite**。旧栈不删、不改 feature,而是成为行为规约和 contract test 来源。

---

## 1. 第一个关键决策:Supervised Rewrite,不是 Strangler Fig

常见建议是 Strangler Fig:新旧系统并行,一块块把流量切过去。

这个项目没有采用它。

原因很简单:这是一个副业 / 自用优先项目,不是企业核心系统,没有"一秒不能停"的流量迁移压力。Strangler Fig 带来的成本反而很真实:

- 要维护 Python ↔ Rust 双写 / 双读 / 桥接层
- 要处理 Neo4j 与新存储之间的一致性
- 要维持两套 connector 生命周期
- 要设计跨语言调试和错误归因
- 每个 PR 都要判断"这是旧栈修,还是新栈修,还是桥接修"

这些成本对于一个没有生产 SLA 的项目并不划算。

所以策略改成 **Supervised Rewrite**:

- `reference/` 是行为 oracle,不再接受 feature 改动
- Rust 新栈只复刻明确需要的行为
- 每个 PRD 复刻时读 reference 源码和测试,而不是只读 PRD 文档
- 行为偏差允许存在,但必须在 commit message 里明示

这让 Phase 1 可以专注搭地基,不用同时伺候旧系统在线迁移。

---

## 2. 第二个关键决策:Tauri 桌面优先,不是 SaaS Web

SRE 巡检工具天然会碰到敏感数据:

- 集群 topology
- workload 名称
- 变更事件
- trace / metric / alert
- 恢复动作和审批记录

如果默认做 SaaS Web,一开始就要面对账号体系、租户隔离、审计、远程存储、网络连通性、数据出域等问题。这些对当前阶段都是干扰。

所以新栈默认是 **Tauri 2.x 桌面应用**:

```text
React UI(webview)
   ↓ Tauri command(JSON IPC)
Rust engine(in-process)
   ↓ Wasmtime Component host
WASM connectors
```

这条路线有几个好处:

1. **数据本地化**:默认数据不离开开发者机器或 SRE 工作站
2. **部署轻**:不像 Electron 那样打包整套 Chromium,Tauri 复用系统 webview
3. **Rust engine 原生嵌入**:UI 和 engine 同进程,用 Tauri command 即可,不需要本地 HTTP server
4. **符合使用心智**:更像 k9s / Lens / Docker Desktop,而不是一个远程 SaaS 控制台

同时也明确了一个反模式:

> 不在 Tauri 里再起 HTTP server。UI ↔ engine 是进程内 IPC,不是 REST。

REST / Flight / gRPC 只在将来真的需要 headless engine-cli + 远程 UI 时再讨论。

---

## 3. 三层数据契约:WIT / Tauri commands / Arrow

Phase 1 最重要的不是某个页面,而是把边界画清楚。当前有三层契约:

| 层 | 协议 | 边界 | 职责 |
|---|---|---|---|
| A | WIT(Component Model) | WASM connector ↔ Rust host | 插件 ABI、capability imports、Fact wire format |
| B | Tauri commands(JSON IPC) | React webview ↔ Rust process | UI 操作入口,如 `list_connectors` / `sync_all_now` |
| C | Arrow RecordBatch + SQLite/Parquet | engine 内部 / 存储 | canonical Fact、批处理、持久化 |

这三层解决的是不同问题,不能混用:

- WIT 是插件边界,不应该把 UI DTO 塞进去
- Tauri command 是 UI IPC,不应该承担 engine 内部数据模型演进
- Arrow 是分析/存储格式,不应该暴露成 WASM guest 必须理解的东西

Phase 1 已落地的 canonical `Fact` 是 7 列:

```text
id
kind
source
resource_id
resource_type
timestamp
attributes_json
```

WIT 里的 `connector.fact` 是 wire format;host 收到后立刻转成 `engine_core::Fact`。后续 storage / query / Arrow 全只认 canonical Fact,避免下游耦合 wit-bindgen 生成类型。

---

## 4. WASM connector:从 hello-world 到 k8s-mini

Phase 1 的 connector 演进分三步。

### 4.1 hello-world:证明 WIT → wasm32-wasip2 → wasmtime 端到端能跑

第一条 connector 只做一件事:返回 1 条 Fact。

这听起来很小,但它验证了整条链:

```text
specs/wit/connector.wit
   ↓ wit-bindgen guest bindings
modules/connectors/hello-world
   ↓ cargo component / wasm32-wasip2
hello_world.wasm Component
   ↓ wasmtime Component host
engine-wasm::WasmConnector::sync
   ↓ HostFact → engine_core::Fact
FactBatch → Arrow RecordBatch
```

这个阶段最大的收益是确认工具链版本、WASI ABI、wasmtime bindgen、guest bindings 之间没有错位。

### 4.2 k8s-mini:第二条 connector,证明多 connector 编排

第二条 connector `k8s-mini` 一开始并不连真实 K8s API,而是从 config JSON 里读 namespace 列表,每个 namespace 产一条 topology Fact。

这样能验证更关键的 host 编排:

- manifest 里加载多个 module
- `WasmRuntime` 持 N 个 `ConnectorEntry`
- 每个 connector 有自己的 wasmtime Store
- `sync_all` 聚合多 connector 的 Fact
- 统一转成 `FactBatch` / Arrow RecordBatch

这一步完成后,engine-cli 可以跑:

```bash
engine-cli tick
```

并看到 hello-world + k8s-mini 的聚合输出。

### 4.3 with_topology:为桌面最小拓扑视图准备分层 mock Fact

Phase 1 末尾又给 k8s-mini 加了 opt-in 配置:

```json
{
  "cluster": "demo",
  "namespaces": ["default", "app"],
  "with_topology": true
}
```

默认 `with_topology=false`,保持已有测试向后兼容;true 时额外产生分层 mock 拓扑:

```text
Cluster
├── Node(control-plane)
├── Node(worker)
├── Namespace(default)
│   ├── Pod(app-0-0)
│   ├── Pod(app-0-1)
│   └── Service(web)
└── Namespace(app)
    ├── Pod(app-1-0)
    ├── Pod(app-1-1)
    └── Service(web)
```

父子关系通过 `attributes_json.parent_resource_id` 表达。这个字段不是前端临时 hack,而是 Phase 2 真 K8s connector 也会继承的约定。

---

## 5. Capability 设计:deny by default + call-time 拒绝

WIT world 里声明了 host imports:

- `logging`
- `clock`
- `http-client`

问题是:guest 能不能直接调 host 的 HTTP client?

Phase 1 G 的答案是:**默认不能,manifest 显式声明后才行**。

manifest 里每个模块都有:

```toml
capabilities = ["logging", "clock", "http-client"]
```

host 端 `State` 里保存:

```rust
allowed_capabilities: HashSet<String>
http_client: reqwest::Client
```

`http_get` 每次调用时检查:

```rust
if !allowed_capabilities.contains("http-client") {
    return Err(HostHttpError::Unauthorized(...));
}
```

这叫 **call-time 拒绝**。

另一个方案是 link-time 拒绝:每个 connector 单独构造 Linker,没有声明 capability 的 connector 根本不链接 `http-client` import。这个方案更硬,但 Phase 1 代价不小:

- 需要 per-WasmConnector Linker
- world import 组合复杂
- guest 即使不调用也可能因为 import 声明而实例化失败
- 后续加 URL allow-list 仍然需要 call-time 检查

所以 Phase 1 选择 call-time gate。它不完美,但简单、可测、可演进。Phase 3 可以继续加:

```toml
[modules.network]
allowed_hosts = ["https://prometheus.local", "https://jaeger.local"]
```

然后在同一个 `http_get` 里多做 URL host 校验。

---

## 6. host 类型与 WIT binding 解耦

Phase 1 G 还有一个刻意设计:`http_host.rs` 不直接使用 wit-bindgen 生成的 `HttpResponse` / `HttpError`。

它定义自己的 host plain types:

```rust
pub struct HostHttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

pub enum HostHttpError {
    Unauthorized(String),
    NotFound,
    Network(String),
    Timeout,
}
```

真正发请求的函数是纯 host 函数:

```rust
pub async fn http_get(
    client: &reqwest::Client,
    allowed_capabilities: &HashSet<String>,
    url: &str,
    headers: &[(String, String)],
) -> Result<HostHttpResponse, HostHttpError>
```

`runtime.rs::HttpClientHost::get` 只做薄适配:

```rust
let resp = http_get(...).await.map_err(map_host_err_to_wit)?;
Ok(HttpResponse { status: resp.status, body: resp.body })
```

这个模式有两个收益:

1. **测试简单**:`http_get` 可以脱离 wasmtime Store、WIT binding、Component 实例化单独测试
2. **WIT 可演进**:未来 WIT 0.3 / WASI p3 加 async-native 或 stream variant 时,host 逻辑不需要跟着大改,只改适配层

`HostFact` → `engine_core::Fact` 也是同样思想:binding 类型只是边界上的 wire type,不是 engine 内部的真模型。

---

## 7. 状态码策略:不要过早发明错误模型

`http-client` WIT error 当前只有:

```wit
variant error {
  unauthorized,
  not-found,
  network(string),
  timeout,
}
```

Phase 1 G 的状态码映射是:

| 情况 | 映射 |
|---|---|
| 缺 capability | `Unauthorized` |
| HTTP 401/403 | `Unauthorized` |
| HTTP 404 | `NotFound` |
| reqwest timeout | `Timeout` |
| DNS/TCP/TLS 等网络错误 | `Network(String)` |
| 其它状态码(包括 5xx) | 返回 `HostHttpResponse { status, body }` 给 guest 自决 |

这里故意没有把 5xx 变成 error。原因是 WIT 没定义 `server-error`,而很多 connector 可能希望读取 500 body 做诊断或重试策略。过早把它吞成 host error 反而剥夺了 guest 的上下文。

---

## 8. Tauri ↔ engine-wasm 桥接:让用户真的点按钮

Phase 1 F 把 engine 接到桌面 UI:

- `list_connectors`:同步读当前加载的 connectors
- `sync_all_now`:触发一次 `WasmRuntime::sync_all`,返回 facts + per-connector 状态

Tauri 启动时:

```text
block_on(WasmRuntime::from_manifest)
   ↓
.manage(runtime)
   ↓
invoke_handler(list_connectors, sync_all_now, get_app_version)
```

如果 wasm 没 build 或 manifest 加载失败,不会让 UI 起不来,而是 fallback 到 empty runtime,页面提示:

```bash
cd modules && cargo wasi-build
```

这对桌面工具很重要:启动失败不能只给一坨 Rust backtrace,要让用户知道下一步该做什么。

Phase 1 Step 2 又把 `sync_all_now` 的结果接到 Cytoscape:

- 点击 Sync all now
- App 传 `with_topology=true` config
- k8s-mini 产生 11 条分层 Fact
- `TopologyView` 解 `attributes_json.parent_resource_id`
- Cytoscape breadthfirst layout 渲染 Cluster / Node / Namespace / Pod / Service

这一步标志着 Phase 1 从"命令行能跑"变成"桌面能看见图"。

---

## 9. 为什么 Phase 1 没有直接做真 K8s connector?

因为 Phase 1 的目标不是"接更多数据",而是验证架构闭环:

```text
WIT 契约 → WASM module → wasmtime host → canonical Fact → Arrow → Tauri IPC → React → Cytoscape
```

真 K8s connector 会引入一堆额外问题:

- kubeconfig 怎么读?file-system capability 怎么设计?
- kube-apiserver URL / token / TLS 怎么授权?
- watch/list 是一次 sync 还是 stream?
- connector 是否需要长期持连接?
- WASI p2 下 async/network 怎么抽象?

这些问题都重要,但它们属于 Phase 2。Phase 1 先用 k8s-mini mock topology 把图画出来,避免把能力边界、UI、数据模型和 K8s SDK 问题混在一个 PR 里。

---

## 10. Phase 1 到目前为止完成了什么?

按 commit 增量:

| 增量 | 内容 | 状态 |
|---|---|---|
| A-B | WIT + toolchain:host-capabilities world + cargo aliases + cargo-component metadata | ✅ |
| 第一刀 | host wasmtime 真加载 hello_world.wasm 端到端跑通 | ✅ |
| C+D | WasmRuntime 多 connector 编排 + canonical Fact + Arrow RecordBatch + engine-cli tick | ✅ |
| E | k8s-mini 第二条 WASM connector + multi-connector 编排验证 | ✅ |
| F | Tauri ↔ engine-wasm 桥接(`list_connectors` + `sync_all_now`) | ✅ |
| G | http-client capability host 实装(reqwest GET + capability allow-list) | ✅ |
| Step 2 | k8s-mini `with_topology` + Cytoscape 最小拓扑视图 | ✅ |

当前可以做到:

```bash
cd modules && cargo wasi-build
cd desktop && npm run tauri dev
```

打开桌面应用后点击 **Sync all now**,能看到:

- connector 列表:hello-world + k8s-mini
- per-connector 状态
- Fact 表
- Cytoscape 拓扑图

实机验证里,真实 Tauri GUI 触发后日志显示:

```text
wasm runtime ready connectors=2 load_errors=0 names=["hello-world", "k8s-mini"]
hello-world sync invoked
k8s-mini sync: cluster=demo namespaces=2 with_topology=true
```

并且窗口截图中出现 Cytoscape 的绿色拓扑节点。

---

## 11. Phase 2 下一步:实数据 + 持久化

Phase 1 结束后,Phase 2 的主题是:**实数据 + 持久化**。

计划拆成 7 块:

1. **FactBus**:多 connector 生产者 → Identity Resolver 单消费者,MPSC + backpressure
2. **Identity Resolver**:DataFusion / SQLite UPSERT 组合,实现 resource dedup / merge / link
3. **K8s connector WASM 化**:替代 k8s-mini,真连集群,设计 kube capability
4. **Prometheus connector WASM 化**:直接消费 Phase 1 G 的 http-client capability
5. **Jaeger / flagd / k8s-events connector WASM 化**
6. **SQLite + Parquet storage**:SQLite 存元数据 / 索引,Parquet 存 Fact 批归档
7. **Tauri 视图迁移**:topology / access-link / node-impact 三个视图先迁

这里最难的不是写 connector,而是两个设计点:

- **capability 粒度**:K8s 是一个粗 `cluster-access`,还是拆成 `kubeconfig-read` / `pod-list` / `pod-watch`?
- **Identity Resolver 表达力**:Neo4j `MERGE` 的语义如何在 DataFusion + SQLite 上等价实现?

Phase 1 的价值,就是让这些 Phase 2 问题有了一个稳定的承载层。

---

## 12. 小结

Phase 1 的关键词不是"功能多",而是"边界清楚":

- Python 旧栈冻结为 reference oracle
- Rust engine 成为新核心
- WASM connector 通过 WIT 定义 ABI
- Capability deny by default
- host 逻辑与 WIT binding 解耦
- canonical Fact 统一下游模型
- Tauri command 取代本地 REST
- Cytoscape 最小视图证明桌面闭环可见

这套地基不酷炫,但它让后续每个 connector / view / storage backend 都有地方放。

从这里开始,项目才真正进入"把 reference 行为迁到新栈"的阶段。
