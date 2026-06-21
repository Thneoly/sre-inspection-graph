# SRE 云原生巡检图谱平台

基于 Neo4j 四层模型的云原生资源巡检 + 故障模拟 + **恢复动作引擎(含审批流 + 一键回滚)** + **OpenTelemetry Demo 真实数据接入(PRD-004)** 平台。

## 架构

```
L1 资源类型图谱  →  14 类型节点 + 35 关系
L2 资源实例图谱  →  应用/组件/Deployment/Pod/中间件 30+ 实例
L3 动态观测层    →  MetricQuery/MetricSnapshot + AlertEvent + ChangeEvent
L4 巡检结果层    →  InspectionRun/Rule/Finding
+  故障模拟引擎   →  FaultScenario + 时间线推进 + 数据维度注入
+  恢复动作引擎   →  RecoveryAction + Dry-run + 审批流 + 回滚 (PRD-001)
+  变更事件追踪   →  ChangeEvent + 故障关联查询 + 前端时间线 + Neo4j 双写 (PRD-002)
+  实数据 connector → K8s + Prometheus + Jaeger + flagd + K8s events (PRD-004)
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
| 审批中心 | `/recovery/approvals` | medium/high_risk 动作的审批操作面板 |
| 恢复历史 | `/recovery/history` | 已执行 / 已回滚的动作审计历史 |
| 变更时间线 | `/change-timeline` | 应用级变更事件时间线 + 影响范围 (PRD-002) |

## 恢复动作引擎(PRD-001)

8 个动作覆盖全部资源类型,运维点击 → dry-run 预演 → (可选审批) → 执行 → 出问题一键回滚。

| 动作 | 风险 | 目标 | 是否要审批 |
|---|---|---|---|
| `scale_deployment` | low | Deployment | 否 |
| `kill_query` | low | MySQL | 否 |
| `restart_service` | low | Service | 否 |
| `restart_pod` | medium | Pod | 是 |
| `refresh_secret` | medium | Secret | 是 |
| `clear_cache` | medium | Redis | 是 |
| `rollback_deployment` | high | Deployment | 是 |
| `drain_node` | high | KubernetesNode | 是 |

**生命周期**: `pending → dry_run_ok → awaiting_approval → approved/rejected → executing → succeeded/failed → rolled_back`

**关键设计**:
- low_risk → 同步执行 (HTTP 200);medium/high → 创建 ApprovalRequest (HTTP 202)
- `approver_team` 从 `target.owner_team` 派生,沿 `BELONGS_TO` 上溯到 Component / Application(软记录,不强制 RBAC)
- 审批 24h TTL,**读时检查**(无后台 cron)
- 一键回滚:`POST /executions/{id}/rollback` 直接走反向 handler,**不再二次审批**(原动作已审批,反向是"撤销")
- handler 当前是 mock(改 DSS 状态),Phase 2 接入真实 client-go / pymysql / redis-py

**端到端验证**:
```bash
make dev-api &                    # 终端 1
bash scripts/sprint3_e2e_test.sh  # 终端 2 — 8 步检查 high_risk 审批流 + 回滚
```

## OpenTelemetry Demo 真实数据接入(PRD-004)

平台从 mock CSV 升级为接入 vm 集群上跑的 **OpenTelemetry Demo 0.32.0**(14 微服务 + Postgres/Valkey/Kafka/flagd)作为第一个真实数据源。5 个 connector 30 秒轮询写入 DSS:

| Connector | 拉什么 | 写入 |
|---|---|---|
| `k8s` | Deployment/Pod/Service/ConfigMap/Secret(kubernetes-asyncio) | DataNode + DataEdge,17 业务 service 自动建模 |
| `prometheus` | OTel Collector spanmetrics(p99 / error_rate / request_rate) | MetricSnapshot,自动推导 component health(green/yellow/red) |
| `jaeger` | trace span ChildOf 引用 | CALLS 边,call_count_5m ≥ 5 阈值过滤 |
| `flagd` | feature flag state diff | ChangeEvent(source=flagd) |
| `k8s_events` | ScalingReplicaSet / SuccessfulRescale | ChangeEvent(deployment_rolled) |

**控制端点**(`/api/v1/connectors`):
- `GET /status` — 5 个 connector 整体健康
- `POST /{name}/sync-now` — 手动触发一次 sync,返回 SyncResult

**8 个 OTel demo fault scenarios**(`backend/app/recovery/scenarios/otel_demo_scenarios.py`):flag 名 → 目标 component → 推荐 PRD-001 action。涵盖 productCatalogFailure / cartFailure / paymentServiceFailure / kafkaQueueProblems 等。

**部署 + 验证**:
```bash
# 1. vm 集群安装 OTel demo(锁定 chart 0.32.0)
bash scripts/otel_demo/deploy.sh

# 2. 起三个 port-forward(Prometheus / Jaeger / flagd)
kubectl -n otel-demo port-forward svc/otel-demo-prometheus-server 19090:9090
kubectl -n otel-demo port-forward svc/otel-demo-jaeger-query 16686:16686
kubectl -n otel-demo port-forward svc/otel-demo-flagd 8013:8013

# 3. 起 API(带正确 env)
KUBECONFIGS=vm-cluster=$HOME/.kube/vm-config \
PROMETHEUS_URL=http://localhost:19090 \
JAEGER_URL=http://localhost:16686/jaeger/ui \
FLAGD_URL=http://localhost:8013 \
make dev-api

# 4. E2E
bash scripts/otel_demo_e2e.sh   # 7 步检查 5 connector + scenario 列表
```

## 故障模拟（独立页面）

访问 `/simulation` 或从拓扑页右上角「⚡ 故障模拟」进入。

- **7 种故障类型**: CPU 飙升 / 内存泄漏 / Pod CrashLoop / 节点磁盘压力 / Service 无后端 / MySQL 慢查询 / Redis 不可达
- **目标校验**: 自动过滤兼容目标（如 CPU 飙升只能注入到 Pod，不能注入到 MySQL）
- **爆炸半径 + 多级级联**: 注入节点故障后，blast radius 波及关联节点，cascade 沿依赖链向上传播
- **每类型独立阈值**: 不同资源类型的容错率和告警延迟不同（如 Application 比 Pod 更"扛"）
- **推进机制**: 每次点击「推进下一阶段」固定前进 1 个阶段，阶段进度可见（如 2/6）
- **数据持久化**: 故障场景持久化到 Neo4j，uvicorn 重启后自动恢复活跃故障

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
├── scripts/           # Mock 生成 + 故障注入 + E2E 测试脚本
│   ├── sprint3_e2e_test.sh   # 审批流 + 回滚端到端手测
│   └── output/        # 生成的 CSV + Cypher
├── backend/
│   ├── app/
│   │   ├── db/        # Neo4j 客户端 + 6 视图 Cypher 查询
│   │   ├── routers/   # API 路由 (视图 + simulation + recovery + health)
│   │   ├── recovery/  # PRD-001:action_defs / cascade / execution / approval / handlers
│   │   ├── datasource/# DSS 内存孪生 (nodes / edges / executions / approvals)
│   │   ├── models/    # Pydantic 模型
│   │   └── services/  # 业务逻辑
│   └── tests/         # 157 pytest (含 104 个 recovery 测试)
├── frontend/
│   └── src/
│       ├── components/
│       │   ├── Graph/      # GraphCanvas + NodeDetailPanel + LayerToggle
│       │   ├── Views/      # 6 视图 + SimulationView
│       │   ├── Recovery/   # RecoveryActionsSection / DryRunModal /
│       │   │              # ExecutionsView / ApprovalsView (PRD-001)
│       │   └── Layout/     # MainLayout (antd)
│       ├── api/            # API client (Axios)
│       ├── hooks/          # useGraphData
│       ├── utils/          # graphStyles + layers + resourceIcons
│       └── __tests__/      # 46 vitest
├── docker-compose.yml
├── Makefile
└── .gitignore
```

## 测试

```bash
make test          # backend 288 + frontend 46 = 334 tests
make test-cov      # backend coverage
```
