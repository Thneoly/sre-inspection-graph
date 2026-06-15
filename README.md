# SRE 云原生巡检图谱平台

基于 Neo4j 四层模型的云原生资源巡检与故障模拟平台。

## 架构

```
L1 资源类型图谱  →  14 类型节点 + 35 关系
L2 资源实例图谱  →  应用/组件/Deployment/Pod/中间件 30+ 实例
L3 动态观测层    →  MetricQuery/MetricSnapshot + AlertEvent
L4 巡检结果层    →  InspectionRun/Rule/Finding
+  故障模拟引擎   →  FaultScenario + 时间线推进 + 数据维度注入
```

## 技术栈

- **图数据库**: Neo4j 5 (Docker)
- **后端**: Python 3.12 + FastAPI + Neo4j Driver + uv
- **前端**: React 18 + TypeScript + Cytoscape.js + Ant Design 5 + Vite
- **部署**: Docker Compose

## 命令速查

| 命令 | 作用 |
|------|------|
| `make setup` | 安装后端 (uv) + 前端 (npm) 依赖 |
| `make mock-data` | 生成模拟 CSV/Cypher |
| `make infra` | 仅启动基础服务 (Neo4j + 数据导入) |
| `make infra_up` | 恢复 Neo4j（不重导数据） |
| `make infra_down` | 停止 Neo4j |
| `make up` | 一键启动 Neo4j + API + 前端 |
| `make down` | 停止所有服务 |
| `make dev-api` | 本地热重载启动 API（自动清理端口占用） |
| `make dev-api-kill` | 强制释放 8000 端口 |
| `make dev-frontend` | 本地 HMR 启动前端 |
| `make test` | 运行全部测试 |
| `make test-cov` | 后端测试 + 覆盖率 |
| `make clean` | 清理容器、数据卷、生成文件 |

## 快速开始

```bash
make setup && make mock-data && make up

# 前端  http://localhost:3000
# API   http://localhost:8000/docs
# Neo4j http://localhost:7474 (neo4j / sre-inspection)
```

## 开发模式

```bash
make infra          # 终端1: Neo4j
make dev-api        # 终端2: API (热重载)
make dev-frontend   # 终端3: 前端 (HMR)
```

## 7 个巡检视图

| 视图 | 路由 | 说明 |
|------|------|------|
| 应用拓扑 | `/topology` | 全链路: Region→AZ→Cluster→NS→Deploy→Pod→Container + 中间件 |
| 访问链路 | `/access-link` | 入口追踪: ELB→Ingress→Gateway→Service→Pod |
| 节点影响 | `/node-impact` | Node 故障爆炸半径 |
| 配置影响 | `/config-impact` | Secret/ConfigMap 变更影响面 |
| 镜像风险 | `/image-risk` | 镜像漏洞影响传播 |
| 告警归并 | `/alert-aggregation` | 多告警按应用归并 |

## 故障模拟（独立页面）

访问 `/simulation` 或从拓扑页右上角「⚡ 故障模拟」进入。

- **7 种故障类型**: CPU 飙升 / 内存泄漏 / Pod CrashLoop / 节点磁盘压力 / Service 无后端 / MySQL 慢查询 / Redis 不可达
- **数据维度注入**: 每次 step 写入 MetricSnapshot + AlertEvent + InspectionFinding
- **自动传播**: 故障沿 DEPLOYED_AS / CONTAINS / USES / DEPENDS_ON 向上游传播
- **时间线**: 注入 → 检测 → 告警 → 巡检发现 → 恢复

## 图层面板

每个视图工具栏右侧有图层标签：

| 图层 | 默认 | 包含关系 |
|------|------|---------|
| 基础拓扑 | ✅ | CONTAINS, DEPLOYED_AS, USES, BELONGS_TO... |
| 可观测 | 关 | MONITORS, VISUALIZES |
| 风险巡检 | 关 | AFFECTS, FIRED_ON, GENERATED... |

## 节点视觉规则

- **形状 = 类型**: 椭圆(Pod/Container) / 菱形(网络) / 六边形(基础设施) / 矩形(工作负载) / 三角形(告警) / 平行四边形(配置)
- **颜色 = 健康**: 绿=正常 / 黄=警告 / 红=严重
- **边框 = 风险**: 细绿=low / 中黄=medium / 粗红=high

点击节点 → 右侧面板显示属性 + 指标 + 巡检发现。
点击边 → 右侧面板显示关系类型 + 双方节点 + 风险信号。

## 目录结构

```
├── doc/               # 设计文档 (8 份)
├── datas/             # 原始 CSV 数据
├── scripts/           # Mock 生成 + 故障注入脚本
│   └── output/        # 生成的 CSV + Cypher
├── backend/
│   ├── app/
│   │   ├── db/        # Neo4j 客户端 + 6 视图 Cypher 查询
│   │   ├── routers/   # API 路由 (视图 + simulation + health)
│   │   ├── models/    # Pydantic 模型
│   │   └── services/  # 业务逻辑
│   └── tests/         # 53 pytest
├── frontend/
│   └── src/
│       ├── components/
│       │   ├── Graph/      # GraphCanvas + NodeDetailPanel + LayerToggle
│       │   ├── Views/      # 6 视图 + SimulationView
│       │   └── Layout/     # MainLayout (antd)
│       ├── api/            # API client (Axios)
│       ├── hooks/          # useGraphData
│       ├── utils/          # graphStyles + layers + resourceIcons
│       └── __tests__/      # 16 vitest
├── docker-compose.yml
├── Makefile
└── .gitignore
```

## 测试

```bash
make test          # backend 53 + frontend 16 = 69 tests
make test-cov      # backend coverage
```
