# 01 — 需求概述与平台目标

## 1. 平台定位

**云原生 SRE 巡检图谱平台**（Cloud Native SRE Inspection Graph Platform）

面向 SRE 工程师和平台运维人员，提供以**知识图谱**为核心的云原生资源对象管理与巡检能力，解决以下痛点：

1. **资源对象碎片化**：不同数据源（gaia/wesee/roma）对同一资源的定义、粒度、关系不同，缺少统一标准
2. **巡检停留在资源类型层**：只知道"Deployment 应该关联 Service"，不知道"具体哪个 Pod 异常影响了哪个业务应用"
3. **动态指标与静态关系割裂**：CPU/内存/QPS 等指标在 Prometheus，资源关系在 CMDB，排查时需要跨多个系统拼凑上下文
4. **巡检结果缺少关联分析**：发现一个镜像漏洞后，无法快速确定影响哪些 Deployment/Pod/应用

## 2. 核心原则

> 图数据库 负责把"具体资源对象、资源关系、当前状态、风险结果、指标查询入口"关联起来；真正的动态明细数据留在 Prometheus、日志系统和巡检结果库里，前端展示时按需联动查询。

| 存储位置 | 存什么 |
|----------|--------|
| **图数据库 图数据库** | 资源身份、归属、关系、当前状态摘要、风险等级、指标查询入口、巡检结果、告警关联 |
| **Prometheus / VictoriaMetrics** | CPU/内存/网络/磁盘/QPS/错误率/延迟等时序指标明细 |
| **Loki / ELK** | 应用日志、容器日志、审计日志、事件日志 |
| **对象存储 / 关系库** | 巡检报告、历史快照、证据文件 |

## 3. 四层模型总览

```
L1 资源类型层 Type Graph
    → 定义资源类型之间的标准关系（14 类，35 关系）

L2 资源实例层 Instance Graph
    → 记录真实环境里的具体资源对象及其关系

L3 动态观测层 Metric / Log / Event
    → 记录指标查询模板、最新值快照、告警事件

L4 巡检结果层 Inspection Graph
    → 记录巡检运行、规则、发现、影响范围和处置建议
```

```
资源标准主数据 v1
        ↓
资源类型图谱 (L1)
        ↓
真实资源实例图谱 (L2)
        ↓
接入 Prometheus / 日志 / 事件 / 巡检结果 (L3)
        ↓
形成可观察、可排障、可影响分析的巡检图谱 (L4)
```

### 3.1 横切层(规划中,PRD-005 引入)

四层模型刻画的是**数据的内容分层**;PRD-005 在数据**采集与合并**这一维引入横切层,所有 L2 / L3 / L4 数据通过它流入 内存孪生层:

```
┌──────────────────────────────────────────────────────────────┐
│ N 个 Connector (K8s / Cloud API / Trace / 代码仓 / GitOps...) │
└────────────────────────────┬─────────────────────────────────┘
                             ▼
                  ┌──────────────────────┐
                  │  Fact 总线 + Identity │  ← PRD-005 新增
                  │  Resolver(横切层)    │
                  └──────────┬───────────┘
                             ▼
                  ┌──────────────────────┐
                  │  Canonical Graph     │
                  │  (内存孪生层 + 图数据库)       │
                  └──────────────────────┘
                  ↑       ↑       ↑     ↑
                  L1     L2      L3    L4
                  类型   实例    观测  巡检
```

PRD-006(代码仓数据源)是这一横切层的具体消费者,贡献节点元数据(`CodeRepo`)、构建映射(`BUILDS`)、PR/MR 事件(扩 `ChangeEvent`)、业务规则(自动抽取 `InspectionRule`)。详见 `doc/11-PRD-005-...` 和 `doc/12-PRD-006-...`。

## 4. 技术选型

| 组件 | 技术 | 理由 |
|------|------|------|
| 图数据库 | 图数据库 5 Community (Docker) | 原生图存储，图查询 查询，成熟生态 |
| 后端 | Rust + 后端 API | 异步高性能，图数据库 官方驱动，AI/数据场景首选 |
| 前端 | React 18 + TypeScript + Vite | 组件化，类型安全，开发体验好 |
| 图可视化 | Cytoscape.js + dagre 布局 | 专为图数据设计，分层布局适合拓扑展示 |
| 状态管理 | TanStack Query (React Query) | 服务端状态缓存，自动刷新 |
| 部署 | Docker Compose | 一键启动 图数据库 + API + 前端 |
| 数据模拟 | 脚本 (csv + 图查询 输出) | 可复现、可版本控制 |

## 5. 用户角色

- **SRE 工程师**：日常巡检，故障排查，影响面分析，告警归并
- **平台运维**：集群健康度监控，版本管理，配置审计
- **安全工程师**：镜像漏洞影响面，Secret 过期风险，TLS 证书管理

## 6. 非功能需求

| 需求 | 指标 |
|------|------|
| 图查询响应时间 | < 2s（200 节点内） |
| 单视图节点上限 | 200 节点（图查询 LIMIT） |
| 前端首屏加载 | < 3s |
| 部署启动时间 | < 5s（tauri dev 冷启动） |
| 数据可复现 | 一键 mock 数据生成，CSV 版本化管理 |

## 7. 推荐展示形态

```
┌──────────┐  ┌─────────────────────────┐  ┌──────────────────┐
│ 左侧      │  │ 中间                     │  │ 右侧              │
│ 资源树    │  │ 图谱拓扑                  │  │ 节点详情           │
│ 应用列表  │  │                         │  │ - 基础信息         │
│          │  │ Application             │  │ - 实时指标         │
│ □ order  │  │   → Component           │  │ - 巡检结果         │
│ □ user   │  │     → Deployment        │  │ - 告警事件         │
│ □ pay    │  │       → Pod → Container │  │                   │
│          │  │                         │  │                   │
├──────────┤  ├─────────────────────────┤  └──────────────────┘
│          │  │ 底部：指标趋势 / 巡检结果 / 告警事件              │
│          │  │ CPU ▁▂▃▄▅▆▇  Memory ▁▂▃▄▅▆  Restart ▁▁▁▃▇  │
└──────────┘  └─────────────────────────────────────────────┘
```

## 8. 数据范围

本阶段实现基于现有 `resource_type.csv`（gaia/wesee/roma 三源数据的整合产物）和已定义的 L1/L2 模型（14 类型节点 + 35 关系），扩展 L3/L4 层，使用**模拟数据**（基于 order-api 示例应用）验证全链路。

## 9. 与上游数据源的对接规划

数据对接经历了三个阶段:

### 9.1 v0 — 静态 CSV(已完成)
L1-L4 全链路验证见 `doc/` 各视图 + 真集群(otel-demo)同步。

### 9.2 v1 — 单源 connector(PRD-004,已完成)
为 OTel Demo 集群接入 6 个真实数据 connector(K8s / Prometheus / Jaeger / flagd / K8s-events / K8s-watch),走 BaseConnector 框架 30s 轮询写 内存孪生层:

| 数据来源 | 采集方式 | 目标层 | 状态 |
|----------|----------|--------|------|
| Kubernetes API | `kubernetes-asyncio` list/watch → k8s_connector | L2 实例图 | ✅ |
| Prometheus(OTel spanmetrics) | HTTP `/api/v1/query` → MetricSnapshot + AlertEvent | L3 指标 + 告警 | ✅ |
| Jaeger trace | `/api/traces` → ChildOf span 聚合 CALLS 边 | L2 调用关系 | ✅ |
| flagd | gRPC ResolveAll → flag diff → ChangeEvent | L3 变更事件 | ✅ |
| K8s events / watch | events 轮询 + watch 长连接 → ChangeEvent | L3 变更事件 | ✅ |
| ArgoCD / Harbor | webhook → ChangeEvent | L3 变更事件 | ✅ |

### 9.3 v2 — 统一拓扑感知(PRD-005 + PRD-006,规划中)

v1 把 6 个 connector 写 内存孪生层 的"管道"打通了,但**每个 connector 直接 `store.upsert_node()`**,多源数据无合并机制、集群外资产靠手工建节点兜底、trace 看到的外部依赖永久丢失。v2 引入横切层解决:

| 数据来源 | 采集方式 | 目标层 | 状态 |
|----------|----------|--------|------|
| 云厂商 API(华为云 RDS / DMS / ELB ...) | SDK 30s 轮询 → Cloud connector | L2 集群外资产 | 📋 PRD-005 S4 |
| 服务注册中心(Nacos / Consul) | Open API → Config plane connector | L2 服务实例 | 📋 PRD-005 S5 |
| 网关控制面(Kong / APISIX) | Admin API → Gateway connector | L2 路由 / L3 路由变更 | 📋 PRD-005 S5 |
| GitOps(ArgoCD CR / Terraform tfstate) | API + state 文件 → 声明意图 | L4 intent-drift finding | 📋 PRD-005 S6 |
| **代码仓**(GitLab / GitHub) | Open API + webhook → code_repo_connector | **L2 CodeRepo + L4 业务规则** | 📋 PRD-006 |
| Trace 增强(OTel span attrs) | 现有 Jaeger connector 升级,挖 `db.system` / `messaging.system` / `peer.service` | L2 客户端嵌入依赖 | 📋 PRD-005 S2 |
| 镜像扫描平台(Harbor/Trivy) | API → ContainerImage 漏洞属性 | L2 镜像节点 | 📋 后续 |
| 日志系统(Loki/ELK) | 查询入口注入(已有 log_source 字段) | L3 日志关联 | 📋 后续 |

**演进的核心是把"connector 直接写 内存孪生层"改成"connector 发 Fact → Identity Resolver 合并 → 内存孪生层",并新增 Unknown Dependency Queue 用 trace 做拓扑完整度自检**。详见 `doc/11-PRD-005-universal-topology-service.md`。
