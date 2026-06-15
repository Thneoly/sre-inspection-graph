# 09 — 数据源服务架构设计

## 1. 目标

将数据源从业务逻辑中**解耦**，引入独立的 **Data Source Service (DSS)** 中间层：

```
故障注入系统                        巡检展示系统
    │                                   │
    │  PATCH /inject                    │  GET /nodes /edges /metrics
    ▼                                   ▼
┌─────────────────────────────────────────────────┐
│              Data Source Service (DSS)           │
│                                                  │
│  ┌──────────┐  ┌──────────┐  ┌───────────────┐  │
│  │ 节点仓库  │  │  边仓库   │  │  指标时序仓库  │  │
│  │ (nodes)  │  │ (edges)  │  │  (metrics)    │  │
│  └──────────┘  └──────────┘  └───────────────┘  │
│                                                  │
│  接口:                                            │
│    GET  /api/v1/datasource/nodes                  │
│    GET  /api/v1/datasource/edges                  │
│    GET  /api/v1/datasource/metrics/{id}           │
│    GET  /api/v1/datasource/topology/{app}         │
│    PATCH /api/v1/datasource/nodes/{id}            │
│    PATCH /api/v1/datasource/edges/{id}            │
│    PATCH /api/v1/datasource/inject-fault          │
│    POST /api/v1/datasource/step                  │
│    POST /api/v1/datasource/reset                 │
└─────────────────────────────────────────────────┘
    │                                   │
    ▼                                   ▼
  Neo4j (持久化)                   In-Memory (实时状态)
```

## 2. 数据模型

### 2.1 节点 (Node)

```yaml
DataNode:
  id: string                      # 唯一标识
  type: string                    # 节点类型 (Pod, Deployment, MySQL...)
  name: string                    # 显示名称
  properties:
    # 静态属性 (类型决定)
    cluster_id: string
    namespace: string
    owner_team: string
    # ...

    # 动态属性 (实时变化)
    health_status: normal | warning | critical
    risk_level: low | medium | high | critical

    # 指标属性 (类型决定)
    cpu_usage_percent: float
    memory_usage_percent: float
    restart_count: int
    qps: float
    error_rate: float
    disk_usage_percent: float
    # ...
```

### 2.2 边 (Edge)

```yaml
DataEdge:
  id: string                      # 唯一标识
  source_id: string               # 源节点 ID
  target_id: string               # 目标节点 ID
  relationship_type: string       # CONTAINS / USES / DEPENDS_ON...
  relationship_name: string       # 中文名称
  properties:
    dependency_strength: 强 | 中 | 弱
    is_required: 是 | 否
    health_status: normal | warning | critical
    risk_signal: string           # 风险描述
    discovery_method: string
```

### 2.3 指标快照 (MetricSnapshot)

```yaml
MetricSnapshot:
  snapshot_id: string
  resource_id: string             # 关联节点 ID
  metric_name: string             # cpu_usage / memory_usage / qps...
  current_value: float
  unit: string
  fetched_at: ISO8601
  warning_breached: bool
  critical_breached: bool
```

### 2.4 故障注入记录

```yaml
FaultInjection:
  injection_id: string
  fault_type: string              # cpu_spike / memory_leak / ...
  target_id: string               # 目标节点 ID
  current_stage: int
  total_stages: int
  status: injected | escalating | propagating | resolved
  injected_at: ISO8601
  stages: list[FaultStage]        # 预定义的阶段数据
```

### 2.5 故障阶段

```yaml
FaultStage:
  sequence: int
  offset_seconds: int
  health: normal | warning | critical
  risk: low | medium | high | critical
  metric_name: string
  metric_value: float
  unit: string
  triggers_alert: bool
  triggers_finding: bool
```

## 3. 数据源服务接口

### 3.1 数据提取接口（供巡检展示系统使用）

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/datasource/nodes` | 所有节点（含当前实时属性） |
| GET | `/datasource/nodes/{id}` | 单个节点详情 |
| GET | `/datasource/edges` | 所有边 |
| GET | `/datasource/topology/{app}` | 应用完整拓扑（节点+边） |
| GET | `/datasource/metrics/{id}` | 节点最新指标快照列表 |
| GET | `/datasource/metrics/{id}/history` | 节点指标历史（时间范围） |

### 3.2 数据注入接口（供故障注入系统使用）

| 方法 | 路径 | 说明 |
|------|------|------|
| PATCH | `/datasource/nodes/{id}` | 更新节点属性（health/risk/metrics） |
| PATCH | `/datasource/edges/{id}` | 更新边属性 |
| POST | `/datasource/inject-fault` | 注入故障场景 |
| POST | `/datasource/step` | 推进时间线 |
| POST | `/datasource/reset` | 重置所有实时数据到基线 |

### 3.3 数据初始化接口

| 方法 | 路径 | 说明 |
|------|------|------|
| POST | `/datasource/init` | 从 Neo4j 加载基线数据到内存 |
| POST | `/datasource/sync` | 将内存状态同步回 Neo4j |

## 4. 各主数据对象的属性取值规范

### 4.1 计算类 (Compute)

**KubernetesCluster**
```
属性: cluster_id, cluster_name, version, provider, region, node_count
取值范围:
  node_count: 1-100
  version: "1.27" | "1.28" | "1.29" | "1.30"
```

**KubernetesNode**
```
属性: node_name, node_ip, instance_type, cpu_capacity, memory_capacity, pod_capacity
      kernel_version, kubelet_version, node_status, conditions
      cpu_usage_percent, memory_usage_percent, disk_usage_percent
取值范围:
  cpu_capacity: 4 | 8 | 16 | 32 | 64
  memory_capacity: "8Gi" | "16Gi" | "32Gi" | "64Gi" | "128Gi"
  pod_capacity: 110 | 250 | 500
  node_status: "Ready" | "NotReady" | "Unknown"
  cpu_usage_percent: 10.0-95.0
  memory_usage_percent: 20.0-95.0
  disk_usage_percent: 10.0-95.0
```

**Deployment**
```
属性: deployment_name, namespace, replicas, available_replicas, strategy, selector
      cpu_request, memory_request, cpu_limit, memory_limit
取值范围:
  replicas: 1-10
  available_replicas: 0-replicas
  strategy: "RollingUpdate" | "Recreate"
  cpu_request: "100m" | "250m" | "500m" | "1000m" | "2000m"
  memory_request: "128Mi" | "256Mi" | "512Mi" | "1Gi" | "2Gi"
```

**Pod**
```
属性: pod_name, pod_ip, host_ip, node_name, phase, ready, restart_count
      owner_kind, owner_name, cpu_usage_percent, memory_usage_percent, qps, error_rate
取值范围:
  phase: "Running" | "Pending" | "Failed" | "Succeeded" | "Unknown"
  ready: true | false
  restart_count: 0-100
  cpu_usage_percent: 5.0-98.0
  memory_usage_percent: 10.0-98.0
  qps: 0-5000
  error_rate: 0.0-1.0 (正常 < 0.01)
```

**Container**
```
属性: container_name, image, image_digest, cpu_request, memory_request
      cpu_limit, memory_limit, cpu_usage_percent, memory_usage_percent
      restart_count, ready, ports
取值范围:
  cpu_usage_percent: 5.0-98.0
  memory_usage_percent: 10.0-98.0
  restart_count: 0-100
```

### 4.2 网络类 (Network)

**Service**
```
属性: service_name, service_type, cluster_ip, ports, selector
      endpoint_count, endpoint_ready_count
取值范围:
  service_type: "ClusterIP" | "NodePort" | "LoadBalancer"
  endpoint_count: 1-10
  endpoint_ready_count: 0-endpoint_count
```

**Ingress**
```
属性: ingress_name, hosts, tls_enabled, tls_expiry, ingress_class, backend_service
取值范围:
  tls_enabled: true | false
  tls_expiry: "2026-01-01" - "2027-12-31"
  ingress_class: "nginx" | "traefik" | "alb"
```

**ELB**
```
属性: elb_name, elb_type, bandwidth_mbps, listeners, backend_count
取值范围:
  elb_type: "public" | "internal"
  bandwidth_mbps: 10-10000
```

**Gateway**
```
属性: gateway_name, gateway_type, routes_count, rate_limit_enabled
取值范围:
  gateway_type: "SpringCloudGateway" | "Kong" | "APISIX"
  routes_count: 1-100
  rate_limit_enabled: true | false
```

**APIG**
```
属性: apig_name, api_count, qps_limit, auth_type
取值范围:
  api_count: 1-200
  qps_limit: 100-50000
  auth_type: "HMAC" | "JWT" | "OAuth2"
```

### 4.3 中间件类 (Middleware)

**MySQL**
```
属性: instance_name, version, instance_type, storage_gb, connections, connections_max
      qps_avg, slow_query_count, replication_lag_seconds
取值范围:
  version: "5.7" | "8.0" | "8.1"
  connections: 10-1000
  connections_max: 100-2000
  qps_avg: 100-10000
  slow_query_count: 0-100
```

**Redis**
```
属性: instance_name, version, maxmemory, eviction_policy, connected_clients
      hit_rate, evicted_keys, used_memory_percent
取值范围:
  version: "6.0" | "6.2" | "7.0" | "7.2"
  hit_rate: 0.0-1.0
  used_memory_percent: 10.0-95.0
```

**Kafka**
```
属性: cluster_name, version, partitions, replication_factor, retention_hours
      consumer_lag, broker_count, isr_count
取值范围:
  version: "2.8" | "3.0" | "3.4" | "3.7"
  partitions: 1-100
  consumer_lag: 0-100000
  broker_count: 3 | 5 | 7
```

**Nacos**
```
属性: cluster_name, version, mode, nodes, services_registered
      config_count, health_status
取值范围:
  version: "2.1" | "2.2" | "2.3"
  mode: "standalone" | "cluster"
  nodes: 1-7
  services_registered: 1-500
```

### 4.4 存储类 (Storage)

**ConfigMap**
```
属性: configmap_name, data_keys, version_hash, size_bytes
取值范围:
  data_keys: 1-50
  size_bytes: 100-1048576
```

**Secret**
```
属性: secret_name, secret_type, expiry_days, rotation_policy, reference_count
取值范围:
  secret_type: "Opaque" | "kubernetes.io/tls" | "kubernetes.io/dockerconfigjson"
  expiry_days: 1-365
  reference_count: 1-20
```

**ContainerImage**
```
属性: image_name, tag, image_digest, build_time, vulnerability_count
取值范围:
  tag: semantic version | "latest"
  vulnerability_count: 0-50
```

## 5. 数据源服务内部架构

```
┌─────────────────────────────────────────────┐
│            DataSourceService                 │
│                                              │
│  ┌────────────────┐  ┌──────────────────┐   │
│  │  NodeStore      │  │  EdgeStore        │   │
│  │  {id: DataNode} │  │  {id: DataEdge}   │   │
│  └────────────────┘  └──────────────────┘   │
│                                              │
│  ┌────────────────┐  ┌──────────────────┐   │
│  │  MetricStore    │  │  FaultStore       │   │
│  │  {rid: [snaps]} │  │  {id: FaultInj}   │   │
│  └────────────────┘  └──────────────────┘   │
│                                              │
│  ┌──────────────────────────────────────┐   │
│  │  FaultInjector                        │   │
│  │  - inject(fault_type, target_id)      │   │
│  │  - step(seconds)                      │   │
│  │  - apply_stage(stage, node)           │   │
│  └──────────────────────────────────────┘   │
│                                              │
│  ┌──────────────────────────────────────┐   │
│  │  SnapshotGenerator                    │   │
│  │  - 为每个指标生成正常范围随机值        │   │
│  │  - 正常基线: CPU 30-60%, Mem 40-70%   │   │
│  └──────────────────────────────────────┘   │
└─────────────────────────────────────────────┘
```

## 6. 实现计划

### Phase A: 数据源服务核心
- `backend/app/datasource/` — 独立 Python 模块
  - `store.py` — NodeStore, EdgeStore (内存字典)
  - `models.py` — DataNode, DataEdge, MetricSnapshot 数据类
  - `snapshot_generator.py` — 正常指标值生成器
  - `loader.py` — 从 Neo4j 加载基线数据

### Phase B: REST API
- `backend/app/routers/datasource.py` — FastAPI Router
  - 数据提取端点 (GET)
  - 数据注入端点 (PATCH)

### Phase C: 故障注入集成
- `backend/app/datasource/fault_injector.py`
  - 通过 DSS 注入故障（不再直接写 Neo4j）
  - DSS 更新节点状态 → 同步回 Neo4j

### Phase D: 前端适配
- 巡检视图从 DSS API 读取数据
- 故障注入页通过 DSS API 注入故障

## 7. 关键设计决策

1. **DSS 是 Neo4j 之上的内存缓存层**，不是替代 Neo4j
   - Neo4j 存储持久化基线数据
   - DSS 维护实时变化的属性（health, risk, metrics）
   - 启动时从 Neo4j 加载基线，运行时在内存操作，定期同步

2. **故障注入只操作 DSS，不直接操作 Neo4j**
   - 故障数据注入 DSS → DSS 更新节点实时状态
   - DSS 定期或按需同步回 Neo4j
   - reset 时清除故障数据，恢复到 Neo4j 基线

3. **巡检展示系统从 DSS 读取数据**
   - 拓扑视图 → GET /datasource/topology/{app}
   - 节点指标 → GET /datasource/metrics/{id}
   - 保证实时性，无 Neo4j 查询延迟
