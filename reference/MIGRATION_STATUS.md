# Migration Status — Python reference → Rust engine

> 每复刻一个模块更新一次。每行追到 commit SHA + contract test 文件。

## 总览

| Phase | 范围 | 目标完成 | 状态 |
|---|---|---|---|
| Phase 1 | 工作台 + 第一个 WASM connector PoC | T+2mo(2026-08) | 🚧 启动中 |
| Phase 2 | PRD-005 UTS(Fact 总线 + Identity Resolver)| T+5mo(2026-11) | ⏳ |
| Phase 3 | PRD-001 + PRD-002 复刻 | T+9mo(2027-03) | ⏳ |
| Phase 4 | PRD-003 + PRD-004 复刻 | T+12mo(2027-06) | ⏳ |
| Phase 5 | v1.0 release + `git rm reference/` | T+14mo(2027-08) | ⏳ |

## 模块复刻表

| Python 模块 | 文件 | Rust 目标 crate | 状态 | Contract test | Commit |
|---|---|---|---|---|---|
| **PRD-001 — Recovery Action Engine** | | | | | |
| Action defs | `app/recovery/action_defs.py` | `engine-recovery::action_defs` | ⏳ | — | — |
| Cascade BFS | `app/recovery/cascade.py` | `engine-recovery::cascade` | ⏳ | — | — |
| Execution lifecycle | `app/recovery/execution.py` | `engine-recovery::execution` | ⏳ | — | — |
| Approval flow | `app/recovery/approval.py` | `engine-recovery::approval` | ⏳ | — | — |
| 8 mock + real handlers | `app/recovery/handlers/*.py` | WASM `handlers/*` | ⏳ | — | — |
| Verifiers | `app/recovery/verifiers.py` | `engine-recovery::verifiers` | ⏳ | — | — |
| Chain orchestrator | `app/recovery/chains.py` | `engine-recovery::chains` | ⏳ | — | — |
| K8s/MySQL/Redis clients | `app/datasource/connectors/k8s_client.py`, `app/recovery/clients/*` | WASM `connectors/k8s` + handlers | ⏳ | — | — |
| **PRD-002 — Change Event Tracking** | | | | | |
| Event service | `app/changes/event_service.py` | `engine-changes::event_service` | ⏳ | — | — |
| Propagation BFS | `app/changes/propagation.py` | `engine-changes::propagation` | ⏳ | — | — |
| YAML diff | `app/changes/yaml_diff.py` | `engine-changes::yaml_diff` | ⏳ | — | — |
| Frequency alert | `app/changes/frequency.py` | `engine-changes::frequency` | ⏳ | — | — |
| Alert correlation | `app/changes/alert_correlation.py` | `engine-changes::alert_correlation` | ⏳ | — | — |
| K8s watcher | `app/datasource/connectors/k8s_watch_connector.py` | WASM `connectors/k8s-watch` | ⏳ | — | — |
| Webhook receiver | `app/routers/webhook.py` | `engine-cli::webhook` or Tauri command | ⏳ | — | — |
| **PRD-003 — Self-Inspection Report** | | | | | |
| Health score | `app/reports/health_score.py` | `engine-reports::health_score` | ⏳ | — | — |
| Module gatherers | `app/reports/modules.py`, `cluster_modules.py`, `incident_modules.py` | `engine-reports::modules` | ⏳ | — | — |
| Template engine | `app/reports/generator.py` + Jinja2 templates | `engine-reports::generator` + Tera | ⏳ | — | — |
| Subscription scheduler | `app/reports/scheduler.py` | `engine-reports::scheduler`(tokio-cron) | ⏳ | — | — |
| Email sender | `app/reports/email_sender.py` | `engine-reports::email_sender`(lettre) | ⏳ | — | — |
| **PRD-004 — Connectors** | | | | | |
| BaseConnector | `app/datasource/connectors/base.py` | `engine-wasm` host runtime | ⏳ | — | — |
| K8s connector | `app/datasource/connectors/k8s_connector.py` | WASM `connectors/k8s` | ⏳ | — | — |
| Prometheus connector | `app/datasource/connectors/prometheus_connector.py` | WASM `connectors/prometheus` | ⏳ | — | — |
| Jaeger connector | `app/datasource/connectors/jaeger_connector.py` | WASM `connectors/jaeger` | ⏳ | — | — |
| flagd connector | `app/datasource/connectors/flagd_connector.py` | WASM `connectors/flagd` | ⏳ | — | — |
| K8s event connector | `app/datasource/connectors/k8s_event_connector.py` | WASM `connectors/k8s-events` | ⏳ | — | — |
| Sync orchestrator | `app/datasource/connectors/sync_orchestrator.py` | `engine-wasm::registry` | ⏳ | — | — |
| **L1-L4 + DSS 底座** | | | | | |
| Data models | `app/datasource/models.py` | `engine-core::models` + Arrow schema(specs/arrow) | ⏳ | — | — |
| DSS store | `app/datasource/store.py` | `engine-core::store`(Arrow + DataFusion) | ⏳ | — | — |
| Neo4j loader | `app/datasource/loader.py` | `engine-storage::neo4j`(可选)| ⏳ | — | — |
| Fault injector | `app/datasource/fault_injector.py` | `engine-core::fault_injector` | ⏳ | — | — |
| **6 巡检视图(纯查询)** | | | | | |
| Topology view | `app/routers/topology.py` + `app/db/queries/topology.cypher` | `engine-core::queries::topology` | ⏳ | — | — |
| Access link | `app/routers/access_link.py` | `engine-core::queries::access_link` | ⏳ | — | — |
| Node impact | `app/routers/node_impact.py` | `engine-core::queries::node_impact` | ⏳ | — | — |
| Config impact | `app/routers/config_impact.py` | `engine-core::queries::config_impact` | ⏳ | — | — |
| Image risk | `app/routers/image_risk.py` | `engine-core::queries::image_risk` | ⏳ | — | — |
| Alert aggregation | `app/routers/alert_aggregation.py` | `engine-core::queries::alert_aggregation` | ⏳ | — | — |

## 状态图例

- ⏳ 未开始
- 🚧 进行中(挂 in-progress PR)
- 🟡 Rust 实现就绪,contract test 尚未全绿
- ✅ 复刻完成(contract test 全绿,且 reference/ 对应模块停用)
- ❌ 决策不复刻(写明 why)

## 进度统计

- 总模块数:**33**(L1-L4 + PRD-001/002/003/004)
- 已复刻:**0**
- 进行中:**0**
- 完成率:**0%**

---

**约定**

1. 每完成一个模块,在「Commit」列填 commit SHA,「Contract test」列填 `tests/contract/parity_<module>.rs` 路径
2. 一次只复刻一个模块。完成 + contract test 全绿后才能开下一个
3. Phase 5 完工条件:此表所有行 ✅,且 30 天无 reference/ 读访问
