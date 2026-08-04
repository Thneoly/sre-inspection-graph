# 03 — L3 动态观测层数据模型

> 定义 L3 层的节点类型、关系、属性。L3 是新设计的核心层，桥接静态资源实例（L2）和动态指标/告警。

## 1. 设计原则

1. **图数据库 存摘要，Prometheus 存明细**：图数据库 只保存最新指标快照和查询模板，时序数据留在 Prometheus
2. **关系建模状态**：Pod/Container/KubernetesNode 是运行态对象，属性会频繁更新
3. **可扩展**：新指标类型只需增加 MetricQuery + MetricSnapshot，不改变图结构

## 2. 新增类型节点

### RT-015: Pod

```yaml
node_id: RT-015
node_name: Pod
node_label: Pod
node_group: 运行态
abstraction_level: L4 运行层
scope: Namespace/Node
lifecycle_type: 动态对象
unique_key: cluster_id + namespace + pod_name
key_properties: "cluster_id, namespace, pod_name, pod_ip, host_ip, node_name, phase, ready, restart_count, owner_kind, owner_name"
inspection_focus: "Pod phase, restart count, resource usage, scheduling status, health probes"
health_fields: "phase, ready, restart_count, health_status, risk_level"
required_relation_summary: "SCHEDULED_ON KubernetesNode; RUNS Container; BELONGS_TO Namespace; CONTROLLED_BY Deployment/StatefulSet"
import_label: :ResourceType:Pod
```

### RT-016: Container

```yaml
node_id: RT-016
node_name: 容器
node_label: Container
node_group: 运行态
abstraction_level: L4 运行层
scope: Pod
lifecycle_type: 动态对象
unique_key: cluster_id + namespace + pod_name + container_name
key_properties: "container_name, image, image_digest, cpu_request, memory_request, cpu_limit, memory_limit"
inspection_focus: "Image pull status, OOM kills, CPU/memory throttling, readiness probe"
health_fields: "ready, restart_count, cpu_usage_pct, memory_usage_pct"
required_relation_summary: "RUNS_IN Pod; USES ContainerImage"
import_label: :ResourceType:Container
```

### RT-017: KubernetesNode

```yaml
node_id: RT-017
node_name: Kubernetes节点
node_label: KubernetesNode
node_group: 平台位置
abstraction_level: L3 平台层
scope: Cluster
lifecycle_type: 半稳定对象
unique_key: cluster_id + node_name
key_properties: "cluster_id, node_name, node_ip, instance_type, cpu_capacity, memory_capacity, kernel_version, kubelet_version"
inspection_focus: "Node readiness, resource pressure (CPU/Memory/Disk/PID), kernel vulnerabilities, disk pressure"
health_fields: "node_status, cpu_pressure, memory_pressure, disk_pressure, pid_pressure"
required_relation_summary: "BELONGS_TO KubernetesCluster; SCHEDULES Pod"
import_label: :ResourceType:KubernetesNode
```

### RT-018: MetricQuery

```yaml
node_id: RT-018
node_name: 指标查询模板
node_label: MetricQuery
node_group: 可观测
abstraction_level: L5 观测层
scope: Global/ResourceType
lifecycle_type: 配置对象
unique_key: query_id
key_properties: "query_id, metric_name, promql_template, target_resource_type, datasource_uid"
inspection_focus: "PromQL validity, datasource availability, label consistency, threshold configuration"
health_fields: "enabled_status, datasource_status, last_validated_at"
required_relation_summary: "HAS_METRIC ResourceType; SNAPSHOTS_TO MetricSnapshot"
import_label: :ResourceType:MetricQuery
```

### RT-019: MetricSnapshot

```yaml
node_id: RT-019
node_name: 指标快照
node_label: MetricSnapshot
node_group: 可观测
abstraction_level: L5 观测层
scope: ResourceInstance
lifecycle_type: 动态对象（latest-N）
unique_key: resource_id + metric_name + fetched_at
key_properties: "metric_name, current_value, unit, fetched_at, is_stale, threshold_breached"
inspection_focus: "Value thresholds, staleness, trend deviation from baseline"
health_fields: "is_stale, warning_breached, critical_breached"
required_relation_summary: "MEASURES ResourceInstance; SNAPSHOTTED_BY MetricQuery"
import_label: :ResourceType:MetricSnapshot
```

## 3. 新增类型关系

| edge_id | source | rel_type | target | dependency_strength | auto_discovery |
|---------|--------|----------|--------|---------------------|----------------|
| REL-035 | Deployment | CONTAINS | Pod | 强 | 自动 |
| REL-036 | Pod | RUNS | Container | 强 | 自动 |
| REL-037 | Pod | SCHEDULED_ON | KubernetesNode | 强 | 自动 |
| REL-038 | KubernetesNode | BELONGS_TO | KubernetesCluster | 强 | 自动 |
| REL-039 | ResourceType | HAS_METRIC | MetricQuery | 中 | 半自动 |
| REL-040 | MetricQuery | SNAPSHOTS_TO | MetricSnapshot | 中 | 自动 |
| REL-041 | MetricSnapshot | MEASURES | ResourceInstance | 中 | 自动 |

### 关系详情

#### REL-035: Deployment CONTAINS Pod
- **目的**: Deployment 管理 Pod 副本（通过 ReplicaSet）
- **巡检检查项**: Pod 数量是否与期望副本数一致；是否存在 CrashLoopBackOff；Restart 次数是否超过阈值
- **风险信号**: Pod 数量不足；频繁重启；Pending 超过 N 分钟
- **影响方向**: Deployment → Pod → Container
- **发现方式**: Kubernetes API 标签匹配

#### REL-036: Pod RUNS Container
- **目的**: Pod 内含一个或多个容器
- **巡检检查项**: 容器是否 Ready；镜像拉取是否成功；OOM Kill 次数
- **风险信号**: ContainerCreating 超过阈值；ImagePullBackOff；OOMKilled
- **影响方向**: Pod → Container

#### REL-037: Pod SCHEDULED_ON KubernetesNode
- **目的**: Pod 被调度到特定节点
- **巡检检查项**: 节点资源是否充足；是否有亲和性/反亲和性冲突
- **风险信号**: 节点不可用影响所有 Pod；节点磁盘压力导致 Pod Eviction

#### REL-038: KubernetesNode BELONGS_TO KubernetesCluster
- **目的**: 节点是集群成员
- **巡检检查项**: 节点状态 Ready/NotReady；节点组/可用区分布

## 4. MetricQuery 指标定义

### 推荐的 7 个核心指标查询

| query_id | metric_name | target_resource_type | promql_template | unit | warning_threshold | critical_threshold |
|----------|-------------|----------------------|-----------------|------|-------------------|--------------------|
| mq-cpu-usage | cpu_usage | Pod | `sum(rate(container_cpu_usage_seconds_total{namespace="{{namespace}}",pod="{{pod}}"}[5m])) * 100` | percent | 80 | 95 |
| mq-memory-usage | memory_usage | Pod | `container_memory_working_set_bytes{namespace="{{namespace}}",pod="{{pod}}"}` | bytes | 80% of limit | 95% of limit |
| mq-qps | qps | Pod | `sum(rate(http_requests_total{namespace="{{namespace}}",pod="{{pod}}"}[5m]))` | requests/s | — | — |
| mq-error-rate | error_rate | Pod | `sum(rate(http_requests_total{namespace="{{namespace}}",pod="{{pod}}",status=~"5.."}[5m])) / sum(rate(http_requests_total{namespace="{{namespace}}",pod="{{pod}}"}[5m]))` | fraction | 0.01 | 0.05 |
| mq-restart-count | restart_count | Pod | `kube_pod_container_status_restarts_total{namespace="{{namespace}}",pod="{{pod}}"}` | count | 3 | 10 |
| mq-node-cpu | node_cpu_usage | KubernetesNode | `100 - (avg(rate(node_cpu_seconds_total{mode="idle",node="{{node_name}}"}[5m])) * 100)` | percent | 80 | 95 |
| mq-node-memory | node_memory_usage | KubernetesNode | `(1 - node_memory_MemAvailable_bytes{node="{{node_name}}"} / node_memory_MemTotal_bytes{node="{{node_name}}"}) * 100` | percent | 80 | 95 |

## 5. Pod 实例节点属性

```yaml
Pod:
  id: "cluster_id/namespace/pod_name"           # 图数据库唯一 ID
  name: "pod_name"
  uid: "kubernetes_uid"                         # K8s 原生 UID
  cluster_id: "cluster_id"
  namespace: "namespace"
  pod_ip: "10.244.1.x"
  host_ip: "10.10.1.x"
  node_name: "node_name"
  phase: "Running | Pending | Failed | Succeeded | Unknown"
  ready: true | false
  restart_count: 0
  owner_kind: "ReplicaSet | Job | StatefulSet | DaemonSet"
  owner_name: "deployment_name-xxxxx"
  created_at: "2026-06-15T08:00:00Z"
  last_seen_time: "2026-06-15T10:00:00Z"
  health_status: "normal | warning | critical"
  risk_level: "low | medium | high | critical"
  metric_source: "Prometheus"
  log_source: "Loki"
  cpu_usage_percent: 45.2                      # 最新快照同步值
  memory_usage_percent: 62.8
  restart_count_24h: 1
```

## 6. Container 实例节点属性

```yaml
Container:
  id: "cluster_id/namespace/pod_name/container_name"
  name: "container_name"
  image: "registry.example.com/order/order-api:v1.2.3"
  image_digest: "sha256:abc123..."
  cpu_request: "500m"
  memory_request: "512Mi"
  cpu_limit: "2000m"
  memory_limit: "2048Mi"
  cpu_usage_percent: 45.2
  memory_usage_percent: 62.8
  restart_count: 0
  ready: true
  ports: "[8080]"
  volume_mounts: "[{\"name\":\"config\",\"mountPath\":\"/app/config\"}]"
```

## 7. KubernetesNode 实例节点属性

```yaml
KubernetesNode:
  id: "cluster_id/node_name"
  name: "node_name"
  node_ip: "10.10.1.x"
  instance_type: "c6.2xlarge"
  cpu_capacity: "8"
  memory_capacity: "32Gi"
  pod_capacity: 110
  kernel_version: "5.15.0-1025-aws"
  kubelet_version: "v1.29.3"
  node_status: "Ready | NotReady | Unknown"
  conditions: "[{\"type\":\"MemoryPressure\",\"status\":\"False\"}, ...]"
  cpu_usage_percent: 65.0
  memory_usage_percent: 72.0
  disk_usage_percent: 58.0
  health_status: "normal | warning | critical"
  created_at: "2025-01-15T00:00:00Z"
```

## 8. MetricSnapshot 实例属性

```yaml
MetricSnapshot:
  id: "snapshot_{resource_id}_{metric_name}_{timestamp}"
  metric_name: "cpu_usage | memory_usage | qps | error_rate | restart_count | ..."
  current_value: 45.2
  unit: "percent | bytes | requests/s | fraction | count"
  fetched_at: "2026-06-15T10:00:00Z"
  ttl_seconds: 300
  is_stale: false
  warning_breached: false
  critical_breached: false
  resource_id: "pod:prod-k8s-01:order:order-api-xxx"
  metric_query_id: "mq-cpu-usage"
```

## 9. 实例关系（L3 层 mock 数据）

基于 order-api 示例扩展：

```
deploy:cce-prod-01:order:order-api
  └─ CONTAINS → pod:cce-prod-01:order:order-api-{hash1}
  └─ CONTAINS → pod:cce-prod-01:order:order-api-{hash2}
  └─ CONTAINS → pod:cce-prod-01:order:order-api-{hash3}

pod:... {hash1}
  ├─ RUNS → container:...order-api
  ├─ SCHEDULED_ON → node:...worker-01
  ├─ MEASURED_BY → snapshot:cpu:...
  └─ MEASURED_BY → snapshot:memory:...

node:...worker-01
  ├─ BELONGS_TO → cluster:cce-prod-01
  ├─ SCHEDULES → pod:...{hash1}
  ├─ SCHEDULES → pod:...{hash4}  (另一个应用的 Pod)
  └─ MEASURED_BY → snapshot:node_cpu:...
```
