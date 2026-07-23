---
name: port-reference-testing
description: Use when porting reference Python/Cypher logic (views, connectors, graph queries) to Rust. Prevents the recurring bug where reference names (Neo4j labels) drift from Rust's actual resource_type/edge_type vocabulary — verify names against real connector output and add realistic-fixture integration tests, not just synthetic-fixture unit tests.
---

# Porting reference → Rust — type-vocabulary & fixture discipline

This project rewrites a Python reference (`reference/app/`, read-only oracle) into Rust. A **recurring bug class**: names copied from the reference (Neo4j `label` properties, Cypher relationship types) do **not** match Rust's actual vocabulary, and synthetic-fixture unit tests never catch it — it only surfaces at GUI runtime against real data.

## The trap (concrete)

Phase 5 `node-impact` view filtered `list_resources_by_types(["KubernetesNode"])` — copied from reference view3's Cypher `label: 'KubernetesNode'`. But the Rust k8s connector emits `resource_type = "Node"`. Result: empty selector, no nodes shown. The 8 `subgraph` unit tests passed (fixtures used the same wrong name); tsc/vitest/build all green. Only a human looking at the real app caught it.

Root cause: `resource_type` / `edge_type` are **stringly-typed with no canonical registry** — duplicated across the k8s mapper, `facts_to_graph`, frontend `SHAPE_BY_TYPE`, and SQLite. The reference's Neo4j label scheme is a *different* vocabulary.

## Rules when porting reference code

1. **Never trust a reference name.** Neo4j `label` / Cypher rel-type strings are the *reference's* vocabulary, not Rust's. Before hardcoding a type/edge name in Rust or TS, verify it against real connector output:
   - Read the SQLite: `SELECT DISTINCT resource_type FROM topology_nodes` / `SELECT DISTINCT edge_type FROM topology_edges` (app DB at `~/.local/share/io.sregraph.desktop/sre-graph.sqlite` after a sync).
   - Or read the producer: `modules/connectors/k8s/src/mapper.rs` (grep the literal, e.g. `"Node"`).
2. **Add a realistic-fixture integration test for any view→command→data path.** Unit tests with hand-rolled synthetic types (`n("x", "KubernetesNode")`) validate the algorithm but not the vocabulary. Build a fixture mirroring real connector output (real type + edge names) and assert non-empty results. See `engine/crates/engine-identity/src/views.rs` `tests::subgraph_views_against_realistic_k8s_topology` for the pattern.
3. **Cross-check data proofs.** When you run a Python data proof over SQLite (common fallback when GUI capture is blocked), you read *real* type names — reconcile them against the literals hardcoded in the frontend page / command. The Phase 5 bug was catchable this way and was missed.

## Canonical vocabulary (until a proper `ResourceType` enum exists)

Measured against the real cluster (otel-demo via kubectl proxy). Use these exact strings:

- **resource_type**: `Cluster`, `Namespace`, `Node`, `Pod`, `Deployment`, `Service`, `ConfigMap`, `Secret`, `Application`, `ApplicationComponent`, `Container`, `Kafka`, `Redis`
- **edge_type**: `CONTAINS`, `ROUTES_TO`, `SCHEDULED_ON`, `USES`, `BELONGS_TO`, `DEPLOYED_AS`, `EXPOSES`, `RUNS`
  - reference-only (not yet produced by the Rust connector): `DEPLOYED_IN`, `STORED_IN`, `CONTROLLED_BY`, `AFFECTS`, `FIRED_ON`, `USES_IMAGE` — safe to include in whitelists (never match), future-proof.

> Note: reference uses Neo4j labels like `KubernetesNode`, `ContainerImage`, `ResourceInstance` — these are **not** Rust resource_types. Map them: `KubernetesNode → Node`, `ContainerImage → (none yet; connector emits Container via RUNS, not image-level nodes)`.

## When GUI verification is blocked

This session is GNOME **Wayland**; programmatic screenshots of the Tauri webview are blocked (XWayland surface captures black; GNOME Shell D-Bus screenshot returns `AccessDenied`; `scrot`/`xdotool`/`WEBKIT_DISABLE_COMPOSITING_MODE` do not help). When GUI pixel capture is impossible:
- Run a **Python data proof** over the live SQLite replicating the Rust logic (BFS subgraph, etc.) to show exactly what each view/command returns against real cluster data. This is objective runtime evidence even without pixels.
- Have the human at the machine do the visual sign-off (they can see the window).
- Add/keep the realistic-fixture Rust test as the durable guard.

## The real fix (tech debt)

A canonical `ResourceType` / `EdgeType` enum or const-set in `engine-core` (or `engine-identity`), emitted by connectors and referenced by the frontend, kills this class entirely. Not yet done — prefer it for the next refactor that touches the type vocabulary.
