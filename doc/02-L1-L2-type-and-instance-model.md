# 02 — L1/L2 资源类型与实例模型

> 本文档化已有的 L1 资源类型图谱和 L2 资源实例图谱，数据来源于 `data/cloud_native_app_inspection_graph_import_package_v1/`。

## 1. L1 资源类型节点（14 类）

L1 层定义云原生运维领域的标准资源类型及其元数据。每个类型节点包含巡检关注点和健康字段定义。

| node_id | node_name | node_label | node_group | scope | lifecycle_type | inspection_focus |
|---------|-----------|------------|------------|-------|----------------|-------------------|
| RT-001 | 业务环境 | Environment | 业务归属 | Tenant/Organization | 稳定对象 | 划定巡检边界；区分生产/测试/开发；环境级风险汇总 |
| RT-002 | 应用 | Application | 业务归属 | Tenant/Environment | 稳定对象 | 应用健康评分、责任归属、SLO/告警/仪表盘覆盖检查 |
| RT-003 | 应用组件 | ApplicationComponent | 业务归属 | Application | 稳定对象 | 组件级巡检入口；Deployment/Service/配置/镜像/告警归并 |
| RT-004 | Kubernetes集群 | KubernetesCluster | 平台位置 | Cluster | 稳定对象 | 集群版本、控制面健康、节点资源、命名空间承载情况 |
| RT-005 | 命名空间 | Namespace | 平台位置 | Cluster | 稳定对象 | 隔离边界、资源配额、命名规范、孤儿命名空间检查 |
| RT-006 | Deployment | Deployment | 工作负载 | Namespace | 稳定对象 | 副本一致性、发布策略、资源限制、镜像和配置引用 |
| RT-007 | Service | Service | 服务访问 | Namespace | 稳定对象 | 服务发现、端口暴露、后端是否存在、访问链路完整性 |
| RT-008 | Ingress | Ingress | 服务访问 | Namespace/Cluster | 稳定对象 | 公网/内网入口、路由规则、TLS证书、后端 Service 可达性 |
| RT-009 | ConfigMap | ConfigMap | 配置安全 | Namespace | 配置对象 | 配置完整性、变更影响、敏感信息误放、过期配置检查 |
| RT-010 | Secret | Secret | 配置安全 | Namespace | 配置对象 | 密钥有效期、轮换策略、引用范围、明文/弱加密风险 |
| RT-011 | 镜像仓库 | ContainerRegistry | 镜像制品 | Global/Region | 稳定对象 | 仓库可用性、访问权限、镜像扫描策略、仓库配额 |
| RT-012 | 容器镜像 | ContainerImage | 镜像制品 | Registry/Repository | 半稳定对象 | 镜像漏洞、latest 标签、过期镜像、镜像来源可信度 |
| RT-013 | 告警规则 | AlertRule | 可观测 | Application/Component/Cluster | 配置对象 | 告警覆盖、阈值合理性、责任人、通知通道 |
| RT-014 | 仪表盘 | Dashboard | 可观测 | Application/Component | 配置对象 | 是否覆盖核心应用/组件；数据源是否可用；面板是否过期 |

### 节点属性模板

| 属性 | 说明 |
|------|------|
| node_id | 唯一标识（RT-001 ~ RT-014） |
| node_name | 中文名称 |
| node_label | 英文标签（Neo4j 节点标签） |
| node_group | 分组（业务归属/平台位置/工作负载/服务访问/配置安全/镜像制品/可观测） |
| abstraction_level | 抽象层级（L0~L7） |
| scope | 作用域 |
| lifecycle_type | 稳定对象 / 半稳定对象 / 配置对象 |
| unique_key | 唯一键（由哪些字段组成） |
| key_properties | 关键属性列表 |
| inspection_focus | 巡检关注点 |
| health_fields | 健康度字段 |
| required_relation_summary | 必需关系摘要 |
| import_label | Neo4j 导入标签格式 |

## 2. L1 资源类型关系（35 条）

### 核心拓扑关系

```
业务环境 (Environment)
  └─ CONTAINS → 应用 (Application)
       ├─ CONTAINS → 应用组件 (ApplicationComponent)
       │    ├─ DEPLOYED_AS → Deployment
       │    ├─ EXPOSES → Service
       │    ├─ USES → ConfigMap (推导)
       │    ├─ USES → Secret (推导)
       │    └─ USES → ContainerImage (推导)
       ├─ HAS_ALERT_RULE → 告警规则 (AlertRule)
       └─ HAS_DASHBOARD → 仪表盘 (Dashboard)

Deployment
  ├─ DEPLOYED_IN → Namespace
  ├─ USES → ConfigMap
  ├─ USES → Secret
  └─ USES → ContainerImage

Namespace
  ├─ BELONGS_TO → KubernetesCluster
  ├─ CONTAINS → Deployment
  ├─ CONTAINS → Service
  ├─ CONTAINS → Ingress
  ├─ CONTAINS → ConfigMap
  └─ CONTAINS → Secret

KubernetesCluster
  └─ CONTAINS → Namespace

Service
  ├─ DEPLOYED_IN → Namespace
  └─ EXPOSES → Deployment

Ingress
  ├─ ROUTES_TO → Service
  └─ DEPLOYED_IN → Namespace

ContainerImage
  └─ STORED_IN → ContainerRegistry

ContainerRegistry
  └─ CONTAINS → ContainerImage

AlertRule
  └─ MONITORS → ApplicationComponent / Deployment / Service / Ingress

Dashboard
  └─ VISUALIZES → ApplicationComponent / Deployment
```

### 关系属性模板

| 属性 | 说明 |
|------|------|
| edge_id | 唯一标识（REL-001 ~ REL-034） |
| relationship_type | 关系类型名 |
| relationship_name | 中文名称 |
| dependency_strength | 依赖强度：强/中/弱 |
| is_required | 是否必需 |
| auto_discovery | 自动发现/半自动/推导 |
| impact_analysis | 是否用于影响分析 |
| inspection_purpose | 巡检目的 |
| inspection_check_item | 巡检检查项 |
| risk_signal | 风险信号 |
| impact_direction | 影响方向 |
| alert_aggregation | 告警归并策略 |
| discovery_method | 发现方式 |
| graph_view | 关联视图 |

## 3. L2 资源实例（15 个模板节点）

基于"订单应用"（order-api）的示例实例数据。

| node_id | node_label | name | 关键属性 |
|---------|------------|------|----------|
| env:prod | Environment | 生产环境 | env_type=prod |
| app:order | Application | 订单应用 | sla_level=P1 |
| comp:order-api | ApplicationComponent | 订单API组件 | runtime_type=java |
| cluster:cce-prod-01 | KubernetesCluster | 生产K8s集群01 | version=1.29 |
| ns:cce-prod-01:order | Namespace | order | quota=cpu=20,memory=64Gi |
| deploy:cce-prod-01:order:order-api | Deployment | order-api | desired=3, available=2 |
| svc:cce-prod-01:order:order-api | Service | order-api-svc | type=ClusterIP, ports=[80] |
| ing:cce-prod-01:order:order-api | Ingress | order-api-ing | host=order.example.com, tls=true |
| cm:cce-prod-01:order:order-api-config | ConfigMap | order-api-config | data_keys=[app.yaml] |
| secret:cce-prod-01:order:order-api-secret | Secret | order-api-secret | expiry_days=14 |
| registry:harbor-prod | ContainerRegistry | 生产Harbor仓库 | url=harbor.example.com |
| image:order-api:1.2.3 | ContainerImage | order-api:1.2.3 | critical_vulns=1, tag=1.2.3 |
| alert:order-api-availability | AlertRule | 订单API可用性告警 | severity=critical |
| dash:order-api-overview | Dashboard | 订单API总览仪表盘 | url=https://grafana.../d/order-api |

### 实例节点公共属性

| 属性 | 说明 |
|------|------|
| node_id | 唯一标识 |
| label | 节点类型标签 |
| name | 名称 |
| unique_key | 业务唯一键 |
| env_code / app_code / component_code | 归属路径 |
| cluster_id / namespace | K8s 定位 |
| owner_team | 负责团队 |
| lifecycle_status | active / inactive |
| health_status | normal / warning / critical |
| risk_level | low / medium / high / critical |
| inspection_status | passed / partial / failed |
| last_inspected_at | 最近巡检时间 |
| source_system | 数据来源系统 |
| source_ref | 来源引用 |
| attrs_json | 扩展属性（JSON） |

## 4. L2 实例关系（14 条模板边）

| edge_id | source | rel_type | target | risk_signal |
|---------|--------|----------|--------|-------------|
| e001 | env:prod | CONTAINS | app:order | |
| e002 | app:order | CONTAINS | comp:order-api | |
| e003 | comp:order-api | DEPLOYED_AS | deploy:...order-api | 可用副本不足 |
| e004 | deploy:...order-api | DEPLOYED_IN | ns:...order | |
| e005 | ns:...order | BELONGS_TO | cluster:cce-prod-01 | |
| e006 | svc:...order-api | EXPOSES | deploy:...order-api | |
| e007 | ing:...order-api | ROUTES_TO | svc:...order-api | |
| e008 | deploy:...order-api | USES | cm:...order-api-config | |
| e009 | deploy:...order-api | USES | secret:...order-api-secret | Secret 即将过期 |
| e010 | deploy:...order-api | USES | image:order-api:1.2.3 | 镜像存在高危漏洞 |
| e011 | image:order-api:1.2.3 | STORED_IN | registry:harbor-prod | |
| e012 | alert:order-api-availability | MONITORS | comp:order-api | |
| e013 | dash:order-api-overview | VISUALIZES | comp:order-api | |

## 5. Neo4j 导入说明

### 约束
```cypher
CREATE CONSTRAINT resource_type_node_id IF NOT EXISTS
FOR (n:ResourceType) REQUIRE n.node_id IS UNIQUE;

CREATE CONSTRAINT resource_instance_node_id IF NOT EXISTS
FOR (n:ResourceInstance) REQUIRE n.node_id IS UNIQUE;
```

### 关系建模策略
使用 `RELATES_TO` 统一关系类型 + `relationship_type` 属性区分（兼容无 APOC 环境）。
如果安装了 APOC 插件，可启用动态关系类型（如 `:CONTAINS`、`:USES`、`:MONITORS`）。

### 导入脚本
见 `data/cloud_native_app_inspection_graph_import_package_v1/neo4j_import_cloud_native_app_inspection_graph_v1.cypher`
