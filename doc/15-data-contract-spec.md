# 15 — 数据契约规范:WIT + Tauri Commands + Arrow + REST 多层

## 0. 上下文

本文是数据契约决策的**详细规约**。所有 Rust 引擎实现、WASM 插件开发、Tauri 桌面前端、可选 headless CLI **必须遵守本文 schema**。


**核心决策**:**四层契约**,各司其职,**不使用 protobuf**。

```
              ┌─────────────────────────────────────┐
              │  Tauri Webview (React + TS)         │
              └─────────────┬───────────────────────┘
                            │ 层 B:Tauri Commands (IPC, JSON)
              ┌─────────────▼───────────────────────┐
              │  Tauri Backend / engine-core (Rust) │
              └──┬──────────┬───────────────────────┘
       层 A: WIT │          │ 层 D: Arrow (内部表示)
                 ▼          ▼
        ┌───────────┐ ┌──────────────────┐
        │ WASM 模块 │ │ SQLite + Parquet │
        └───────────┘ └──────────────────┘

                     [ headless 模式可选 ]
              ┌─────────────────────────────────────┐
              │  engine-cli                          │
              └─────────────┬───────────────────────┘
                            │ 层 C:REST + Arrow Flight
              ┌─────────────▼───────────────────────┐
              │  外部客户端 / SaaS Web shell         │
              └─────────────────────────────────────┘
```

| 层 | 协议 | 边界 | 频率/体量 | 桌面默认 | Headless 模式 |
|---|---|---|---|---|---|
| **A** | WIT (Component Model) | WASM guest ↔ Rust host | 小消息高频 | ✅ | ✅ |
| **B** | **Tauri Commands** (JSON IPC + specta TS gen) | Webview ↔ Rust 主进程 | 低-中频 req/resp | ✅ **首选** | ❌ N/A |
| **C** | REST + Arrow Flight | 外部 ↔ engine-cli | 低频 + 高吞吐流 | ❌ N/A | ✅ **首选** |
| **D** | Arrow RecordBatch (内存) + Parquet (归档) | Rust 内部 | 全部 | ✅ | ✅ |

## 1. 层 A:WIT 接口(WASM ↔ Host)

### 1.1 设计原则

- 使用 **WIT Component Model**(不是 wasm32-wasi-preview1)— 2024 起稳定,长期方向
- 每类 WASM 模块一个 WIT 包,版本号独立演进
- 接口尽量 **stateless** — host 持状态,WASM 仅做转换 / 推断
- 错误统一用 `result<T, string>`,人类可读

### 1.2 包结构

```
topology-engine/
├── wit/
│   ├── connector.wit       # 数据源 connector 接口
│   ├── rule.wit            # 巡检规则 / 业务规则接口
│   ├── handler.wit         # Recovery 自定义 handler 接口
│   └── types.wit           # 共享类型(fact, change-event 等)
```

### 1.3 共享类型(`wit/types.wit`)

```wit
package sre:topology@0.1.0;

interface types {
  /// Unix milliseconds since epoch
  type timestamp = u64;
  
  /// Confidence in [0.0, 1.0]
  type confidence = f32;

  /// A correlation key with prefix.
  /// Examples:
  ///   "ip:10.0.0.1"
  ///   "endpoint:https://api.stripe.com"
  ///   "arn:aws:rds:us-west-2:..."
  ///   "domain:api.stripe.com"
  ///   "cluster-dns:cart.otel-demo.svc.cluster.local"
  ///   "git-url:gitlab.example.com/team/repo"
  type correlation-key = string;

  /// A topology fact emitted by any source.
  /// payload-json is intentionally string-encoded JSON to allow
  /// arbitrary nested structure without WIT variant explosion.
  record fact {
    source: string,
    observed-at: timestamp,
    ttl-seconds: u32,
    confidence: confidence,
    fact-type: fact-kind,
    payload-json: string,
    correlation-keys: list<correlation-key>,
  }

  enum fact-kind {
    node,
    edge,
    attr,
    absence,
  }

  /// Common error envelope.
  record err {
    code: string,           /// e.g. "auth_failed", "rate_limited", "schema_mismatch"
    message: string,
    retryable: bool,
  }
}
```

### 1.4 Connector 接口(`wit/connector.wit`)

```wit
package sre:topology@0.1.0;

use types.{fact, err};

interface connector {
  /// Configuration passed at instantiation. JSON-encoded.
  /// Schema is connector-specific; documented per-connector.
  type config = string;

  /// Perform one sync cycle. Host calls this on a schedule.
  /// Returns facts; host is responsible for forwarding to fact-bus.
  sync: func(cfg: config) -> result<list<fact>, err>;

  /// Liveness check. Should be fast (< 100ms).
  health-check: func() -> result<_, err>;

  /// Optional: stream-based emit for high-throughput sources.
  /// Returns stream-handle host can poll with `next-batch`.
  stream-begin: func(cfg: config) -> result<stream-handle, err>;
  next-batch: func(h: stream-handle, max: u32) -> result<list<fact>, err>;
  stream-end: func(h: stream-handle) -> result<_, err>;

  type stream-handle = u32;
}

world connector-world {
  export connector;
}
```

### 1.5 Rule 接口(`wit/rule.wit`)

用于 PRD-006 业务规则 / 巡检规则。**严格沙箱**,无 fs / network 能力:

```wit
package sre:topology@0.1.0;

use types.{timestamp};

interface rule {
  /// One node/edge or metric snapshot the rule evaluates against.
  variant input {
    metric(metric-input),
    node(node-input),
    edge(edge-input),
  }

  record metric-input {
    resource-id: string,
    metric-name: string,
    value: f64,
    observed-at: timestamp,
    labels-json: string,
  }

  record node-input {
    node-id: string,
    node-type: string,
    properties-json: string,
  }

  record edge-input {
    edge-id: string,
    edge-type: string,
    src-id: string,
    dst-id: string,
    properties-json: string,
  }

  record finding {
    severity: severity,
    title: string,
    description: string,
    evidence-json: string,
  }

  enum severity {
    info,
    warning,
    critical,
  }

  /// Evaluate the rule. Empty list = no finding.
  evaluate: func(i: input) -> list<finding>;

  /// Optional metadata (cached by host).
  metadata: func() -> rule-meta;

  record rule-meta {
    rule-id: string,
    version: string,
    description: string,
    applies-to: list<string>,        /// node-types or metric-names
  }
}

world rule-world {
  export rule;
}
```

### 1.6 Handler 接口(`wit/handler.wit`)

用于 PRD-001 Phase 3 自定义恢复动作 — **本来 Phase 3 延后是因为没有沙箱,WASM 解锁这个**:

```wit
package sre:topology@0.1.0;

use types.{err};

interface handler {
  record handler-input {
    target-id: string,
    params-json: string,
    dry-run: bool,
  }

  record handler-result {
    success: bool,
    side-effects-json: string,      /// for 内存孪生层 twin update
    rollback-id: option<string>,
    message: string,
  }

  /// host injects authorized capabilities via custom imports
  /// (e.g. k8s-api, mysql-conn). WASM module cannot syscall.
  execute: func(i: handler-input) -> result<handler-result, err>;
  verify: func(target-id: string) -> result<bool, err>;
  rollback: func(rollback-id: string) -> result<_, err>;
}
```

**host 注入能力**(`wasmtime::Linker`):
- `k8s-api`(只读 / 受限只写)
- `mysql-conn`(预设的 sql 模板)
- `redis-conn`(预设命令集)
- 严格 **deny by default**,白名单加 capability

### 1.7 WIT 版本演化策略

- **`0.x.y`** 阶段(到 T+12mo):允许破坏性变更,但每次变更必须在 `CHANGELOG.md` 留痕
- **`1.0.0`** 之后:遵循 semver
- WIT 加字段:**只允许加到 record 末尾**(不能复用 wasm component canonical ABI offset)
- WIT 改字段名 / 删字段:重大版本,必须新 namespace(`sre:topology@2.0.0`)
- host 同时支持 **N 和 N-1** 版本至少 6 个月

### 1.8 工具链

```toml
# Cargo.toml [build-dependencies]
wit-bindgen = "0.30"
wasmtime = "23"
wasmtime-wasi = "23"

# Build WASM modules with cargo-component
# cargo install cargo-component
```


## 2. 层 B:Tauri Commands(桌面 UI ↔ Rust)

详细 commands 设计、安全模型、TS 类型自动生成、AppState 模式见 [`17-tauri-desktop-architecture.md`](./17-tauri-desktop-architecture.md) §3-§4。本节列契约规范。

### 2.1 设计原则

- Tauri 2.x `#[tauri::command]` + `#[specta::specta]` 双标
- 命名 snake_case 动词开头:`list_executions`, `execute_recovery`, `dry_run_action`
- 参数包成 struct(便于 specta TS 类型生成)
- 错误统一 `Result<T, AppError>`,AppError 是 tagged enum
- 全部 `async fn`
- 事件 emit 用 snake_case 名词:`fact_emitted`, `connector_synced`

### 2.2 模块命名表(对应 connector router)

| Tauri commands 模块 | 等价 设计 router | Phase |
|---|---|---|
| `topology` | `topology.py` / `access_link.py` / `node_impact.py` | 1-2 |
| `recovery` | `recovery.py` | 3 |
| `change_events` | `change_event.py` | 3 |
| `reports` | `report.py` | 4 |
| `connectors` | `connectors.py` | 1-2 |
| `fault_simulation` | `simulation.py` | 4 |
| `system` | (新增 — 桌面专属:配置、路径、版本) | 1 |

### 2.3 类型契约

所有 Tauri commands 入参 / 出参类型 → `specs/tauri-commands/index.ts` 自动生成(`make specs-generate-tauri-types`)。

**前端唯一允许的 API 调用路径**:

```typescript
import { invoke } from '@tauri-apps/api/core';
import type { ExecuteActionArgs, ExecutionResult } from '../../specs/tauri-commands';

const result: ExecutionResult = await invoke('execute_action', { args: { ... } });
```

**禁止** webview 直接 `fetch` / `axios`(Tauri allowlist 已 disable http)。

### 2.4 错误信封

```rust
#[derive(Debug, thiserror::Error, Serialize, Type)]
#[serde(tag = "kind", content = "message")]
pub enum AppError {
    NotFound(String),
    InvalidInput(String),
    Engine(String),
    Storage(String),
    ApprovalRequired(String),
    Internal(String),
}
```

前端 TS 拿到结构化 `{ kind: "ApprovalRequired", message: "..." }`,可 switch 分支。

### 2.5 演化策略

- 加 command:加 + 注册即可,前端可选用
- 加字段(入参 / 出参 struct):末尾加可选字段(`Option<T>`),不破老前端
- 改字段类型 / 改名:必须新 command 名(`execute_action_v2`),老 command 标 deprecated 至少 3 个月
- 删 command:走两步(deprecated → 删)

### 2.6 调试

- DevTools(右键 → Inspect Element):`window.__TAURI__` 全 API 可玩
- Rust 侧 `tracing` 日志:`make desktop-dev` 终端直接看
- specta 生成的 TS 类型与 Rust struct 自动对齐,IDE 提示完整

## 3. 层 C:REST + Arrow Flight(headless 模式专属)

**仅在 `engine-cli serve` 启动时生效**;Tauri 桌面模式下这层不存在。

### 3.1 适用场景

- 团队部署 engine-cli 作中心服务,多客户端共享
- SaaS Web shell(未来)调 engine-cli
- 远程 connector(部署在客户云端)上传 fact 到中心

### 3.2 协议分工

REST 走低频 req/resp(状态查询 / 触发 sync / 列资源),Arrow Flight 走高吞吐 fact 数据流(connector 上传 fact batch)。复用底层 tonic gRPC,共享 mTLS 配置。

### 3.3 Arrow Flight Fact Schema

所有 fact 内部表示统一 Arrow RecordBatch(connector → bus → Identity Resolver → store)。跨进程发送走 Arrow Flight(底层 gRPC + Arrow IPC,零拷贝)。进程内传递走 RecordBatch 直引用。

Batch size:**500-2000 fact / batch**(初值 1000),流式上传用 `DoExchange` 控制 backpressure。

```rust
// engine/crates/engine-core/src/schema/fact.rs
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use std::sync::Arc;

pub fn fact_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("source", DataType::Utf8, false),
        Field::new(
            "observed_at",
            DataType::Timestamp(TimeUnit::Millisecond, Some(Arc::from("UTC"))),
            false,
        ),
        Field::new("ttl_seconds", DataType::UInt32, false),
        Field::new("confidence", DataType::Float32, false),
        // "node" | "edge" | "attr" | "absence"
        Field::new("fact_type", DataType::Utf8, false),
        // JSON-encoded payload(见 §3.5)
        Field::new("payload_json", DataType::Utf8, false),
        Field::new(
            "correlation_keys",
            DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
            false,
        ),
        Field::new("trace_id", DataType::Utf8, true),
    ]))
}
```

**字段说明**:

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `source` | string | ✅ | connector 标识,如 `"k8s"`, `"jaeger"`, `"coderepo:gitlab"` |
| `observed_at` | timestamp(ms, UTC) | ✅ | 原始观测时刻,**非**发送时刻 |
| `ttl_seconds` | u32 | ✅ | 默认 600 |
| `confidence` | f32 | ✅ | [0.0, 1.0],trace 推断 0.6 / API 直读 0.95 |
| `fact_type` | string | ✅ | 4 取一 |
| `payload_json` | string | ✅ | 见 §3.5 |
| `correlation_keys` | list\<string\> | ✅ | 至少 1 个,Identity Resolver 用来 merge |
| `trace_id` | string | ❌ | OpenTelemetry trace_id,用于调试 |

### 3.4 为什么 payload 用 JSON string 而不是 Arrow Union

- Arrow Union(Dense/Sparse)跨语言互操作有边角 case
- 4 类 payload 字段差异大,硬塞 Struct 要 nullable 一堆
- 解析时按 `fact_type` 走 `serde_json` 分支,等同 protobuf oneof 的开销
- **未来想升 Arrow Struct 时,加一列 `payload_typed` 并存即可,不破老契约**

### 3.5 已知 payload 形态(JSON schema)

**Node payload**:
```json
{
  "node_type": "ExternalService",
  "node_id_hint": "ext:api.stripe.com",
  "display_name": "api.stripe.com",
  "properties": {
    "endpoint": "https://api.stripe.com",
    "owner_team": "team-pay"
  }
}
```

**Edge payload**:
```json
{
  "edge_type": "CALLS",
  "src_id": "deploy:vm-cluster:otel-demo:payment-service",
  "dst_correlation_key": "domain:api.stripe.com",
  "properties": {
    "call_count_5m": 217,
    "error_count_5m": 0
  }
}
```

`dst_correlation_key`:**关键**,允许指向"还不存在的节点",Identity Resolver 解析后回填实际 `dst_id`。

**Attr payload**:
```json
{
  "target_correlation_key": "domain:api.stripe.com",
  "field": "owner_team",
  "value": "team-pay",
  "source_priority": 0.8
}
```

**Absence payload**(罕用,"我观察到 X 不存在了"):
```json
{
  "target_correlation_key": "ip:10.0.0.5",
  "reason": "k8s_pod_deleted",
  "last_seen": 1718937600000
}
```

### 3.6 Flight RPC 路径

Rust Arrow Flight server 暴露:

| Path | Verb | 用途 |
|---|---|---|
| `/grpc.flight.FlightService/DoPut` | client-stream | Connector 推 fact batch |
| `/grpc.flight.FlightService/DoGet` | server-stream | 引擎内部 fact 订阅 |
| `/grpc.flight.FlightService/DoExchange` | bidi | 高吞吐 + backpressure |
| `/grpc.flight.FlightService/GetSchema` | unary | 客户端发现 fact schema |
| `/grpc.flight.FlightService/ListFlights` | server-stream | 列已注册 stream(诊断) |

**Auth**:Flight 底层 tonic gRPC,直接配 mTLS;Phase 1 可先 token-based(`Authorization: Bearer <token>`)。

### 3.7 Connector → Bus 上传示例(伪 Rust client)

```rust
use arrow_flight::client::FlightClient;
use arrow::record_batch::RecordBatch;

let mut client = FlightClient::new(channel);
let batch: RecordBatch = build_fact_batch(facts);  // 1000 fact
let descriptor = FlightDescriptor::new_path(vec!["facts".into(), "v1".into()]);

let stream = futures::stream::iter(vec![FlightData::from(&batch)]);
client.do_put(descriptor, stream).await?;
```

### 3.8 REST 端点(engine-cli 暴露)

| Method | Path | 用途 |
|---|---|---|
| GET | `/api/v1/health` | 引擎健康 |
| GET | `/api/v1/connectors` | 列 connector(含 WASM 加载状态) |
| POST | `/api/v1/connectors/{name}:sync` | 触发同步 |
| GET | `/api/v1/facts/recent` | 调试:最近 N 条 fact(分页) |
| GET | `/api/v1/nodes/{node_id}` | 查节点 |
| GET | `/api/v1/nodes/{node_id}/neighbors` | 一跳邻居 |
| POST | `/api/v1/queries:bfs` | 反向 BFS 查 propagation |
| GET | `/api/v1/unknown-deps` | PRD-005 Queue |
| POST | `/api/v1/unknown-deps/{id}:promote` | 一键入图 |
| GET | `/api/v1/executions` | 列 recovery executions |
| POST | `/api/v1/executions` | 发起 recovery |
| POST | `/api/v1/executions/{id}:rollback` | 回滚 |
| GET | `/api/v1/change-events` | 列 ChangeEvent |
| ... | ... | (其余对照 Tauri commands,1:1 映射) |

### 3.9 REST 设计原则

- 资源风格 URL,动词通过 HTTP method 表达;**资源用复数**:`/executions` ✅ / `/getExecution` ❌
- JSON body,顶层永远是对象,方便加字段
- 子资源平铺,**不深嵌**
- 动作类用动词后缀(Google AIP 风格):`/executions/{id}:rollback`,与 RESTful 资源解耦
- 版本走 URL prefix:`/api/v1/...`

### 3.10 错误信封(REST 模式)

```json
{
  "error": {
    "code": "unknown_correlation_key",
    "message": "No fact matched correlation_key=domain:foo.example.com in last 1h",
    "details": {
      "correlation_key": "domain:foo.example.com",
      "window_seconds": 3600
    },
    "trace_id": "0x123abc...",
    "retryable": false
  }
}
```

### 3.11 分页

```
GET /api/v1/executions?cursor=<opaque>&limit=100
```

```json
{
  "items": [...],
  "next_cursor": "abc123",
  "total": null
}
```

**不用** offset/page-number(长尾大数据集下不稳)。

### 3.12 Schema / 工具链

```toml
# Cargo.toml(engine-cli 依赖)
[dependencies]
arrow         = "54"
arrow-flight  = "54"
datafusion    = "44"          # SQL on Arrow,Identity Resolver 用
tonic         = "0.12"        # gRPC,Flight 底层
axum          = "0.7"
utoipa        = "5"           # OpenAPI generator
utoipa-swagger-ui = "8"
```

**Schema 演化**(Arrow Flight):
- **加列**:允许,放末尾,旧 reader 忽略
- **改列类型**:不允许,必须新版本 schema(`facts/v2`)
- **删列**:走两步(deprecated 6 个月 → 删,升 schema 版本)
- **改列名**:同删列

每个 Flight descriptor path 带版本(`facts/v1` / `facts/v2`),server 同时支持 N 和 N-1 至少 6 个月。

## 4. 跨层数据流动示例

**场景**:Tauri 桌面模式下,K8s Pod 通过 trace 暴露对 Stripe 的调用,经过完整契约链。

```
1. jaeger-connector.wasm
     ↓ 层 A:WIT  (sync() → list<fact>)
2. Rust host(Tauri 主进程内)接收 fact list
     ↓ 层 D:封装成 Arrow RecordBatch(内存)
3. Fact bus 写入(Arrow 内部表示)
     ↓ DataFusion SQL 在 RecordBatch 上做 Identity Resolver
4. Unknown Dep Queue 入队(Arrow 表)
     ↓ Tauri emit('unknown_dep_added', payload)
5. React webview 收到事件,刷新 unknown-deps 视图
     ↓ 层 B:Tauri command — invoke('list_unknown_deps')
6. SRE 点击「一键入图」
     ↓ 层 B:Tauri command — invoke('promote_unknown_dep', {id})
7. Rust 写入 canonical store(Arrow)+ SQLite metadata
     ↓ engine-storage 持久化
8. Tauri emit('node_added')
     ↓ webview 收事件,refetch topology
9. Cytoscape 渲染新节点
```

**Headless 模式同场景**(engine-cli + 远程 Web shell):
- 步骤 4 → Arrow Flight DoExchange 推 fact 给 SaaS Web shell 或事件总线
- 步骤 5-6 → REST `GET /api/v1/unknown-deps` + `POST /api/v1/unknown-deps/{id}:promote`
- 其余等价

**Tauri 模式下所有跨层通信是进程内函数调用 + Tauri IPC**,无 HTTP 栈。Headless 模式下层 C(REST + Flight)出现。

## 5. 性能 / 体量 baseline

| 路径 | 目标 | 测试方式 |
|---|---|---|
| WASM connector → Rust host fact emit | ≥ 50k fact/s | criterion bench |
| Arrow Flight DoPut(headless 模式,本机) | ≥ 100k fact/s | 自带 bench |
| Identity Resolver(DataFusion SQL) | < 50ms 处理 10k fact | criterion |
| Tauri invoke 一次往返(空 command) | < 1ms | 自建 bench |
| Tauri invoke 一次往返(典型 query) | < 10ms | 自建 bench |
| REST `/nodes/{id}` P99(headless) | < 10ms(内存图) | wrk 压测 |
| BFS depth=4 on 10k-node graph | < 100ms | criterion |
| Cytoscape 渲染 200 节点 | < 200ms | browser perf |

**Phase 1 验收**:跑通这些 baseline,**与基线对比 ≥ 5×**。

## 6. Contract Testing 框架

Contract test 是 **单 Rust runner**(无双语言双跑)。

每个核心模块,写 Rust contract test(钉住关键行为):

```rust
// tests/contract/prd-002-changes/record_change.rs
use engine_testkit::{TestEngine, expect};

#[tokio::test]
async fn configmap_update_propagation() {
    let engine = TestEngine::with_fixtures("changes_basic").await;

    let result = engine.record_change(RecordChangeArgs {
        change_type: "configmap_updated".into(),
        target_resource_id: "cm:vm-cluster:otel-demo:app-config".into(),
        author: "alice".into(),
        diff_summary: serde_json::json!({"key":"log_level","old":"INFO","new":"DEBUG"}),
    }).await.unwrap();

    expect!(result.severity_estimate, "medium");
    expect!(result.propagated_to.len(), between: 3..=8);
    expect!(result.change_event_id, matches: r"^ce-[0-9]+$");

    // 副作用断言
    expect!(engine.storage.change_events_count().await, 1);
}
```

契约测试与实现在同一 workspace,直接跑:

```bash
cargo test --test contract -p engine-changes configmap_update_propagation
```

行为断言(propagated_to 长度 / severity 档位 / ID 格式 / 副作用计数)全部写在测试内,不依赖外部基线。



## 7. 版本基线锁定(T+0)

### 7.1 Rust crate baseline

```toml
# topology-engine/Cargo.toml
[workspace.dependencies]
# Async runtime
tokio = { version = "1.40", features = ["full"] }

# Arrow ecosystem
arrow = "54"
arrow-flight = "54"
arrow-ipc = "54"
datafusion = "44"

# WASM
wasmtime = "23"
wasmtime-wasi = "23"
wit-bindgen = "0.30"

# Network
tonic = "0.12"
axum = "0.7"
tower = "0.5"
hyper = "1"

# Serde
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Logging
tracing = "0.1"
tracing-subscriber = "0.3"

# Utility
anyhow = "1"
thiserror = "1"

# Dev
criterion = "0.5"
proptest = "1"
```

### 7.3 升级节奏

- **patch 版本**:可随意升,CI 通过即可
- **minor 版本**:每季度评估,主流 crate 走最新稳定
- **major 版本**:每半年评估,要写迁移 RFC

## 8. 安全考虑

| 边界 | 风险 | 控制 |
|---|---|---|
| WASM 模块来源 | 恶意 connector / rule | host 验证 `.wasm` SHA-256 + 签名(Phase 3 加入) |
| WASM capability 注入 | rule 调 k8s 删 Pod | 默认 deny-all,白名单 capability,Recovery handler 严格审计 |
| Arrow Flight 跨网络 | 中间人 / 重放 | mTLS + 短 TTL token |
| REST 控制面 | 越权 / 注入 | OAuth2/JWT(Phase 2 接入),input 强 Pydantic / serde 校验 |
| 图数据库 持久化 | 写入污染 | 所有写操作经 Rust 引擎,外部 adapter 仅作 sink |

## 9. 不做(本规范)

- **不用 protobuf**
- **不用 GraphQL**(REST + 内嵌 BFS 端点足够)
- **不用 MessagePack / CBOR / Avro**(选定 Arrow + JSON 不再增)
- **不在 WASM 边界传 Arrow**(用 WIT 即可,Arrow 留 host 内部)
- **不实现自定义 RPC 协议**(已经有 Flight + REST,够了)

## 10. 检索 / 工具

```bash
# WIT 文件检查
wasm-tools component wit wit/

# Arrow schema 比对 —— engine-storage 自带 dump example
#   cargo run -p engine-storage --example dump_topology -- db.sqlite

# headless sync / view 验证 —— engine-cli tick + inspect_views example
#   cargo run -p engine-cli --release -- tick

# OpenAPI lint
npx @redocly/cli lint http://localhost:8080/openapi.json

# Contract test 双跑
make contract-test         # 运行 pytest + cargo test
```

## 11. 变更管理

- 任何 schema / 接口变更必须改本文 + 升 crate / proto / WIT 版本
- 破坏性变更走 RFC 流程(doc/rfc-NNN-...md),至少留 14 天评议
- contract YAML 永远只 append,不改老条目

## 12. 相关文档

- PRD 实施:[`11-PRD-005-...`](./11-PRD-005-universal-topology-service.md) / [`12-PRD-006-...`](./12-PRD-006-code-repo-source.md)
- 端到端剧本:[`13-story-unknown-dep-stripe.md`](./13-story-unknown-dep-stripe.md)
- 导航:[`00-README.md`](./00-README.md)

---

**版本**:v0.1.0 — 初稿,Phase 0 决策快照(2026-06-23)。Phase 1 实施开始时升 v0.2.0。
