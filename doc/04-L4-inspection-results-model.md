# 04 — L4 巡检结果层数据模型

> 定义 L4 巡检结果层的节点类型、关系、属性。巡检结果作为独立节点存储，形成可追溯的巡检历史。

## 1. 设计原则

1. **巡检结果是独立节点**：不以属性形式写在资源节点上，便于追溯和聚合
2. **规则与发现分离**：InspectionRule 定义巡检逻辑，InspectionFinding 记录具体发现
3. **告警是独立事件**：AlertEvent 来自 Prometheus/AlertManager，独立建模
4. **影响传播链**：Finding → Pod → Deployment → ApplicationComponent → Application

## 2. 新增类型节点

### RT-020: InspectionRun

```yaml
node_id: RT-020
node_name: 巡检运行
node_label: InspectionRun
node_group: 巡检
abstraction_level: L6 巡检层
scope: Application/Cluster
lifecycle_type: 记录对象
unique_key: run_id
key_properties: "run_id, run_name, run_type, started_at, completed_at, overall_status"
inspection_focus: "巡检执行状态、覆盖率、执行时长、通过率"
health_fields: "overall_status, pass_rate, execution_duration"
required_relation_summary: "GENERATED InspectionFinding; EXECUTES InspectionRule"
import_label: :ResourceType:InspectionRun
```

### RT-021: InspectionRule

```yaml
node_id: RT-021
node_name: 巡检规则
node_label: InspectionRule
node_group: 巡检
abstraction_level: L6 巡检层
scope: Global/ResourceType
lifecycle_type: 配置对象
unique_key: rule_id
key_properties: "rule_id, rule_name, rule_category, severity, applies_to_resource_type"
inspection_focus: "规则覆盖完整性、规则有效性、阈值合理性"
health_fields: "enabled_status, last_executed_at, hit_rate"
required_relation_summary: "APPLIES_TO ResourceType; GENERATES InspectionFinding"
import_label: :ResourceType:InspectionRule
```

### RT-022: InspectionFinding

```yaml
node_id: RT-022
node_name: 巡检发现
node_label: InspectionFinding
node_group: 巡检
abstraction_level: L6 巡检层
scope: ResourceInstance
lifecycle_type: 记录对象
unique_key: finding_id
key_properties: "finding_id, severity, status, affected_resource_id, detected_at"
inspection_focus: "发现数量、严重程度分布、修复时效、误报率"
health_fields: "status, severity, time_to_resolve"
required_relation_summary: "FOUND_IN InspectionRun; VIOLATES InspectionRule; AFFECTS ResourceInstance"
import_label: :ResourceType:InspectionFinding
```

### RT-023: AlertEvent

```yaml
node_id: RT-023
node_name: 告警事件
node_label: AlertEvent
node_group: 可观测
abstraction_level: L5 观测层
scope: ResourceInstance
lifecycle_type: 事件对象
unique_key: alert_event_id
key_properties: "alert_event_id, alert_name, severity, status, fired_at"
inspection_focus: "告警数量、告警归并、误报噪声、响应时效"
health_fields: "status, severity, duration"
required_relation_summary: "FIRED_ON ResourceInstance; AGGREGATES_TO Application"
import_label: :ResourceType:AlertEvent
```

## 3. 新增类型关系

| edge_id | source | rel_type | target | dependency_strength | auto_discovery |
|---------|--------|----------|--------|---------------------|----------------|
| REL-042 | InspectionRun | GENERATED | InspectionFinding | 强 | 自动 |
| REL-043 | InspectionFinding | VIOLATES | InspectionRule | 强 | 自动 |
| REL-044 | InspectionFinding | AFFECTS | ResourceInstance | 强 | 自动 |
| REL-045 | InspectionFinding | PROPAGATES_TO | ResourceInstance | 中 | 推导 |
| REL-046 | AlertEvent | FIRED_ON | ResourceInstance | 强 | 自动 |
| REL-047 | AlertEvent | AGGREGATES_TO | Application | 中 | 推导 |

### 关系详情

#### REL-042: InspectionRun GENERATED InspectionFinding
- **目的**: 一次巡检运行产生一组发现
- **巡检检查项**: 巡检是否完整执行；是否有规则未执行
- **风险信号**: 巡检运行失败；大量新发现
- **发现方式**: 巡检引擎回调

#### REL-043: InspectionFinding VIOLATES InspectionRule
- **目的**: 发现命中了哪条规则
- **巡检检查项**: 规则命中率；高命中率规则是否需要调整阈值

#### REL-044: InspectionFinding AFFECTS ResourceInstance
- **目的**: 发现关联到具体资源
- **风险信号**: 核心资源关联高危发现
- **影响方向**: Finding → Resource → upstream to Application

#### REL-045: InspectionFinding PROPAGATES_TO ResourceInstance
- **目的**: 影响传播链（Finding 影响 Pod → 影响 Deployment → 影响 Component）
- **发现方式**: 图遍历推导（沿 SCHEDULED_ON / DEPLOYED_AS / CONTAINS 反向传播）

#### REL-046: AlertEvent FIRED_ON ResourceInstance
- **目的**: 告警触发在某个具体资源上
- **风险信号**: 同一资源多个告警；同一应用多个资源告警

#### REL-047: AlertEvent AGGREGATES_TO Application
- **目的**: 告警按应用归并
- **发现方式**: 从 FIRED_ON 的资源沿关系链向上推导到 Application

## 4. InspectionRule 巡检规则定义

### 推荐的 10 个核心巡检规则

| rule_id | rule_name | category | severity | applies_to | description |
|---------|-----------|----------|----------|------------|-------------|
| rule-001 | Pod CPU 使用率过高 | resource | warning | Pod | CPU 使用率超过 80% 阈值 |
| rule-002 | Pod 频繁重启 | availability | critical | Pod | 24h 内重启超过 10 次 |
| rule-003 | Deployment 副本不一致 | availability | critical | Deployment | 期望副本与可用副本不一致 |
| rule-004 | Secret 即将过期 | security | warning | Secret | 证书/密钥在 30 天内过期 |
| rule-005 | 镜像存在高危漏洞 | security | critical | ContainerImage | 镜像存在 Critical 级别 CVE |
| rule-006 | Service 无后端 | availability | critical | Service | Service 选择器无匹配 Pod |
| rule-007 | Ingress TLS 即将过期 | security | warning | Ingress | TLS 证书在 14 天内过期 |
| rule-008 | 节点资源压力 | resource | warning | KubernetesNode | 节点 CPU/内存/磁盘压力 |
| rule-009 | ConfigMap 配置漂移 | config | warning | ConfigMap | 与基线版本不一致 |
| rule-010 | 容器以 root 运行 | security | critical | Container | 安全上下文 allowPrivilegeEscalation |

## 5. InspectionFinding 实例属性

```yaml
InspectionFinding:
  id: "finding_{run_id}_{rule_id}_{resource_id}"
  run_id: "run-20260615-001"
  rule_id: "rule-001"
  rule_name: "Pod CPU 使用率过高"
  rule_category: "resource"
  severity: "warning | critical"
  status: "open | acknowledged | resolved | false_positive"
  affected_resource_id: "pod:prod-k8s-01:order:order-api-xxx"
  affected_resource_type: "Pod"
  affected_resource_name: "order-api-xxx"
  description: "Pod order-api-xxx CPU 使用率 86.5% 超过 80% 阈值"
  evidence:
    current_value: 86.5
    threshold: 80
    unit: "percent"
    measured_at: "2026-06-15T10:03:00Z"
  detected_at: "2026-06-15T10:03:00Z"
  resolved_at: null
  assigned_to: "SRE"
  recommendation: "检查应用是否有性能问题；考虑增加 CPU 限制或水平扩展"
```

## 6. InspectionRun 实例属性

```yaml
InspectionRun:
  id: "run-20260615-001"
  run_name: "生产环境定时巡检 #20260615-001"
  run_type: "scheduled | manual | ad_hoc"
  scope: "prod/order"
  started_at: "2026-06-15T10:00:00Z"
  completed_at: "2026-06-15T10:05:00Z"
  duration_seconds: 300
  total_rules: 10
  passed_rules: 7
  failed_rules: 2
  skipped_rules: 1
  overall_status: "passed | warning | failed"
  triggered_by: "cron:0 */6 * * *"
```

## 7. AlertEvent 实例属性

```yaml
AlertEvent:
  id: "alert_{alertmanager_fingerprint}"
  alert_name: "OrderAPIHighErrorRate"
  severity: "critical | warning | info"
  status: "firing | resolved"
  fired_at: "2026-06-15T09:45:00Z"
  resolved_at: null
  prometheus_alert_id: "abc123def456"
  summary: "订单 API 错误率超过 5%"
  description: "order-api 组件错误率 7.2% 超过 critical 阈值 5%，持续 5 分钟"
  affected_labels:
    namespace: "order"
    pod: "order-api-6fd9c8b7c9-abcde"
    deployment: "order-api"
    component: "order-api"
    application: "order"
  silence_url: "https://alertmanager.example.com/#/silences/new?..."
  dashboard_url: "https://grafana.example.com/d/order-api?var-namespace=order"
```

## 8. 实例关系（L4 层 mock 数据）

### 巡检发现关系图

```
run-20260615-001 (InspectionRun, status=passed)
  ├─ GENERATED → finding-run-001-rule-003-deploy (replicas mismatch, warning, open)
  │    ├─ VIOLATES → rule-003 (Deployment 副本不一致)
  │    ├─ AFFECTS → deploy:cce-prod-01:order:order-api
  │    └─ PROPAGATES_TO → comp:order-api (推导)
  │
  ├─ GENERATED → finding-run-001-rule-004-secret (secret expiry, warning, open)
  │    ├─ VIOLATES → rule-004 (Secret 即将过期)
  │    └─ AFFECTS → secret:cce-prod-01:order:order-api-secret
  │
  └─ GENERATED → finding-run-001-rule-005-image (critical vuln, critical, open)
       ├─ VIOLATES → rule-005 (镜像存在高危漏洞)
       └─ AFFECTS → image:order-api:1.2.3

run-20260615-002 (InspectionRun, status=warning)
  ├─ GENERATED → finding-run-002-rule-001-pod1 (high cpu, warning, open)
  └─ GENERATED → finding-run-002-rule-002-pod2 (restart loop, critical, open)
```

### 告警事件关系图

```
alert:OrderAPIHighErrorRate (AlertEvent, severity=critical, status=firing)
  ├─ FIRED_ON → comp:order-api (ApplicationComponent)
  └─ AGGREGATES_TO → app:order

alert:PodRestartLoop (AlertEvent, severity=warning, status=firing)
  ├─ FIRED_ON → pod:cce-prod-01:order:order-api-{hash2}
  ├─ PROPAGATES_TO → deploy:cce-prod-01:order:order-api
  └─ AGGREGATES_TO → app:order

alert:NodeDiskPressure (AlertEvent, severity=warning, status=firing)
  ├─ FIRED_ON → node:cce-prod-01:worker-02
  └─ AFFECTS → pod:cce-prod-01:order:order-api-{hash2} (调度的 Pod)
```

## 9. L4 查询示例

### 查询某次巡检的所有发现及其影响
```cypher
MATCH (run:ResourceInstance:InspectionRun {id: "run-20260615-001"})
MATCH (run)-[:GENERATED]->(finding:InspectionFinding)
MATCH (finding)-[:AFFECTS]->(resource:ResourceInstance)
OPTIONAL MATCH (finding)-[:VIOLATES]->(rule:InspectionRule)
RETURN run, finding, resource, rule
```

### 查询某个应用的所有未关闭风险
```cypher
MATCH (app:ResourceInstance:Application {id: "app:order"})
MATCH path = (finding:InspectionFinding {status: "open"})-[:AFFECTS|PROPAGATES_TO*1..4]->(resource:ResourceInstance)-[:BELONGS_TO|CONTAINS*1..4]->(app)
RETURN finding, resource, path
```

### 查询某个节点的所有告警和影响
```cypher
MATCH (node:ResourceInstance:KubernetesNode {id: "node:cce-prod-01:worker-01"})
MATCH (pod:Pod)-[:SCHEDULED_ON]->(node)
MATCH (alert:AlertEvent)-[:FIRED_ON]->(pod)
OPTIONAL MATCH (pod)-[:BELONGS_TO|CONTAINS*1..4]->(app:Application)
RETURN node, pod, alert, app
```
