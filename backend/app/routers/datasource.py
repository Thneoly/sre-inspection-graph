"""Data Source Service REST API — 数据提取 + 数据注入"""
from fastapi import APIRouter, Query, HTTPException
from pydantic import BaseModel
from app.datasource.store import store, DataNode, DataEdge, MetricSnapshot
from app.datasource.loader import load_baseline, reset_dss, sync_to_neo4j
from app.datasource.fault_injector import inject, step, reset, FAULT_DEFS

router = APIRouter(prefix="/api/v1/datasource", tags=["DataSource"])


class NodeUpdate(BaseModel):
    properties: dict

class EdgeUpdate(BaseModel):
    properties: dict

class InjectRequest(BaseModel):
    fault_type: str
    target_id: str


# ═══════════════════════════════════════════
# 数据提取接口（巡检展示系统使用）
# ═══════════════════════════════════════════

@router.get("/nodes")
def get_nodes():
    nodes = store.get_all_nodes()
    return {
        "nodes": [
            {"id": n.id, "type": n.type, "name": n.name, "properties": n.properties}
            for n in nodes
        ],
        "total": len(nodes),
    }


@router.get("/nodes/{node_id}")
def get_node(node_id: str):
    n = store.get_node(node_id)
    if not n:
        raise HTTPException(404, "Node not found")
    return {"id": n.id, "type": n.type, "name": n.name, "properties": n.properties}


@router.get("/edges")
def get_edges():
    edges = store.get_all_edges()
    return {
        "edges": [
            {
                "id": e.id, "source": e.source_id, "target": e.target_id,
                "type": e.relationship_type, "properties": e.properties,
            }
            for e in edges
        ],
        "total": len(edges),
    }


@router.get("/metrics/{resource_id}")
def get_metrics(resource_id: str, n: int = Query(default=20)):
    snaps = store.get_metrics(resource_id, n)
    return {
        "resource_id": resource_id,
        "metrics": [
            {
                "id": s.snapshot_id, "metric_name": s.metric_name,
                "current_value": s.current_value, "unit": s.unit,
                "fetched_at": s.fetched_at,
                "warning_breached": s.warning_breached,
                "critical_breached": s.critical_breached,
            }
            for s in snaps
        ],
    }


@router.get("/topology/{app_code}")
def get_topology(app_code: str, depth: int = Query(default=5, ge=1, le=10)):
    """应用拓扑（从 DSS 内存计算 BFS）"""
    app_id = f"app:{app_code}"
    if app_id not in store.nodes:
        raise HTTPException(404, f"App '{app_code}' not found")

    visited_nodes: set[str] = set()
    visited_edges: set[str] = set()
    queue = [(app_id, depth)]

    while queue:
        nid, d = queue.pop(0)
        if nid in visited_nodes or d <= 0:
            continue
        visited_nodes.add(nid)
        for e in store.get_all_edges():
            if e.source_id == nid:
                visited_edges.add(e.id)
                queue.append((e.target_id, d - 1))
            elif e.target_id == nid:
                visited_edges.add(e.id)
                queue.append((e.source_id, d - 1))

    nodes = [store.nodes[nid] for nid in visited_nodes if nid in store.nodes]
    edges = [store.edges[eid] for eid in visited_edges if eid in store.edges]

    # Summary
    risk = {"high": 0, "medium": 0, "low": 0}
    health = {"normal": 0, "warning": 0, "critical": 0}
    for n in nodes:
        r = n.properties.get("risk_level", "low")
        h = n.properties.get("health_status", "normal")
        risk[r] = risk.get(r, 0) + 1
        health[h] = health.get(h, 0) + 1

    return {
        "nodes": [{"id": n.id, "type": n.type, "label": n.type, "properties": n.properties} for n in nodes],
        "edges": [{"id": e.id, "source": e.source_id, "target": e.target_id, "type": e.relationship_type, "properties": e.properties} for e in edges],
        "summary": {"total_nodes": len(nodes), "total_edges": len(edges), "risk_counts": risk, "health_counts": health},
    }


# ═══════════════════════════════════════════
# 数据注入接口（故障注入系统使用）
# ═══════════════════════════════════════════

@router.patch("/nodes/{node_id}")
def patch_node(node_id: str, update: NodeUpdate):
    store.update_node_props(node_id, **update.properties)
    return {"status": "ok", "node_id": node_id}


@router.patch("/edges/{edge_id}")
def patch_edge(edge_id: str, update: EdgeUpdate):
    store.update_edge_props(edge_id, **update.properties)
    return {"status": "ok", "edge_id": edge_id}


@router.get("/fault-types")
def fault_types():
    return {"types": {k: {"name": v["name"], "target_type": v["target_type"]} for k, v in FAULT_DEFS.items()}}


@router.post("/inject-fault")
def inject_fault(inj: InjectRequest):
    fault = inject(inj.fault_type, inj.target_id)
    if not fault:
        raise HTTPException(400, f"Unknown fault type: {inj.fault_type}")
    return {"status": "ok", "injection_id": fault.injection_id, "stages": fault.total_stages}


@router.post("/step")
def step_time(seconds: int = Query(default=60, ge=10, le=3600)):
    n = step(seconds)
    return {"status": "ok", "seconds": seconds, "updated": n}


@router.get("/fault-status")
def fault_status():
    active = store.get_active_faults()
    unhealthy = [{"id": n.id, "type": n.type, "health": n.properties.get("health_status", "?"), "risk": n.properties.get("risk_level", "?")} for n in store.get_all_nodes() if n.properties.get("health_status", "normal") not in ("normal", "?")]
    return {
        "active_count": len(active),
        "active": [{"id": f.injection_id, "type": f.fault_type, "status": f.status, "target": f.target_id, "stage": f.current_stage, "total": f.total_stages} for f in active],
        "unhealthy_nodes": unhealthy,
    }


@router.post("/reset")
def reset_all():
    reset()
    return {"status": "ok"}


# ═══════════════════════════════════════════
# 数据管理
# ═══════════════════════════════════════════

@router.post("/init")
def init_dss():
    load_baseline()
    return {"status": "ok", "nodes": len(store.nodes), "edges": len(store.edges)}


@router.post("/sync")
def sync_dss():
    sync_to_neo4j()
    return {"status": "ok"}
