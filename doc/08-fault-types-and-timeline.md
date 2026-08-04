# 08 — 故障类型与时间线设计

## 1. 概述

为平台增加**时间维度**和**故障模拟**能力：
- 定义常见云原生故障类型
- 每条故障包含时间线：注入 → 升级 → 传播 → 恢复
- 随时间推进，图数据库 中的节点健康状态和 MetricSnapshot 动态变化
- 前端支持**时间线回放**和**故障影响面推演**

## 2. 数据模型

### 2.1 FaultScenario — 故障场景

```
(:FaultScenario {
  scenario_id: "fault-001",
  name: "订单服务 Pod CPU 飙升",
  fault_type: "resource_cpu_spike",
  severity: "warning" → "critical",
  target_resource_id: "pod:cce-prod-01:order:order-api-6fd9c8b7c9-abcdf",
  status: "pending | injected | escalating | propagating | recovering | resolved",
  injected_at: "2026-06-16T08:00:00Z",
  resolved_at: null,
  duration_seconds: 3600,
  description: "..."
})
```

### 2.2 FaultTimeline — 故障时间线事件

```
(:FaultTimeline {
  timeline_id: "tl-001-01",
  scenario_id: "fault-001",
  sequence: 1,
  timestamp: "2026-06-16T08:00:00Z",
  event_type: "inject | escalate | propagate | recover | resolve",
  affected_resource_id: "pod:...",
  old_health: "normal",
  new_health: "warning",
  old_risk: "low",
  new_risk: "medium",
  metric_name: "cpu_usage",
  old_value: 45.2,
  new_value: 86.5,
  description: "CPU 使用率从 45% 飙升到 86%"
})
```

## 3. 故障类型体系

### 3.1 资源类故障（Resource）

| 类型码 | 名称 | 目标节点 | 症状 | 升级条件 |
|--------|------|---------|------|---------|
| `resource_cpu_spike` | CPU 飙升 | Pod | CPU > 80% | CPU > 95% 持续 5 分钟 |
| `resource_memory_leak` | 内存泄漏 | Pod/Container | 内存持续增长 | OOM Kill |
| `resource_disk_pressure` | 磁盘压力 | KubernetesNode | 磁盘 > 85% | 磁盘 > 95% 触发 Pod Eviction |
| `resource_pid_pressure` | PID 耗尽 | KubernetesNode | PID > 90% | 无法创建新进程 |
| `resource_network_saturation` | 网络带宽饱和 | Pod/Node | 带宽 > 80% | 丢包率 > 5% |

### 3.2 可用性类故障（Availability）

| 类型码 | 名称 | 目标节点 | 症状 | 升级条件 |
|--------|------|---------|------|---------|
| `avail_pod_crashloop` | Pod 频繁重启 | Pod | restart > 5/10min | CrashLoopBackOff |
| `avail_node_notready` | 节点不可用 | KubernetesNode | Node NotReady | 所有 Pod 被驱逐 |
| `avail_deployment_degraded` | 副本降级 | Deployment | 可用副本 < 期望 | 副本归零 |
| `avail_service_no_endpoints` | Service 无后端 | Service | Endpoints = 0 | 流量完全中断 |
| `avail_pod_pending` | Pod 调度失败 | Pod | Pending > 5min | 永远无法调度 |

### 3.3 性能类故障（Performance）

| 类型码 | 名称 | 目标节点 | 症状 | 升级条件 |
|--------|------|---------|------|---------|
| `perf_slow_query` | 慢查询 | MySQL | 响应时间 > 1s | 连接池耗尽 |
| `perf_kafka_lag` | Kafka 消费延迟 | Kafka | Lag > 10000 | Lag > 100000 |
| `perf_redis_eviction` | Redis 内存淘汰 | Redis | evicted_keys 增长 | 命中率 < 80% |
| `perf_high_error_rate` | 错误率飙升 | Pod | 5xx > 2% | 5xx > 10% |

### 3.4 安全类故障（Security）

| 类型码 | 名称 | 目标节点 | 症状 | 升级条件 |
|--------|------|---------|------|---------|
| `sec_cert_expiring` | 证书即将过期 | Secret/Ingress | 剩余 < 14 天 | 剩余 < 3 天 |
| `sec_cve_discovered` | 新漏洞发现 | ContainerImage | CVSS > 7.0 | CVSS > 9.0 |
| `sec_config_drift` | 配置漂移 | ConfigMap | 与基线不一致 | 影响关键配置 |
| `sec_privileged_container` | 特权容器 | Container | privileged=true | 已被利用 |

### 3.5 依赖类故障（Dependency）

| 类型码 | 名称 | 目标节点 | 症状 | 传播 |
|--------|------|---------|------|------|
| `dep_mysql_conn_exhausted` | 连接池耗尽 | MySQL | connections = max | 所有依赖方请求排队 |
| `dep_redis_unavailable` | Redis 不可达 | Redis | 连接超时 | 所有缓存查询穿透到 DB |
| `dep_kafka_broker_down` | Kafka Broker 故障 | Kafka | ISR 缩减 | 消息积压 |
| `dep_nacos_unreachable` | Nacos 不可达 | Nacos | 服务发现失败 | 新节点无法注册 |

### 3.6 级联故障（Cascade）

| 类型码 | 名称 | 描述 |
|--------|------|------|
| `cascade_node_down` | 节点宕机级联 | Node → 所有调度 Pod → Deployment → Component → Application |
| `cascade_secret_expired` | 密钥过期级联 | Secret → Deployment → 所有使用方 Pod |
| `cascade_image_vuln` | 镜像漏洞级联 | ContainerImage → 所有使用该镜像的 Deployment |
| `cascade_network_partition` | 网络分区 | Namespace 间网络中断 → 跨命名空间依赖中断 |

## 4. 时间线推进模型

### 4.1 阶段定义

```
T0: 注入 (inject)       — 故障发生，初始症状
T1: 检测 (detect)       — 监控系统发现（AlertManager firing）
T2: 升级 (escalate)     — 严重程度上升
T3: 传播 (propagate)    — 影响相邻节点
T4: 发现 (find)         — 巡检引擎发现 (InspectionFinding)
T5: 干预 (mitigate)     — 运维介入处理
T6: 恢复 (recover)      — 指标回落
T7: 解决 (resolve)      — 告警清除，状态恢复
```

### 4.2 示例：Pod CPU 飙升

```
时间           阶段       Pod 健康    CPU%    告警        巡检发现
08:00         正常        normal      45%     -           -
08:05 [T0]    注入        warning     86%     firing      -
08:10 [T2]    升级        critical    96%     firing      -
08:15 [T3]    传播        critical    96%     firing      Deployment 副本不一致
08:20 [T4]    发现        critical    96%     firing      CPU 超阈值, 副本降级
08:25 [T5]    干预        warning     75%     firing      -
08:30 [T6]    恢复        normal      48%     resolved    -
08:35 [T7]    解决        normal      45%     -           resolved
```

## 5. 传播规则

故障从源节点沿关系链传播：

```
Pod (CPU spike)
  → 同 Deployment 其他 Pod（负载转移）
  → Deployment（副本不一致风险）
  → ApplicationComponent（组件健康降级）
  → Application（应用健康评分下降）
  → Service（响应延迟增加）

KubernetesNode (NotReady)
  → 所有调度在该节点的 Pod
  → 这些 Pod 所属的 Deployment
  → 这些 Deployment 所属的 Component
  → 这些 Component 所属的 Application
```

传播规则通过 图数据库 图遍历实现，不需要额外建模。

## 6. 前端展示

### 6.1 时间线控件
- 时间进度条（play / pause / speed）
- 时钟显示当前模拟时间
- 节点颜色/健康状态随时间变化

### 6.2 故障注入面板
- 选择故障类型
- 选择目标节点
- 设置严重程度和持续时间
- "注入"按钮 → 图数据库 写入 FaultScenario + FaultTimeline

### 6.3 影响面推演
- 从故障源出发，沿关系链正向/反向遍历
- 高亮受影响节点
- 显示传播路径

## 7. 实现阶段

| Phase | 内容 |
|-------|------|
| Phase A | 故障类型定义 + FaultScenario/FaultTimeline 数据模型 |
| Phase B | 故障注入脚本（选择类型+目标，写入 图数据库） |
| Phase C | 时间线推进引擎（定时更新节点健康状态和指标） |
| Phase D | 前端时间线控件 + 故障注入面板 |
| Phase E | 级联故障推演 + 影响面可视化 |
