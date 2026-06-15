# SRE 云原生巡检图谱平台

基于 Neo4j 图数据库的四层模型巡检平台。

## 架构

```
L1 资源类型图谱  →  14 类型节点 + 35 关系（已定义）
L2 资源实例图谱  →  具体应用/组件/Deployment/Pod/Container 实例
L3 动态观测层    →  MetricQuery/MetricSnapshot 指标快照
L4 巡检结果层    →  InspectionRun/Rule/Finding/AlertEvent
```

## 技术栈

- **图数据库**: Neo4j 5 (Docker)
- **后端**: Python 3.12 + FastAPI + Neo4j Driver + uv
- **前端**: React 18 + TypeScript + Cytoscape.js + Vite
- **部署**: Docker Compose

## 命令速查

| 命令 | 作用 |
|------|------|
| `make setup` | 安装后端 (uv) + 前端 (npm) 依赖 |
| `make mock-data` | 生成模拟 CSV/Cypher |
| `make infra` | 仅启动基础服务 (Neo4j + 数据导入) |
| `make up` | 一键启动 Neo4j + API + 前端 |
| `make down` | 停止所有服务 |
| `make dev-api` | 本地热重载启动 API |
| `make dev-frontend` | 本地 HMR 启动前端 |
| `make test` | 运行全部测试 |
| `make test-cov` | 运行测试 + 覆盖率报告 |
| `make clean` | 清理容器、数据卷、生成文件 |

## 快速开始

```bash
# 1. 首次使用：安装依赖
make setup

# 2. 生成 Mock 数据
make mock-data

# 3. 启动全部服务
make up

# 4. 访问
# 前端:   http://localhost:3000
# API:    http://localhost:8000/docs
# Neo4j:  http://localhost:7474  (用户名 neo4j, 密码 sre-inspection)
```

## 开发模式

开发时只启动 Neo4j，API 和前端本地热重载运行：

```bash
# 终端 1：启动基础设施
make infra

# 终端 2：启动 API (修改代码自动重载)
make dev-api

# 终端 3：启动前端 (修改代码 HMR 更新)
make dev-frontend
```

## 测试

```bash
# 运行全部测试 (backend 53 + frontend 24 = 77)
make test

# 仅后端测试 + 覆盖率
make test-cov
```

## 6 个巡检视图

| 视图 | 路由 | 说明 |
|------|------|------|
| 应用拓扑 | `/topology` | Application → Component → Deployment → Pod → Container |
| 访问链路 | `/access-link` | Ingress → Service → Deployment → Pod → Container |
| 节点影响 | `/node-impact` | Node 故障爆炸半径分析 |
| 配置影响 | `/config-impact` | Secret/ConfigMap 变更影响面 |
| 镜像风险 | `/image-risk` | 镜像漏洞影响传播 |
| 告警归并 | `/alert-aggregation` | 多告警按应用归并 |

## 目录结构

```
├── doc/            # 设计文档 (7 份)
├── data/           # 原始 CSV + Cypher 脚本
├── scripts/        # Mock 数据生成脚本
│   └── output/     # 生成的 CSV + Cypher
├── backend/        # FastAPI 应用
│   ├── app/
│   │   ├── db/         # Neo4j 客户端 + Cypher 查询
│   │   ├── routers/    # API 路由 (6 视图 + health + metrics)
│   │   ├── models/     # Pydantic 模型
│   │   └── services/   # 业务逻辑
│   └── tests/          # pytest 测试
├── frontend/       # React 前端
│   └── src/
│       ├── components/
│       │   ├── Graph/   # 图渲染核心组件
│       │   ├── Views/   # 6 个视图页面
│       │   └── Layout/  # 布局组件
│       ├── api/         # API 客户端
│       ├── hooks/       # 自定义 Hooks
│       ├── utils/       # 样式映射
│       └── __tests__/   # vitest 测试
├── docker-compose.yml
├── Makefile
└── .gitignore
```
