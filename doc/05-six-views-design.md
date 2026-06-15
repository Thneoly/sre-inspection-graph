# 05 — 六大巡检视图设计

> 定义 6 个推荐巡检视图的业务目的、Cypher 查询、API 映射和前端展示。

## 1. 应用拓扑视图（Application Topology View）

### 业务目的
从应用视角看全链路：Application → Component → Deployment → Pod → Container。适用于巡检应用是否完整部署、Deployment 是否有足够副本、Pod 是否 Ready、容器是否频繁重启。

### 起始节点
`app:{app_code}` (Application)

### 遍历深度
5 跳

### Cypher 查询

```cypher
MATCH path = (app:ResourceInstance:Application {node_id: $app_node_id})
  -[:RELATES_TO*1..5]-(related:ResourceInstance)
WHERE ALL(r IN relationships(path) WHERE r.relationship_type IN [
  'CONTAINS', 'DEPLOYED_AS', 'DEPLOYED_IN', 'BELONGS_TO',
  'EXPOSES', 'ROUTES_TO', 'USES', 'STORED_IN',
  'MONITORS', 'VISUALIZES', 'RUNS', 'SCHEDULED_ON'
])
RETURN nodes(path) AS nodes, relationships(path) AS edges
LIMIT 200
```

### 节点着色
- Application: 蓝色大矩形
- ApplicationComponent: 绿色中矩形
- Deployment: 紫色中矩形
- Pod: 紫色椭圆（健康=绿, 警告=黄, 危险=红）
- Container: 浅蓝小椭圆
- ConfigMap: 棕色平行四边形
- Secret: 红色平行四边形
- Service: 青色菱形
- Ingress: 橙红菱形

### 右侧面板
- 点击 Application: 显示 SLO、负责人、环境、健康评分
- 点击 Pod: 显示 Pod IP、Node、Phase、CPU%、Memory%、重启次数
- 点击 Deployment: 显示期望/可用副本、发布策略

---

## 2. 访问链路视图（Access Link View）

### 业务目的
从 Ingress 入口追踪到后端容器：Ingress → Service → Deployment → Pod → Container。适用于巡检 Ingress 是否正常、Service 是否有后端、Pod 是否 Ready、访问链路是否断裂。

### 起始节点
`app:{app_code}` 关联的所有 Ingress

### Cypher 查询

```cypher
MATCH path = (ing:ResourceInstance:Ingress)
  -[:RELATES_TO*1..5]-(related:ResourceInstance)
WHERE EXISTS {
  MATCH (ing)-[:RELATES_TO*1..5]->(app:ResourceInstance:Application {node_id: $app_node_id})
}
AND ALL(r IN relationships(path) WHERE r.relationship_type IN [
  'ROUTES_TO', 'EXPOSES', 'DEPLOYED_IN', 'BELONGS_TO',
  'CONTAINS', 'DEPLOYED_AS', 'RUNS', 'SCHEDULED_ON'
])
RETURN nodes(path) AS nodes, relationships(path) AS edges
LIMIT 200
```

### 节点着色
- Ingress: 橙红菱形（重点高亮入口）
- Service: 青色菱形
- Deployment: 紫色矩形
- Pod: 椭圆（按健康状态着色）
- Container: 浅蓝小椭圆

### 链路段检查
- Ingress → Service: 路由是否可达，TLS 是否配置
- Service → Pod: 是否有 Endpoints
- Pod → Container: 容器是否 Ready

---

## 3. 节点影响视图（Node Impact View / Blast Radius）

### 业务目的
当某个 KubernetesNode 异常时，分析影响哪些 Pod → 哪些 Deployment → 哪些 ApplicationComponent → 哪些 Application。即"爆炸半径"分析。

### 起始节点
`node:{cluster_id}:{node_name}` (KubernetesNode)

### Cypher 查询

```cypher
MATCH path = (node:ResourceInstance:KubernetesNode {node_id: $node_id})
  <-[:RELATES_TO*1..4]-(affected:ResourceInstance)
WHERE ALL(r IN relationships(path) WHERE r.relationship_type IN [
  'SCHEDULED_ON', 'CONTAINS', 'DEPLOYED_AS', 'BELONGS_TO',
  'RUNS', 'CONTROLLED_BY'
])
OPTIONAL MATCH (affected)-[:RELATES_TO*1..3]->(app:ResourceInstance:Application)
WHERE ALL(r IN relationships(path) WHERE r.relationship_type IN [
  'CONTAINS', 'DEPLOYED_AS', 'BELONGS_TO'
])
RETURN nodes(path) AS nodes, relationships(path) AS edges, app.node_id AS impacted_app
LIMIT 200
```

### 节点着色
- KubernetesNode: 红色八角形（异常源）
- Pod: 按健康状态着色
- Deployment: 紫色
- ApplicationComponent: 绿色
- Application: 蓝色（变红=受影响）

---

## 4. 配置影响视图（Configuration Impact View）

### 业务目的
分析 Secret/ConfigMap 变更或过期影响哪些 Deployment → Pod → ApplicationComponent → Application。适用于密钥过期风险评估、配置变更影响分析。

### 起始节点
Secret 或 ConfigMap 实例节点 ID

### Cypher 查询

```cypher
MATCH path = (config:ResourceInstance)
  <-[:RELATES_TO*1..4]-(related:ResourceInstance)
WHERE config.node_id = $resource_id
  AND config.label IN ['Secret', 'ConfigMap']
  AND ALL(r IN relationships(path) WHERE r.relationship_type IN [
    'USES', 'CONTAINS', 'DEPLOYED_AS', 'BELONGS_TO', 'RUNS', 'SCHEDULED_ON'
  ])
RETURN nodes(path) AS nodes, relationships(path) AS edges
LIMIT 200
```

### 节点着色
- Secret: 红色平行四边形（重点关注）
- ConfigMap: 棕色平行四边形
- Deployment: 紫色（受影响的 Deployment 高亮）
- Application: 蓝色
- Pod: 按健康状态

---

## 5. 镜像风险视图（Image Risk View）

### 业务目的
分析有漏洞的镜像被哪些 Deployment 使用 → 影响哪些 Pod → 哪些 ApplicationComponent → 哪些 Application。适用于镜像漏洞影响面评估。

### 起始节点
`image:xxx` (ContainerImage)

### Cypher 查询

```cypher
MATCH path = (image:ResourceInstance:ContainerImage {node_id: $image_id})
  <-[:RELATES_TO*1..4]-(related:ResourceInstance)
WHERE ALL(r IN relationships(path) WHERE r.relationship_type IN [
  'USES', 'CONTAINS', 'DEPLOYED_AS', 'BELONGS_TO', 'RUNS', 'SCHEDULED_ON'
])
RETURN nodes(path) AS nodes, relationships(path) AS edges
LIMIT 200
```

### 节点着色
- ContainerImage: 红色矩形（漏洞源）
- Deployment: 紫色（受影响的 Deployment 高亮）
- Pod: 按健康状态
- ApplicationComponent: 绿色
- Application: 蓝色

### 风险汇总
- 使用该镜像的 Deployment 列表
- 受影响的 Pod 列表
- 影响的应用列表
- 镜像漏洞详情（CVE 编号、CVSS 评分）

---

## 6. 告警归并视图（Alert Aggregation View）

### 业务目的
将多个 Pod/Deployment 的告警归并到同一个 Deployment/ApplicationComponent/Application。适用于判断是否多个告警属于同一故障根因、定位应用级故障。

### 起始节点
所有 AlertEvent（按时间范围/严重程度过滤）

### Cypher 查询

```cypher
MATCH (alert:AlertEvent)
WHERE alert.status = 'firing'
  AND ($severity IS NULL OR alert.severity = $severity)
  AND alert.fired_at >= $since
MATCH (alert)-[:FIRED_ON]->(resource:ResourceInstance)
OPTIONAL MATCH path = (resource)-[:RELATES_TO*1..4]->(app:ResourceInstance:Application)
WHERE ALL(r IN relationships(path) WHERE r.relationship_type IN [
  'CONTAINS', 'DEPLOYED_AS', 'BELONGS_TO', 'SCHEDULED_ON', 'RUNS'
])
RETURN alert, resource, app, nodes(path) AS nodes, relationships(path) AS edges
LIMIT 200
```

### 归并逻辑
- 前端按 Application 分组
- 同一 Deployment 下的多个 Pod 告警 → 归并到 Deployment
- 同一 ApplicationComponent 下的多个 Deployment 告警 → 归并到 Component
- 同一 Application 下的多个 Component 告警 → 归并到 Application

### 节点着色
- AlertEvent: 红色三角形（firing=红, resolved=绿）
- Pod: 按健康状态
- Deployment: 紫色
- Application: 蓝色（受影响的标红）

---

## 视图汇总

| 视图 | 起始节点 | 方向 | 深度 | 核心用途 |
|------|----------|------|------|----------|
| 应用拓扑 | Application | 正向 | 5 | 全链路健康度巡检 |
| 访问链路 | Ingress | 正向 | 5 | 入口到后端连通性检查 |
| 节点影响 | KubernetesNode | 反向 | 4 | 节点故障爆炸半径 |
| 配置影响 | Secret/ConfigMap | 反向 | 4 | 配置/密钥变更影响面 |
| 镜像风险 | ContainerImage | 反向 | 4 | 镜像漏洞影响面 |
| 告警归并 | AlertEvent | 正向 | 4 | 多告警归并到根因 |
