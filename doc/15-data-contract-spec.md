# 15 — 数据契约规范:WIT + Arrow Flight + JSON/REST 三层

## 0. 上下文

本文是 [`14-long-term-tech-strategy.md`](./14-long-term-tech-strategy.md) §4 数据契约决策的**详细规约**。所有 Rust 引擎实现、WASM 插件开发、Python ↔ Rust 互操作、前端 API 调用**必须遵守本文 schema**。

**核心决策**:三层契约,各司其职,**不使用 protobuf**。

```
┌─────────────────┐  JSON/REST       ┌─────────────────┐
│   Frontend TS   │ ◄──────────────► │  Python FastAPI │
└─────────────────┘                  └────────┬────────┘
                                              │ JSON/REST(控制面)
                                              ▼
                                     ┌─────────────────┐
                          ┌─────────►│ Rust            │
                          │          │ topology-engine │
              WIT (零拷贝) │          └────────┬────────┘
                          │                   │ Arrow Flight
                  ┌───────┴────┐              │ (数据面)
                  │ WASM Module│              ▼
                  └────────────┘    ┌──────────────────┐
                                    │ External Agents  │
                                    │ (Cloud/On-Prem)  │
                                    └──────────────────┘
```

| 层 | 协议 | 边界 | 频率/体量 | 调试方式 |
|---|---|---|---|---|
| **A** | WIT (Component Model) | WASM guest ↔ Rust host | 小消息高频 | `wasmtime explore`, `wit-tools` |
| **B** | Arrow Flight (gRPC + Arrow IPC) | Connector → Fact 总线 | 大批量流式 | `pyarrow.flight.FlightClient` REPL |
| **C** | JSON over REST | TS/Python ↔ Rust 控制面 | 低频 req/resp | `curl`, OpenAPI Swagger UI |

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
    side-effects-json: string,      /// for DSS twin update
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

Python 端:`wasmtime-py`(只在测试 / CLI 时用,生产 host 是 Rust)。

## 2. 层 B:Arrow Flight(Fact 总线数据面)

### 2.1 设计原则

- **所有 fact 内部表示统一 Arrow RecordBatch**(connector → bus → Identity Resolver → store)
- 跨进程发送走 **Arrow Flight**(底层 gRPC + Arrow IPC,零拷贝)
- 进程内传递走 **RecordBatch 直引用**(不再 marshal)
- batch size:**500-2000 fact / batch**(实测拿 sweet spot,初值用 1000)
- 流式上传用 `DoExchange`(双向),控制 backpressure

### 2.2 Fact 主 Schema

```rust
// topology-engine/src/schema/fact.rs
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
        // "node" | "edge" | "attr" | "absence" (low-card; future: DictionaryArray)
        Field::new("fact_type", DataType::Utf8, false),
        // JSON-encoded payload (see §2.4 for known payload shapes)
        Field::new("payload_json", DataType::Utf8, false),
        Field::new(
            "correlation_keys",
            DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
            false,
        ),
        // Optional: trace id for debugging
        Field::new("trace_id", DataType::Utf8, true),
    ]))
}
```

**字段说明**:

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `source` | string | ✅ | connector 标识符,如 `"k8s"`, `"jaeger"`, `"coderepo:gitlab"` |
| `observed_at` | timestamp(ms, UTC) | ✅ | 原始观测时刻,**非**发送时刻 |
| `ttl_seconds` | u32 | ✅ | 多久后这条 fact 应被视为过期(默认 600) |
| `confidence` | f32 | ✅ | [0.0, 1.0],trace 推断 0.6 / API 直读 0.95 |
| `fact_type` | string | ✅ | 4 取一 |
| `payload_json` | string | ✅ | 见 §2.4 |
| `correlation_keys` | list\<string\> | ✅ | 至少 1 个,Identity Resolver 用它做合并 |
| `trace_id` | string | ❌ | OpenTelemetry trace_id,用于调试和审计 |

### 2.3 为什么 payload 用 JSON string 而不是 Arrow Union

- Arrow Union(Dense/Sparse)跨语言互操作有边角 case,pyarrow 旧版本支持不全
- 4 类 payload 字段差异大(node 有 node_type / display_name / properties;edge 有 src/dst / edge_type / properties),硬塞 Struct 要 nullable 一堆
- 解析时按 `fact_type` 走 `serde_json` 分支,等同 protobuf oneof 的开销
- **未来想升 Arrow Struct 时,加一列 `payload_typed` 并存即可,不破老契约**

### 2.4 已知 payload 形态(JSON schema)

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

**Absence payload**(罕用,用于"我观察到 X 不存在了"):
```json
{
  "target_correlation_key": "ip:10.0.0.5",
  "reason": "k8s_pod_deleted",
  "last_seen": 1718937600000
}
```

### 2.5 Flight RPC 路径

Rust Arrow Flight server 暴露:

| Path | Verb | 用途 |
|---|---|---|
| `/grpc.flight.FlightService/DoPut` | client-stream | Connector 推 fact batch |
| `/grpc.flight.FlightService/DoGet` | server-stream | 引擎内部 fact 订阅(留 Phase 2 用) |
| `/grpc.flight.FlightService/DoExchange` | bidi | 高吞吐场景 + backpressure |
| `/grpc.flight.FlightService/GetSchema` | unary | 客户端发现 fact schema |
| `/grpc.flight.FlightService/ListFlights` | server-stream | 列已注册 stream(诊断用) |

**Auth**:Flight 底层是 tonic gRPC,直接配 mTLS;Phase 1 可先 token-based(`Authorization: Bearer <token>`)。

### 2.6 Connector → Bus 上传示例(伪 Python)

```python
import pyarrow as pa
import pyarrow.flight as flight

# Connector 累积 fact 到 batch
client = flight.FlightClient("grpc://localhost:50051")
schema = pa.schema([
    ("source", pa.string()),
    ("observed_at", pa.timestamp("ms", tz="UTC")),
    ("ttl_seconds", pa.uint32()),
    ("confidence", pa.float32()),
    ("fact_type", pa.string()),
    ("payload_json", pa.string()),
    ("correlation_keys", pa.list_(pa.string())),
])

batch = pa.record_batch([
    pa.array(["k8s"] * 1000),
    pa.array([...]),       # observed_at
    pa.array([600] * 1000, type=pa.uint32()),
    ...
], schema=schema)

descriptor = flight.FlightDescriptor.for_path("facts/v1")
writer, _ = client.do_put(descriptor, schema)
writer.write_batch(batch)
writer.close()
```

### 2.7 Schema 演化策略

- **加列**:允许,放末尾,旧 reader 忽略
- **改列类型**:不允许,必须新版本 schema(`facts/v2`)
- **删列**:走两步:先标 deprecated 6 个月,再删,且必须升 schema 版本
- **改列名**:同删列,新名加列 + 旧名标 deprecated

每个 Flight descriptor path 带版本(`facts/v1` / `facts/v2`),server 同时支持 N 和 N-1 至少 6 个月。

### 2.8 工具链

```toml
# Cargo.toml
[dependencies]
arrow = "54"
arrow-flight = "54"
datafusion = "44"          # SQL on Arrow,Identity Resolver 用
tonic = "0.12"             # gRPC,Flight 底层
tokio = { version = "1", features = ["full"] }
```

Python 端:
```toml
# pyproject.toml
pyarrow = "^18"            # 与 Rust arrow=54 ABI 兼容
```

**ABI 兼容性**:`arrow-rs 54.x` ↔ `pyarrow 18.x`(Arrow C Data Interface)。升级时双侧锁版本,先在 staging 验证。

## 3. 层 C:JSON over REST(控制面 + 业务 API)

### 3.1 设计原则

- **资源风格 URL**,动词通过 HTTP method 表达
- **JSON body**,顶层永远是对象(不是数组,方便加字段)
- **错误统一信封**(下文 §3.4)
- **OpenAPI 3.1 自动生成**,前端 / Python 都从此白嫖 client
- 版本走 URL prefix:`/api/v1/...` / `/api/v2/...`

### 3.2 URL 命名约定

- 资源用复数:`/facts`, `/connectors`, `/executions`,**不用** `/getFact`
- 子资源平铺:`/connectors/{name}/sync` ✅,**不深嵌**:`/connectors/{name}/sync-cycles/{cycle}/...` ❌
- 动作类用动词后缀:`/executions/{id}:rollback`(Google AIP 风格),与 RESTful 资源解耦

### 3.3 标准端点(Rust topology-engine 暴露)

| Method | Path | 用途 |
|---|---|---|
| GET | `/api/v1/health` | 引擎健康 |
| GET | `/api/v1/connectors` | 列 connector(含 WASM 加载状态) |
| POST | `/api/v1/connectors/{name}:sync` | 触发同步 |
| GET | `/api/v1/facts/recent` | 调试:最近 N 条 fact(分页) |
| GET | `/api/v1/nodes/{node_id}` | 查节点 |
| GET | `/api/v1/nodes/{node_id}/neighbors` | 一跳邻居(替代 Cypher) |
| POST | `/api/v1/queries:bfs` | 反向 BFS 查 propagation |
| GET | `/api/v1/unknown-deps` | PRD-005 Queue |
| POST | `/api/v1/unknown-deps/{id}:promote` | 一键入图 |
| GET | `/api/v1/wasm-modules` | 列已加载 WASM 模块 |
| POST | `/api/v1/wasm-modules:load` | 加载新 WASM(WASM URL + sha256 hash) |

Python FastAPI 端**保留现有所有路由**(`/api/v1/topology`, `/api/v1/recovery/*`, `/api/v1/change-events/*` 等),只是内部实现从 Python DSS 改为调 Rust REST。

### 3.4 错误信封

所有 4xx / 5xx 响应:

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

`error.code` 是机读;`message` 是给人看的;`details` 是结构化上下文;`trace_id` 让用户报问题时能立刻定位。

### 3.5 分页约定

列表型 GET:
- query param:`?cursor=<opaque>&limit=100`
- response:
  ```json
  {
    "items": [...],
    "next_cursor": "abc123",       // null when no more
    "total": null                  // optional, only when cheap
  }
  ```
- **不用** offset/page-number(在长尾大数据集下不稳)

### 3.6 OpenAPI 工具链

```toml
# Rust axum 端
[dependencies]
axum = "0.7"
utoipa = "5"              # OpenAPI generator
utoipa-swagger-ui = "8"   # Swagger UI

# 生成的 openapi.json 可被前端 TS 用:
# npx openapi-typescript openapi.json -o src/api/generated.ts
```

Python 端 FastAPI 已自带 OpenAPI 自动生成。

## 4. 跨层数据流动示例

**场景**:一个 K8s Pod 通过 trace 暴露出对 Stripe 的调用,经过完整 3 层契约。

```
1. jaeger-connector.wasm
     ↓ WIT  (sync() → list<fact>)
2. Rust host 接收 fact list
     ↓ 内部封装成 Arrow RecordBatch
3. Fact bus 写入 (Arrow 内部表示)
     ↓ DataFusion SQL 在 RecordBatch 上做 Identity Resolver
4. Unknown Dep Queue 入队 (Arrow 表)
     ↓ Arrow Flight DoGet (前端轮询)
     ↓ 或 REST GET /api/v1/unknown-deps  (前端直接查)
5. SRE 点击「一键入图」
     ↓ REST POST /api/v1/unknown-deps/{id}:promote
6. Rust 写入 canonical store (Arrow)
     ↓ 异步双写 Neo4j (通过 Python adapter REST)
7. Python FastAPI /api/v1/topology 查询时
     ↓ REST GET (内部调) Rust /api/v1/nodes/{id}/neighbors
     ↓ Rust 返回 JSON (从 Arrow store 转)
8. 前端 Cytoscape 渲染新节点
```

**3 种契约都参与了**:WIT (1)、Arrow Flight + 内部 Arrow (2-4)、REST (4-7)。

## 5. 性能 / 体量 baseline

| 路径 | 目标 | 测试方式 |
|---|---|---|
| WASM connector → Rust host fact emit | ≥ 50k fact/s | criterion bench |
| Arrow Flight DoPut(本机) | ≥ 100k fact/s | 自带 bench |
| Identity Resolver(DataFusion SQL) | < 50ms 处理 10k fact | criterion |
| REST /nodes/{id} P99 | < 10ms(内存图) | wrk 压测 |
| BFS depth=4 on 10k-node graph | < 100ms | criterion |
| Python ↔ Rust REST 一次往返(本机) | < 5ms | requests benchmark |

**Phase 1 验收**:跑通这些 baseline,**与 Python 现状对比 ≥ 5×**。

## 6. Contract Testing 框架

每个被迁移的 Python 模块,迁移前先写 YAML contract:

```yaml
# tests/contract/prd-002/record_change.yaml
contract: record_change
description: PRD-002 ChangeEvent 写入 + propagation 推导 + Neo4j dual-write

inputs:
  - name: configmap_update_with_propagation
    request:
      change_type: configmap_updated
      target_resource_id: cm:vm-cluster:otel-demo:app-config
      author: alice
      diff_summary: { key: log_level, old: INFO, new: DEBUG }
    expected:
      response_status: 200
      response_body:
        change_event_id: { type: string, pattern: "^ce-[0-9]+$" }
        severity_estimate: medium
        propagated_to_count: { gte: 3, lte: 8 }
      side_effects:
        dss.change_events.count: +1
        neo4j.cypher:
          query: "MATCH (:ChangeEvent {node_id:$id}) RETURN count(*) as n"
          expected: { n: 1 }
```

**测试 runner 双跑**:
- `pytest tests/contract/` — Python 实现跑过
- `cargo test --test contract` — Rust 实现跑过同一份 YAML

Phase 4 迁移每个模块时:contract 全过 + 流量 diff 为零 + 切流量。

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

### 7.2 Python baseline

```toml
# backend/pyproject.toml(增量)
[project.dependencies]
pyarrow = "^18"
grpcio = "^1.66"        # for pyarrow.flight
fastapi = "^0.115"
pydantic = "^2.9"
httpx = "^0.27"
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
| Neo4j 持久化 | 写入污染 | 所有写操作经 Rust 引擎,Python adapter 仅作 sink |

## 9. 不做(本规范)

- **不用 protobuf**(理由见 doc/14 §4)
- **不用 GraphQL**(REST + 内嵌 BFS 端点足够)
- **不用 MessagePack / CBOR / Avro**(选定 Arrow + JSON 不再增)
- **不在 WASM 边界传 Arrow**(用 WIT 即可,Arrow 留 host 内部)
- **不实现自定义 RPC 协议**(已经有 Flight + REST,够了)

## 10. 检索 / 工具

```bash
# WIT 文件检查
wasm-tools component wit wit/

# Arrow schema 比对
python -c "import pyarrow.parquet as pq; print(pq.read_schema('sample.parquet'))"

# Flight 端点联通性
python -c "
import pyarrow.flight as flight
c = flight.FlightClient('grpc://localhost:50051')
for f in c.list_flights():
    print(f.descriptor)
"

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

- 上游决策:[`14-long-term-tech-strategy.md`](./14-long-term-tech-strategy.md)
- PRD 实施:[`11-PRD-005-...`](./11-PRD-005-universal-topology-service.md) / [`12-PRD-006-...`](./12-PRD-006-code-repo-source.md)
- 端到端剧本:[`13-story-unknown-dep-stripe.md`](./13-story-unknown-dep-stripe.md)
- 导航:[`00-README.md`](./00-README.md)

---

**版本**:v0.1.0 — 初稿,Phase 0 决策快照(2026-06-23)。Phase 1 实施开始时升 v0.2.0。
