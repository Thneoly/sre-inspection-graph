# 06 — REST API 规格

> 后端 API 完整规格定义，后端 API 实现参考。

## 基础信息

- **Base URL**: `http://localhost:8000/api/v1`
- **Content-Type**: `application/json`
- **CORS**: 允许所有来源（开发模式，生产需限制）
- **图数据库**: Bolt 协议，连接池管理

## 通用响应格式

### 图视图响应
```json
{
  "nodes": [
    {
      "id": "app:order",
      "label": "Application",
      "type": "Application",
      "properties": {
        "name": "订单应用",
        "app_code": "order",
        "owner_team": "订单团队",
        "health_status": "warning",
        "risk_level": "medium",
        "sla_level": "P1",
        "inspection_status": "partial"
      }
    }
  ],
  "edges": [
    {
      "id": "e002",
      "source": "app:order",
      "target": "comp:order-api",
      "type": "CONTAINS",
      "properties": {
        "relationship_name": "包含",
        "dependency_strength": "强",
        "health_status": "normal"
      }
    }
  ],
  "summary": {
    "total_nodes": 25,
    "total_edges": 32,
    "risk_counts": {"high": 0, "medium": 3, "low": 12, "unknown": 10},
    "health_counts": {"normal": 15, "warning": 6, "critical": 1, "unknown": 3}
  }
}
```

### 错误响应
```json
{
  "detail": "App not found: app:unknown",
  "error_code": "RESOURCE_NOT_FOUND"
}
```

---

## 端点列表

### 1. 应用拓扑视图

```
GET /topology/app/{app_code}
```

**路径参数**:
| 参数 | 类型 | 说明 |
|------|------|------|
| app_code | string | 应用代码，如 `order` |

**查询参数**:
| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| depth | int | 5 | 遍历深度 (1-10) |

**响应**: `GraphResponse`

---

### 2. 访问链路视图

```
GET /access-link/{app_code}
```

**路径参数**:
| 参数 | 类型 | 说明 |
|------|------|------|
| app_code | string | 应用代码 |

**响应**: `GraphResponse`

---

### 3. 节点影响视图

```
GET /node-impact/{node_id}
```

**路径参数**:
| 参数 | 类型 | 说明 |
|------|------|------|
| node_id | string | 节点 ID，如 `node:cce-prod-01:worker-01` |

**查询参数**:
| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| depth | int | 4 | 遍历深度 (1-10) |

**响应**: `GraphResponse`

---

### 4. 配置影响视图

```
GET /config-impact/{resource_id}
```

**路径参数**:
| 参数 | 类型 | 说明 |
|------|------|------|
| resource_id | string | Secret/ConfigMap 节点 ID |

**响应**: `GraphResponse`

---

### 5. 镜像风险视图

```
GET /image-risk/{image_id}
```

**路径参数**:
| 参数 | 类型 | 说明 |
|------|------|------|
| image_id | string | 容器镜像节点 ID |

**响应**: `GraphResponse`

---

### 6. 告警归并视图

```
GET /alert-aggregation
```

**查询参数**:
| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| severity | string | null | 过滤级别: critical/warning/info |
| status | string | "firing" | 告警状态: firing/resolved/all |
| since | string | null | 起始时间 (ISO 8601) |
| limit | int | 200 | 返回上限 |

**响应**: `GraphResponse`

---

### 7. 资源指标

```
GET /metrics/{resource_id}
```

**路径参数**:
| 参数 | 类型 | 说明 |
|------|------|------|
| resource_id | string | 资源节点 ID |

**查询参数**:
| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| metric_name | string | null | 过滤指定指标 |

**响应**:
```json
{
  "resource_id": "pod:cce-prod-01:order:order-api-xxx",
  "metrics": [
    {
      "id": "snapshot_xxx_cpu_usage_xxx",
      "metric_name": "cpu_usage",
      "current_value": 45.2,
      "unit": "percent",
      "fetched_at": "2026-06-15T10:00:00Z",
      "is_stale": false,
      "warning_breached": false,
      "critical_breached": false,
      "warning_threshold": 80,
      "critical_threshold": 95
    }
  ]
}
```

---

### 8. 资源详情

```
GET /resource/{node_id}
```

**路径参数**:
| 参数 | 类型 | 说明 |
|------|------|------|
| node_id | string | 资源节点 ID |

**响应**:
```json
{
  "node": {
    "id": "pod:cce-prod-01:order:order-api-xxx",
    "label": "Pod",
    "type": "Pod",
    "properties": {
      "name": "order-api-6fd9c8b7c9-abcde",
      "namespace": "order",
      "pod_ip": "10.244.1.23",
      "node_name": "worker-01",
      "phase": "Running",
      "ready": true,
      "restart_count": 1,
      "health_status": "normal",
      "risk_level": "low"
    }
  },
  "metrics": [...],
  "findings": [...],
  "alerts": [...]
}
```

---

### 9. 巡检运行列表

```
GET /inspection/runs
```

**查询参数**:
| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| status | string | null | overall_status: passed/warning/failed |
| limit | int | 20 | 返回数量 |
| offset | int | 0 | 分页偏移 |

**响应**:
```json
{
  "runs": [
    {
      "id": "run-20260615-001",
      "run_name": "生产环境定时巡检 #20260615-001",
      "run_type": "scheduled",
      "overall_status": "passed",
      "started_at": "2026-06-15T10:00:00Z",
      "completed_at": "2026-06-15T10:05:00Z",
      "total_rules": 10,
      "passed_rules": 7,
      "failed_rules": 2,
      "skipped_rules": 1
    }
  ],
  "total": 5
}
```

---

### 10. 巡检发现（按资源）

```
GET /inspection/findings/{resource_id}
```

**路径参数**:
| 参数 | 类型 | 说明 |
|------|------|------|
| resource_id | string | 资源节点 ID |

**查询参数**:
| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| status | string | null | open/acknowledged/resolved/all |

**响应**:
```json
{
  "resource_id": "deploy:cce-prod-01:order:order-api",
  "findings": [
    {
      "id": "finding-run-001-rule-003-deploy",
      "rule_name": "Deployment 副本不一致",
      "severity": "warning",
      "status": "open",
      "description": "期望 3 副本，实际 2 副本可用",
      "detected_at": "2026-06-15T10:03:00Z",
      "recommendation": "检查 Pod 状态，排查 CrashLoopBackOff 或资源不足"
    }
  ]
}
```

---

### 11. 健康检查

```
GET /health
```

**响应**:
```json
{
  "status": "ok",
  "graph": "connected",
  "version": "1.0.0",
  "uptime_seconds": 3600
}
```

---

## Pydantic 模型定义

```
# backend/app/models/graph.py
from pydantic import BaseModel
from typing import Optional, Any
from datetime import datetime

class GraphNode(BaseModel):
    id: str
    label: str
    type: str
    properties: dict[str, Any]

class GraphEdge(BaseModel):
    id: str
    source: str
    target: str
    type: str
    properties: dict[str, Any]

class GraphSummary(BaseModel):
    total_nodes: int
    total_edges: int
    risk_counts: dict[str, int]
    health_counts: dict[str, int]

class GraphResponse(BaseModel):
    nodes: list[GraphNode]
    edges: list[GraphEdge]
    summary: GraphSummary


# backend/app/models/metrics.py
class MetricSnapshotOut(BaseModel):
    id: str
    metric_name: str
    current_value: float
    unit: str
    fetched_at: datetime
    is_stale: bool
    warning_breached: bool
    critical_breached: bool
    warning_threshold: Optional[float] = None
    critical_threshold: Optional[float] = None

class ResourceMetricsResponse(BaseModel):
    resource_id: str
    metrics: list[MetricSnapshotOut]


# backend/app/models/inspection.py
class InspectionFindingOut(BaseModel):
    id: str
    rule_name: str
    severity: str
    status: str
    description: str
    detected_at: datetime
    recommendation: Optional[str] = None

class InspectionFindingsResponse(BaseModel):
    resource_id: str
    findings: list[InspectionFindingOut]

class InspectionRunOut(BaseModel):
    id: str
    run_name: str
    run_type: str
    overall_status: str
    started_at: datetime
    completed_at: Optional[datetime] = None
    total_rules: int
    passed_rules: int
    failed_rules: int
    skipped_rules: int

class InspectionRunsResponse(BaseModel):
    runs: list[InspectionRunOut]
    total: int
```
