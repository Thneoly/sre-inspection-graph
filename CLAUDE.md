# CLAUDE.md

## Project Overview

SRE 云原生巡检图谱平台 — cloud-native resource inspection graph platform based on a 4-layer Neo4j model with fault simulation.

## Architecture

```
L1 Resource Type Graph  →  14 type nodes + 35 relationships (static, in CSV)
L2 Resource Instance Graph → application/component/Deployment/Pod/middleware instances
L3 Dynamic Observability → MetricQuery + MetricSnapshot + AlertEvent
L4 Inspection Results → InspectionRun/Rule/Finding

+ Data Source Service (DSS) → in-memory cache layer, decouples fault injection from Neo4j
```

## Tech Stack

- **Graph DB**: Neo4j 5 (Docker, bolt://localhost:7687, neo4j/sre-inspection)
- **Backend**: Python 3.12 + FastAPI + uv (`backend/`)
- **Frontend**: React 18 + TypeScript + Cytoscape.js + Ant Design 5 + Vite (`frontend/`)
- **Deployment**: Docker Compose

## Commands

```bash
# Development
make infra          # Start Neo4j only (docker compose up -d neo4j neo4j-init)
make dev-api        # API hot-reload on port 8000
make dev-frontend   # Frontend HMR on port 3000

# Full stack
make up             # Docker Compose start all
make down           # Stop all
make clean          # Remove containers + volumes + generated data

# Testing
make test           # Backend 53 tests + Frontend 16 tests
make test-cov       # Backend coverage report

# Mock data
make mock-data      # Generate CSV + Cypher → scripts/output/
```

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
│   │   ├── models.py        # DataNode, DataEdge, MetricSnapshot, FaultInjection
│   │   ├── store.py         # In-memory singleton stores
│   │   ├── loader.py        # Load baseline from Neo4j → DSS
│   │   └── fault_injector.py # Fault injection via DSS
│   ├── routers/
│   │   ├── topology.py, access_link.py, node_impact.py, config_impact.py,
│   │   │   image_risk.py, alert_aggregation.py, health.py
│   │   ├── simulation.py    # Legacy fault simulation (direct Neo4j writes)
│   │   └── datasource.py    # DSS REST API (extraction + injection)
│   ├── models/              # Pydantic: GraphNode, GraphEdge, GraphResponse, metrics
│   └── services/            # graph_service.py, metrics_service.py
└── tests/
    ├── conftest.py          # Mock Neo4j fixtures + FastAPI TestClient
    ├── mocks.py             # MockNeo4jNode, MockNeo4jRel, MockNeo4jPath
    ├── test_services.py     # 25 tests for graph_service + metrics_service
    ├── test_queries.py      # 15 tests for 6 view Cypher queries
    └── test_routers.py      # 13 tests for API endpoints

frontend/src/
├── components/
│   ├── Graph/               # GraphCanvas (Cytoscape), NodeDetailPanel, LayerToggle
│   ├── Views/               # 6 inspection views + SimulationView
│   └── Layout/              # MainLayout (antd Layout + Sider)
├── utils/
│   ├── graphStyles.ts       # Node shapes, health colors (green/yellow/red), edge styles
│   ├── layers.ts            # Layer definitions + filterGraphData()
│   └── resourceIcons.ts     # SVG icons for node types (unused in current design)
├── api/client.ts            # Axios client + TypeScript types
└── __tests__/               # 16 vitest tests

scripts/
├── generate_all_mock_data.py   # Generate L3/L4 mock CSV + Cypher
├── generate_l3_mock_data.py    # L3: Pod/Container/Node/MetricQuery/Snapshot
├── generate_l4_mock_data.py    # L4: InspectionRun/Rule/Finding/AlertEvent
├── fault_simulation.py         # CLI fault injector (legacy, direct Neo4j)
├── add_infra_nodes.py          # Add Region/AZ/ELB/Gateway/Nacos/MySQL/Redis/Kafka
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

- **ellipse**: Pod, Container
- **diamond**: Service, Ingress, ELB, Gateway, APIG, Nacos
- **hexagon**: KubernetesCluster, KubernetesNode, ContainerRegistry, Region, AZ
- **rectangle**: Deployment, Namespace, ContainerImage, MySQL, Redis, Kafka, Dashboard
- **round-rectangle**: Application, ApplicationComponent, Environment, InspectionRun
- **parallelogram**: ConfigMap, Secret
- **triangle**: AlertRule, AlertEvent
- **tag**: InspectionFinding, InspectionRule

## Fault Types (7)

cpu_spike, memory_leak, pod_crashloop, node_disk_pressure, service_no_endpoints, mysql_slow_query, redis_unavailable
