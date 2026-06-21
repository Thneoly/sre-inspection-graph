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
make test           # Backend 288 tests + Frontend 46 tests
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

### File Map

- Backend: `backend/app/changes/event_service.py` (`record_change` + `_persist_change_event`)
- API: `backend/app/routers/change_event.py`
- Frontend: `frontend/src/components/Graph/ChangeTimelineSection.tsx`, `frontend/src/components/Views/ChangeTimelineView.tsx`, `frontend/src/components/Views/ConfigImpactView.tsx`, `frontend/src/components/Graph/NodeDetailPanel.tsx`, `frontend/src/api/client.ts`
- Tests: `backend/tests/test_change_events.py` (40 tests incl. 3 Neo4j persistence) + `frontend/src/__tests__/{ChangeTimelineSection,ChangeTimelineView}.test.tsx` (8 tests)
- Mock generator: `scripts/generate_change_events.py` (~150 events across 7 days); bulk import: `scripts/import_change_events.py`
