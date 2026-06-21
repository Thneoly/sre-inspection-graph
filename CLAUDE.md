# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

SRE 云原生巡检图谱平台 — cloud-native resource inspection graph platform based on a 4-layer Neo4j model with fault simulation, **a recovery action engine (PRD-001) covering 8 actions / dry-run / approval flow / one-click rollback**, **change event tracking with correlated query (PRD-002)**, and **real-data ingestion via 5 OTel-Demo connectors (PRD-004)**.

## Architecture

```
L1 Resource Type Graph  →  14 type nodes + 35 relationships (static, in CSV)
L2 Resource Instance Graph → application/component/Deployment/Pod/middleware instances
L3 Dynamic Observability → MetricQuery + MetricSnapshot + AlertEvent + ChangeEvent
L4 Inspection Results → InspectionRun/Rule/Finding

+ Data Source Service (DSS) → in-memory cache layer, decouples fault injection from Neo4j
+ Recovery Action Engine (PRD-001) → 8 actions / dry-run / approval flow / rollback
+ Change Event Tracking (PRD-002) → ChangeEvent + correlated query + propagation BFS
+ Real-data Connectors (PRD-004) → K8s / Prometheus / Jaeger / flagd / K8s-events
+ Self-Inspection Report (PRD-003) → Jinja2 Markdown 报告 + 3 模板 + 12 模块 + 邮件订阅
```

## Tech Stack

- **Graph DB**: Neo4j 5 (Docker, bolt://localhost:7687, neo4j/sre-inspection)
- **Backend**: Python 3.12 + FastAPI + uv (`backend/`)
- **Frontend**: React 18 + TypeScript + Cytoscape.js + Ant Design 5 + Vite (`frontend/`)
- **Deployment**: Docker Compose

## Commands

```bash
# First-time setup (uv sync backend + npm install frontend)
make setup

# Development
make infra          # Start Neo4j + run import (depends on mock-data)
make infra_up       # Restart Neo4j without re-importing data
make infra_down     # Stop Neo4j only
make dev-api        # API hot-reload on port 8000 (auto-frees port via dev-api-kill)
make dev-frontend   # Frontend HMR on port 3000

# Full stack
make up             # Docker Compose start all
make down           # Stop all
make clean          # Remove containers + volumes + generated data

# Testing
make test           # Backend 362 tests + Frontend 62 tests
make test-cov       # Backend coverage report

# Single test
cd backend && uv run python -m pytest tests/test_routers.py::test_topology -v -p no:asyncio
cd backend && uv run python -m pytest tests/ -k "fault" -v -p no:asyncio    # filter by name
cd backend && uv run python -m pytest tests/ -k "recovery" -v -p no:asyncio # 104 recovery tests
cd backend && uv run python -m pytest tests/test_sprint23_connectors.py -v -p no:asyncio  # 39 PRD-004 tests
cd frontend && npm test -- GraphCanvas                                       # vitest substring match

# Mock data
make mock-data      # Generate CSV + Cypher → scripts/output/

# E2E (Sprint 3 — approval flow + rollback)
bash scripts/sprint3_e2e_test.sh    # 8 步检查 high_risk 审批流 + 一键回滚

# E2E (PRD-004 — 5 connector live verify against vm cluster)
bash scripts/otel_demo_e2e.sh       # 7 步检查 K8s/Prom/Jaeger/flagd/k8s_events + scenarios
```

Note: backend pytest **must** be run with `-p no:asyncio` — the project uses sync FastAPI TestClient and pytest-asyncio's auto-mode otherwise breaks fixture scoping.

## Directory Structure

```
backend/
├── app/
│   ├── main.py              # FastAPI entry, CORS, router registration
│   ├── config.py            # Env vars (NEO4J_URI, NEO4J_USER, NEO4J_PASSWORD)
│   ├── db/
│   │   ├── neo4j_client.py  # Singleton Neo4j driver, session-based queries
│   │   └── queries/         # 6 Cypher view queries (view1..view6)
│   ├── datasource/          # DSS — Data Source Service
│   │   ├── models.py        # DataNode, DataEdge, MetricSnapshot, FaultInjection,
│   │   │                    # RecoveryExecution, ApprovalRequest, ChangeEvent
│   │   ├── store.py         # In-memory singleton stores
│   │   ├── loader.py        # Load baseline from Neo4j → DSS
│   │   ├── fault_injector.py # Fault injection via DSS
│   │   └── connectors/      # PRD-004 — real-data connectors
│   │       ├── base.py      # BaseConnector + asyncio polling loop
│   │       ├── k8s_connector.py / k8s_mapper.py  # K8s topology sync
│   │       ├── prometheus_connector.py + prometheus_queries.py + health_rules.py
│   │       ├── jaeger_connector.py + trace_aggregator.py  # CALLS edges
│   │       ├── flagd_connector.py        # flag diff → ChangeEvent
│   │       ├── k8s_event_connector.py    # K8s event → ChangeEvent
│   │       └── sync_orchestrator.py      # ConnectorRegistry singleton
│   ├── changes/             # PRD-002 — ChangeEvent service
│   │   ├── event_service.py # record_change + correlated query + timeline
│   │   └── propagation.py   # Reverse BFS along PROPAGATION_EDGES
│   ├── recovery/            # PRD-001 Recovery Action Engine
│   │   ├── action_defs.py   # 8 action templates (single source of truth)
│   │   ├── cascade.py       # Reverse-cascade BFS for dry-run impact
│   │   ├── execution.py     # Lifecycle orchestration (low_risk sync /
│   │   │                    # medium-high → awaiting_approval / rollback)
│   │   ├── approval.py      # request / approve / reject / 24h TTL /
│   │   │                    # _derive_approver_team along BELONGS_TO
│   │   ├── handlers/        # 8 mock handlers (Phase 2 → real K8s/MySQL/Redis)
│   │   └── scenarios/       # 8 OTel-demo flag → action mappings (PRD-004)
│   ├── routers/
│   │   ├── topology.py, access_link.py, node_impact.py, config_impact.py,
│   │   │   image_risk.py, alert_aggregation.py, health.py
│   │   ├── recovery.py      # PRD-001 endpoints (actions / dry-run / execute /
│   │   │                    # executions / approvals / rollback)
│   │   ├── change_event.py  # PRD-002 endpoints (record / correlated / timeline)
│   │   ├── connectors.py    # PRD-004 endpoints (status / sync-now)
│   │   ├── simulation.py    # Legacy fault simulation (direct Neo4j writes)
│   │   └── datasource.py    # DSS REST API (extraction + injection)
│   ├── models/              # Pydantic: GraphNode, GraphEdge, GraphResponse, metrics
│   └── services/            # graph_service.py, metrics_service.py
└── tests/
    ├── conftest.py          # Mock Neo4j fixtures + FastAPI TestClient
    ├── mocks.py             # MockNeo4jNode, MockNeo4jRel, MockNeo4jPath
    ├── test_services.py, test_queries.py, test_routers.py   # baseline 53
    ├── test_fault_*.py                                       # fault simulation
    ├── test_recovery.py                                      # actions / dry-run
    ├── test_recovery_execute.py                              # Sprint 2 execute
    └── test_recovery_approval.py                             # Sprint 3 approval + rollback

frontend/src/
├── components/
│   ├── Graph/               # GraphCanvas (Cytoscape), NodeDetailPanel, LayerToggle
│   ├── Views/               # 6 inspection views + SimulationView
│   ├── Recovery/            # RecoveryActionsSection, DryRunModal,
│   │                        # ExecutionsView, ApprovalsView (PRD-001)
│   └── Layout/              # MainLayout (antd Layout + Sider, 8 menu entries)
├── utils/
│   ├── graphStyles.ts       # Node shapes, health colors (green/yellow/red), edge styles
│   ├── layers.ts            # Layer definitions + filterGraphData()
│   └── resourceIcons.ts     # SVG icons for node types (unused in current design)
├── api/client.ts            # Axios client + TypeScript types (incl. ApprovalRequest)
└── __tests__/               # 38 vitest tests

scripts/
├── generate_all_mock_data.py   # Generate L3/L4 mock CSV + Cypher
├── generate_l3_mock_data.py    # L3: Pod/Container/Node/MetricQuery/Snapshot
├── generate_l4_mock_data.py    # L4: InspectionRun/Rule/Finding/AlertEvent
├── fault_simulation.py         # CLI fault injector (legacy, direct Neo4j)
├── add_infra_nodes.py          # Add Region/AZ/ELB/Gateway/Nacos/MySQL/Redis/Kafka
├── sprint3_e2e_test.sh         # Sprint 3 approval + rollback E2E (curl + jq)
└── output/                     # Generated CSV + Cypher files
```

## Key Design Decisions

1. **Neo4j 5+ requires string interpolation for path depth** — `*1..5` not `*1..$depth`. Depth validated by FastAPI (ge=1, le=10), safe to interpolate.
2. **Nodes use property-based labels** — nodes are `ResourceInstance` with a `label` property (e.g., `"Application"`), NOT `:ResourceInstance:Application`.
3. **Neo4j query returns native Record objects** — use `session.run()` (not `execute_query`) to get Neo4j Path/Node/Relationship objects. Record uses `.get()` not `in` operator.
4. **DSS decouples fault injection from Neo4j** — faults write to DSS memory, DSS syncs to Neo4j. Production should use DSS endpoints, not simulation endpoints.
5. **Health = fill color (green/yellow/red), Shape = resource type** — no per-type coloring, shapes differentiate types.
6. **Uvicorn auto-kill on port 8000** — `make dev-api` runs `dev-api-kill` first.

## Node Visual Rules

- **Shape = resource type**, **fill color = health** (green/yellow/red), **border weight + color = risk level** (thin green = low, medium yellow, thick red = high). No per-type fill coloring.
- **ellipse**: Pod, Container
- **diamond**: Service, Ingress, ELB, Gateway, APIG, Nacos
- **hexagon**: KubernetesCluster, KubernetesNode, ContainerRegistry, Region, AZ
- **rectangle**: Deployment, Namespace, ContainerImage, MySQL, Redis, Kafka, Dashboard
- **round-rectangle**: Application, ApplicationComponent, Environment, InspectionRun
- **parallelogram**: ConfigMap, Secret
- **triangle**: AlertRule, AlertEvent
- **tag**: InspectionFinding, InspectionRule

## Inspection Views & Routes

Each view is one router on the backend + one component under `frontend/src/components/Views/` + one Cypher query under `backend/app/db/queries/`.

| Route | Backend router | Purpose |
|---|---|---|
| `/topology` | `topology.py` | Full chain Region→AZ→Cluster→NS→Deploy→Pod→Container + middleware |
| `/access-link` | `access_link.py` | Ingress trace ELB→Ingress→Gateway→Service→Pod |
| `/node-impact` | `node_impact.py` | KubernetesNode failure blast radius |
| `/config-impact` | `config_impact.py` | Secret/ConfigMap change impact surface |
| `/image-risk` | `image_risk.py` | Container image vulnerability propagation |
| `/alert-aggregation` | `alert_aggregation.py` | Multi-alert rollup by application |
| `/recovery/approvals` | `recovery.py` | Approval center for medium/high_risk actions |
| `/recovery/history`   | `recovery.py` | Execution audit history with rollback button |
| `/simulation` | `simulation.py` + `datasource.py` | Fault simulation (DSS-backed) |

Layer toggles (`frontend/src/utils/layers.ts`) filter the response: 基础拓扑 (default) / 可观测 (MONITORS, VISUALIZES) / 风险巡检 (AFFECTS, FIRED_ON, GENERATED).

## Fault Types (7)

cpu_spike, memory_leak, pod_crashloop, node_disk_pressure, service_no_endpoints, mysql_slow_query, redis_unavailable

- Target validation: fault type must match target node type (e.g. cpu_spike → Pod only)
- Blast radius: faults affect connected nodes (e.g. node_disk_pressure → all Pods on that Node)
- Cascade: `blast_propagate_to` chain propagates upstream (Pod→Deployment→Component→Application)
- Thresholds: each resource type has `degradation_delay`, `warning_at_pct`, `critical_at_pct`, `risk_multiplier`
- Step always advances 1 stage per click (not time-based). Stage progress visible as "阶段 2/6".
- Faults persist to Neo4j via `persist_fault()` / `update_fault_in_neo4j()`. `_recover_faults()` on startup.

## Recovery Action Engine — PRD-001

8 actions covering all resource types. Operators click → dry-run preview → (approval if needed) → execute → optional one-click rollback.

| Action | Risk | Target | Approval |
|---|---|---|---|
| `scale_deployment` | low | Deployment | no |
| `kill_query` | low | MySQL | no |
| `restart_service` | low | Service | no |
| `restart_pod` | medium | Pod | yes |
| `refresh_secret` | medium | Secret | yes |
| `clear_cache` | medium | Redis | yes |
| `rollback_deployment` | high | Deployment | yes |
| `drain_node` | high | KubernetesNode | yes |

**Lifecycle**: `pending → dry_run_ok → awaiting_approval → approved/rejected → executing → succeeded/failed → rolled_back`

### Key Design Decisions

1. **HTTP semantics**: low_risk → 200 (sync done). medium/high → 202 Accepted (awaiting_approval + `approval_id`). Frontend branches on `status` field, not status code.
2. **`_continue_after_approval`**: approve endpoint atomically marks approved → triggers handler → returns succeeded/failed in same HTTP call. No separate "start execution" click.
3. **Rollback skips second approval** — `POST /executions/{id}/rollback` runs reverse handler directly even if rollback_action is high_risk. Reasoning: original action was already approved, reverse is "undo" not "new risk".
4. **`approver_team` derivation**: read `target.owner_team`; for Pod / Service etc. without it, traverse `BELONGS_TO` edges up to Component / Application; default `"platform"`. Soft-record only — no RBAC enforcement.
5. **24h TTL is read-time** — every list / get approval call sweeps `pending` requests with `expiry_at < now` and marks them `expired`. No background cron.
6. **Rollback idempotency** — only `succeeded` executions rollback once; `rolled_back` final.
7. **`ApprovalRequest` not in Neo4j** (runtime-only). `RecoveryExecution` continues dual-write to DSS + Neo4j.
8. **Mock handlers** — Sprint 3 handlers update DSS node properties to simulate the action (e.g. `restart_count++`, `current_revision--`, `cordoned=True`). Phase 2 swaps in client-go / pymysql / redis-py.

### File Map

- Action templates + propagation rules: `backend/app/recovery/action_defs.py`
- Dry-run cascade BFS: `backend/app/recovery/cascade.py`
- Lifecycle orchestration: `backend/app/recovery/execution.py` — `execute()`, `_continue_after_approval()`, `rollback()`, `_run_handler_and_persist()`
- Approval flow: `backend/app/recovery/approval.py` — `request_approval()`, `approve()`, `reject()`, `_derive_approver_team()`, `_is_expired()`
- Mock handlers: `backend/app/recovery/handlers/{scale_deployment, kill_query, restart_service, restart_pod, rollback_deployment, refresh_secret, drain_node, clear_cache}.py`
- API endpoints: `backend/app/routers/recovery.py` — actions / dry-run / execute / executions / **approvals/{id}/{approve|reject}** / **executions/{id}/rollback**
- Frontend components: `frontend/src/components/Recovery/{RecoveryActionsSection, DryRunModal, ExecutionsView, ApprovalsView}.tsx`
- Tests: 104 recovery tests in `backend/tests/test_recovery*.py` + 16 frontend tests in `frontend/src/__tests__/{RecoveryActionsSection, DryRunModal, ExecutionsView, ApprovalsView}.test.tsx`

### E2E

`bash scripts/sprint3_e2e_test.sh` runs 8 curl steps against a live API: high_risk submit → approval list → approve → duplicate-approve 409 → low_risk sync → rollback → original marked rolled_back → duplicate-rollback 409.

## OTel Demo Real-Data Connectors — PRD-004

5 asyncio-based connectors poll the vm cluster (otel-demo namespace, OTel demo Helm chart 0.32.0) every 30s and write to DSS. Frontend untouched in this PRD — verification is curl-only.

| Connector | Source | Writes |
|---|---|---|
| `k8s` | kubernetes-asyncio (Deployment/Pod/Service/CM/Secret) | DataNode + DataEdge with `discovery_method=k8s_connector` |
| `prometheus` | OTel Collector spanmetrics (`duration_milliseconds_*`, `calls_total`) | MetricSnapshot + auto-derives component `health` |
| `jaeger` | Jaeger HTTP `/api/traces` (ChildOf span refs) | CALLS edges with `call_count_5m`, threshold ≥ 5 |
| `flagd` | gRPC `/flagd.evaluation.v1.Service/ResolveAll` | ChangeEvent (source=flagd) on flag diff |
| `k8s_events` | K8s events (ScalingReplicaSet / SuccessfulRescale) | ChangeEvent (deployment_rolled) |

### Key Design Decisions

1. **`discovery_method` property** isolates connector-owned data from baseline. Diff-update only touches nodes/edges with the matching method.
2. **First-sync baseline** for flagd / k8s_events: snapshot current state but emit zero events (avoids 100+ ChangeEvents on startup).
3. **Service name normalization**: `_service_to_component_id("cartservice", ...)` → `comp:vm-cluster:otel-demo:cart` (strips "service" suffix). `frauddetectionservice` → `fraud-detection`.
4. **Health derivation in connector**: `derive_health(snapshots)` returns None if no data (don't refresh), `red` if any critical breach, `yellow` if any warning, else `green`. Critical beats warning across metrics.
5. **PromQL window 5m**: shorter windows return 0 results because OTel collector pushes spanmetrics at scrape interval too long for `rate()` over 2m.
6. **Jaeger base-path**: Helm chart 0.32.0 sets `--query.base-path=/jaeger/ui`, so API is at `/jaeger/ui/api/services` not `/api/services` (default `JAEGER_URL` reflects this).
7. **CALLS edge threshold ≥ 5**: filter noise from one-off cross-service calls. Self-calls excluded.
8. **8 fault scenarios** (`backend/app/recovery/scenarios/otel_demo_scenarios.py`): map flag name (`productCatalogFailure` / `cartServiceFailure` / etc.) → target component → recommended PRD-001 action (restart_pod / clear_cache / scale_deployment / rollback_deployment / restart_service).

### File Map

- BaseConnector: `backend/app/datasource/connectors/base.py` — abstract `sync_once()`, swallowed exceptions, `status()` for control endpoint
- K8s: `k8s_connector.py` (kubernetes-asyncio loops + `_index_rs_to_deploy`) + `k8s_mapper.py` (pure-function mapping, `normalize_component_name`, `detect_middleware`, `is_infra`)
- Prometheus: `prometheus_connector.py` + `prometheus_queries.py` (3 PromQL templates, QueryDef thresholds) + `health_rules.py` (warn/critical → green/yellow/red)
- Jaeger: `jaeger_connector.py` + `trace_aggregator.py` (counts CHILD_OF span pairs across traces)
- flagd: `flagd_connector.py` (`_extract_value` for boolValue/doubleValue/stringValue/intValue, `_state_differs` by variant)
- K8s events: `k8s_event_connector.py` (`INTERESTING_REASONS`, `_event_to_change` with ReplicaSet → Deployment name strip)
- Orchestrator: `sync_orchestrator.py` (`registry`, `init_connectors`, `start_all_connectors`, `stop_all_connectors`)
- Tests: `backend/tests/test_sprint23_connectors.py` (39 tests via `asyncio.run()` + `httpx.AsyncClient` patched with AsyncMock)

### Endpoints (`/api/v1/connectors`)

- `GET /status` — list all connectors with `running`, `error_count_24h`, `last_result`
- `GET /{name}` — single connector status detail
- `POST /{name}/sync-now` — trigger one sync, return SyncResult

### E2E

`bash scripts/otel_demo_e2e.sh` checks 5 connectors registered → forces sync on each → checks DSS for nodes/edges/metrics/CALLS/ChangeEvents → lists 8 OTel demo scenarios. Requires port-forwards to Prometheus (19090), Jaeger (16686), flagd (8013) and API started with `KUBECONFIGS / PROMETHEUS_URL / JAEGER_URL / FLAGD_URL` env vars.

## Change Event Tracking — PRD-002 Sprint 1 + Sprint 2

ChangeEvent is a typed event recording **what was changed by whom on which resource at what time**. Sprint 1 shipped backend (model + correlated query + propagation BFS); Sprint 2 adds **Neo4j dual-write** (audit survives uvicorn restart) + **frontend timeline** (3 integration points).

- 4 change types: `configmap_updated` / `secret_rotated` / `deployment_rolled` / `image_pushed`
- Sources: `k8s_api` / `argo_cd` / `gitops` / `manual` / `unknown` / `flagd` (added by PRD-004 Sprint 3)
- Propagation: `derive_propagation(target_id)` does reverse BFS on PROPAGATION_EDGES (USES, CONTAINS, DEPLOYED_AS, BELONGS_TO, RUNS, SCHEDULED_ON, EXPOSES, ROUTES_TO), capped at depth 4
- `severity_estimate`: `len(propagated) >= 10` → high, ≥ 5 → medium, else low
- Endpoints (`/api/v1/change-events`): POST create / GET list with filters / GET `/correlated?target_resource_id=X&window=300` / GET `/{id}/impact` / GET `/timeline?application_id=Y`

### Sprint 2 — Neo4j Dual-Write

`record_change()` writes DSS (主存储) then best-effort dual-writes Neo4j, mirroring the recovery `_persist_execution()` pattern. Neo4j failure → `logger.warning` only, never blocks the API.

- Node: `MERGE (:ChangeEvent:ResourceInstance {node_id: $eid})` (dual-label, keyed by `change_event_id`)
  - `diff_summary` stored as `diff` (JSON-serialized string, `json.dumps(..., ensure_ascii=False, sort_keys=True)`)
  - `propagated_to` stored natively as a Neo4j list property + `pc` count for fast indexing
- Edge: `MERGE (e)-[:RELATES_TO {edge_id:'change_target_'+$eid}]->(t)` with `relationship_type='CHANGED'`. Target matched via `MATCH (t:ResourceInstance {node_id:$tid})` — if absent the edge is skipped (no stub node), only the edge is lost.
- **No PROPAGATES_TO fan-out edges** (decision): `propagated_to` as a list property is queryable without write amplification on high-severity events.

CSV bulk import: `scripts/import_change_events.py` reads `scripts/output/change_events.csv` (`generate_change_events.py --csv`, ~150 events) and UNWIND-MERGEs nodes + main edges in batches of 200. Run via `cd backend && uv run python ../scripts/import_change_events.py` (needs the uv env for the neo4j driver).

### Sprint 2 — Frontend Timeline (3 integration points)

1. **NodeDetailPanel** — `ChangeTimelineSection` Card after the recovery section: antd `<Timeline>` of the resource's last 50 changes, severity-colored dots (low=green / medium=gold / high=red) + Chinese change-type labels. Drawer widened 380→460.
2. **`/change-timeline` page** (`ChangeTimelineView`) — application-level timeline with range presets (1h/6h/24h/7d), type checkboxes, `by_type` Tag aggregation, and a detail Drawer rendering the `/{id}/impact` tree via antd `Tree`. Menu entry `变更时间线` (`FieldTimeOutlined`), 5s refetch.
3. **ConfigImpactView** — right-side `近 24h 变更资源` Card (280px, flex): aggregates 24h changes onto visible graph nodes, top 20 by count, click selects the node.

### Sprint 2.5 — 从变更直接调起恢复动作(集成 PRD-001)

`prd-002 §9` 点名的 Phase 2 项「变更回滚(从此处直接调起 PRD-001 rollback)」。在变更事件抽屉里展示推荐恢复动作 + 一键发起执行,把"看到变更"贯通到"执行恢复"。

- 后端 `CHANGE_ACTION_SUGGESTIONS`(`action_defs.py`)按 `change_type` 推荐动作,镜像 `RULE_ACTION_SUGGESTIONS` 结构:`configmap_updated`→`rollback_deployment` / `secret_rotated`→`refresh_secret`+`rollback_deployment` / `deployment_rolled`→`rollback_deployment` / `image_pushed`→`rollback_deployment`
- 目标解析 `get_recovery_suggestion(event_id)`(`event_service.py`):事件 target 类型与动作 `target_type` 匹配 → `direct`;否则在已算好的 `propagated_to`(反向 BFS)里找第一个类型匹配节点 → `propagated`(例:ConfigMap 变更 → 找到 USES 它的 Deployment);都不可达 → `unresolved`(`resolved_target_resource_id=null`,前端禁用执行按钮)
- 端点 `GET /api/v1/change-events/{id}/recovery-suggestion`
- 前端 `RecoverySuggestionCard`(`ChangeTimelineView.tsx`)挂在事件抽屉底部:展示动作名 / risk / 置信度 / 目标解析 tag + `发起` 按钮调 `postRecoveryExecute`(high_risk → awaiting_approval 提示去审批中心)。unresolved 时按钮 disabled 并提示手动指定

### File Map

- Backend: `backend/app/changes/event_service.py` (`record_change` + `_persist_change_event` + `get_recovery_suggestion`), `backend/app/recovery/action_defs.py` (`CHANGE_ACTION_SUGGESTIONS` + `suggest_for_change`)
- API: `backend/app/routers/change_event.py`
- Frontend: `frontend/src/components/Graph/ChangeTimelineSection.tsx`, `frontend/src/components/Views/ChangeTimelineView.tsx` (含 `RecoverySuggestionCard`), `frontend/src/components/Views/ConfigImpactView.tsx`, `frontend/src/components/Graph/NodeDetailPanel.tsx`, `frontend/src/api/client.ts`
- Tests: `backend/tests/test_change_events.py` (47 tests incl. 3 Neo4j persistence + 7 recovery-suggestion) + `frontend/src/__tests__/{ChangeTimelineSection,ChangeTimelineView}.test.tsx` (10 tests)
- Mock generator: `scripts/generate_change_events.py` (~150 events across 7 days); bulk import: `scripts/import_change_events.py`

## Self-Inspection Report — PRD-003 Sprint 1 + Sprint 2

一键生成自检报告(Markdown)。Sprint 1 上线 `application_health` 模板 + 异步生成;**Sprint 2** 加 `cluster_overview` + `incident_report` 模板 + APScheduler cron 订阅 + SMTP 邮件 + Neo4j 订阅持久化。**PDF / matplotlib 图表 / IM 推送 延后**(决策:报告主读者是工程团队,Markdown 够用)。

### 数据源适配(重要)

PRD §3.4 假设复用 `inspection_service` / `alert_service` —— **这两个不存在**。`services/` 只有 `graph_service`(Neo4j 记录格式化器)+ `metrics_service`。View routers 是纯 Neo4j、测试态 mock 返空。→ **报告所有模块全部从 DSS store 采集**。`InspectionFinding`/`AlertEvent` 在 DSS 无对应模型,Health Score 公式从「节点 health_status + 活跃故障」适配(Phase 2 接真实巡检 finding 切回 PRD 原公式)。

### Health Score 适配公式

PRD 原公式(critical Finding -10 / warning -3 / fault Pod -2)→ DSS 适配:
- `critical = red-health 节点数 + 活跃 fault 目标数` ×10
- `warning = yellow-health 节点数` ×3
- `fault_pod = 活跃 fault 中 Pod 类目标数` ×2
- `score = max(0, 100 - critical*10 - warning*3 - fault_pod*2)`
- rating:`≥80 健康 / 60-79 健康警告 / 40-59 风险中 / <40 风险高`

### 模板与模块

**`application_health`(Sprint 1)** — 5 模块:health_score / seven_views / risk_list / recommended_actions / historical_trends。scope 必含 `application_id`。

**`cluster_overview`(Sprint 2)** — 4 模块 + 跨应用聚合:
- `cluster_health` — 列所有 Application,逐个 `compute_health_score`,按 score 升序 + rating 分布
- `cluster_risk_top_n` — Top-N 风险应用 + 全局活跃故障 + 高危变更计数
- `cluster_changes` — by_type 变更聚合 + Top-5 受变更资源
- `cluster_recoveries` — RecoveryExecution status 分布 + 成功率
- scope 可空(全公司)或 `cluster_id`(L1 模型反向 BFS 不通,简化为 `resource_id` prefix 匹配,Phase 2 重做)

**`incident_report`(Sprint 2)** — 3 模块,围绕单个事件锚点:
- 锚点解析:`scope.fault_id`(DSS FaultInjection)或 `change_event_id`(DSS ChangeEvent),二选一;失败 → `ValueError` → generator failed 分支
- `incident_summary` — 锚点元信息 + 反向 BFS 受影响节点(`derive_propagation` 复用 PRD-002)
- `incident_timeline` — 锚点 ±window_seconds(默认 3600s)内交叉 ChangeEvent + RecoveryExecution,按时间排序(返回 key 用 `events` 不是 `items` — Jinja2 与 dict.items() 冲突)
- `incident_recoveries` — 已执行恢复 + 推荐后续(change 锚点调 `suggest_for_change`)

### 异步生成

- `generate_report(report_id)` 同步函数:按 `template_id` 路由到 `gatherers_for_template()` 对应表 → 顺序调采集函数(每步更新 progress/current_step)→ Jinja2 渲染 `{template_id}.md` → 落盘 `backend/reports/{id}.md` → completed;异常 → failed + error_message
- `run_generation_background(report_id)` 包 `threading.Thread`(daemon)。**测试直接调同步 `generate_report` 避免线程 flaky**
- `report_store` 单例(对标 DSS `store`)hold 所有 ReportTask;uvicorn 重启任务丢失(产物在磁盘)

### 订阅 + 调度(Sprint 2)

- `ReportSubscription` dataclass + `subscription_store` 单例(`backend/app/reports/subscription_store.py`)— 字段:template_id / scope / modules / cron / recipients / enabled / last_run_at / last_status / last_error / last_report_id
- `ReportScheduler` 包 APScheduler `BackgroundScheduler`(`backend/app/reports/scheduler.py`)— `register_subscription` 用 `CronTrigger.from_crontab` 注册 cron job;`unregister` / `reload_all` / `trigger_now`
- job 触发 → `_run_subscription_safely(sub_id)`:读 sub → 创建一次性 ReportTask → 同步 `generate_report` → 调 `EmailSender.send(recipients, subject, body=markdown, attachments=[.md])` → 更新 `last_*`
- `EmailSender` 抽象(`backend/app/reports/email_sender.py`):`InMemoryEmailSender`(默认 / 测试,sent 列表累加)+ `SmtpEmailSender`(stdlib smtplib,`SMTP_HOST` env 切真 SMTP)+ `get_email_sender()` 单例工厂
- 启动时 `load_subscriptions_from_neo4j()` 反向 hydrate + `report_scheduler.start()` + `reload_all()`(`backend/app/main.py` lifespan)

### Neo4j 订阅持久化(Sprint 2)

`backend/app/reports/persistence.py` 镜像 `_persist_change_event` best-effort 模式:
- `_persist_subscription(sub)` — `MERGE (:ReportSubscription:ResourceInstance {node_id: $sid})`,scope 用 JSON str(`scope_json`)、modules / recipients 原生 list
- `_delete_subscription_node(sub_id)` — `DETACH DELETE`
- `load_subscriptions_from_neo4j()` — `MATCH (:ReportSubscription) RETURN ...` 反向 hydrate;Neo4j 离线 → `logger.warning` 不阻塞启动
- 失败一律 `logger.warning` 不抛(API / 内存 store 不阻塞)

### 端点(`/api/v1/reports`)

报告:
- `POST /generate` → 202 + `{report_id, status:"pending"}`(校验 template_id ∈ 3 个、format ∈ {markdown}、按模板校验 modules 子集、application_id / anchor 必填)
- `GET /{id}/status` → `{status, progress, current_step, error_message}`
- `GET /{id}/download?format=markdown` → FileResponse(.md),非 completed → 409,非 markdown → 400
- `GET /` → 列表(过滤 template_id / application_id)

订阅(Sprint 2):
- `POST /subscriptions`(201)— 校验 cron / recipients / scope → 注册 scheduler → Neo4j dual-write
- `GET /subscriptions` / `GET /subscriptions/{id}`
- `PATCH /subscriptions/{id}` — 改 cron / enabled / recipients / modules,自动重注册 scheduler
- `DELETE /subscriptions/{id}`(204)— scheduler 注销 + Neo4j delete
- `POST /subscriptions/{id}/trigger` — 同步立即跑(发邮件 + 更新 last_*)
- `GET /sent-emails` — 仅 InMemoryEmailSender 模式调试,生产 SMTP → 501

### 前端

- `/reports` 页(`ReportsView`)外层 antd `<Tabs>`:
  - 「报告列表」(`ReportsListPanel`)— Table + 「生成新报告」Modal,模板 Select 切换时动态切换 scope 输入(应用 ID / 集群 ID / fault_id + change_event_id 二选一)+ 同步模块默认值;3s 刷新
  - 「订阅管理」(`SubscriptionsPanel`)— Table(模板/范围/cron Tag/收件人 Tag/最近运行/启用 Switch/操作) + 「新建订阅」Modal(动态 scope + 4 个 cron 预设按钮 + 收件人逗号分隔输入 + 模块多选 + enabled Switch);行操作:立即运行 / Switch 启停 / 删除(Popconfirm);5s 刷新
- `NodeDetailPanel` 仅 Application 节点显示「📄 自检报告」Card,一键生成
- 下载:`utils/download.ts` `downloadBlob`(全项目首个 blob 下载 — createObjectURL + `<a download>` + revoke)

### Key Design Decisions

1. **Markdown-only** —— 不上 weasyprint(PDF 延后)、不上 matplotlib(趋势用文本表格)。原生库已就绪,Phase 2 可平滑切 PDF
2. **DSS 为唯一数据源** —— view routers 纯 Neo4j 测试态返空,报告必须可测,故全走 DSS。Health Score 适配公式是这一选择的直接后果
3. **threading + 内存任务表** —— 不引入 Celery;`report_store` 单例对标 DSS。后台线程测试里用同步调用替代
4. **多模板路由** —— `gatherers_for_template(template_id)` 返回对应 gatherer 字典,延迟 import 避免循环依赖(cluster_modules / incident_modules 反向引用 health_score)
5. **incident 锚点 fault_id / change_event_id 二选一** —— 没有独立 Incident 模型,复用现有 DSS 资源,失败抛 `ValueError` → generator failed 分支
6. **EmailSender 抽象 + 单例工厂** —— 默认 InMemoryEmailSender 不污染线上;`SMTP_HOST` env 切真 SMTP;`reset_email_sender()` 测试用
7. **APScheduler BackgroundScheduler** —— 同步 scheduler 配 FastAPI lifespan 简洁;`trigger_now` 暴露给 API + 测试免起线程
8. **订阅 Neo4j dual-write** —— uvicorn 重启订阅不丢;失败不阻塞主流程(同 `_persist_change_event` 模式)
9. **cluster_id prefix 匹配** —— L1 模型 KubernetesCluster 不直接 CONTAINS Application,反向 BFS 走不通;Sprint 2 简化为 `resource_id` 字符串 prefix,Phase 2 重做
10. **Jinja2 字段名避开 `items`** —— `incident_timeline` 返回 `events` 而非 `items`,避免与 dict.items() 方法冲突

### File Map

- 后端核心:`backend/app/reports/{store,health_score,modules,generator}.py` + `cluster_modules.py` + `incident_modules.py` + `subscription_store.py` + `email_sender.py` + `scheduler.py` + `persistence.py`
- 模板:`backend/app/reports/templates/{application_health,cluster_overview,incident_report}.md`
- API:`backend/app/routers/report.py`(报告 4 端点 + 订阅 7 端点),注册于 `backend/app/main.py`(lifespan startup hydrate + scheduler.start)
- 前端:`frontend/src/components/Views/{ReportsView,SubscriptionsPanel}.tsx`,`frontend/src/components/Graph/NodeDetailPanel.tsx`,`frontend/src/utils/download.ts`,`frontend/src/api/client.ts`
- 测试:`backend/tests/test_reports.py`(21)+ `test_reports_sprint2.py`(22 — cluster/incident/multi-template)+ `test_reports_sprint2_sub.py`(24 — 订阅 / 邮件 / 调度 / persistence)+ `frontend/src/__tests__/{ReportsView,NodeDetailPanelReport,SubscriptionsPanel}.test.tsx`(14)
