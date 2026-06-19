"""Connectors 控制端点 — PRD-004 Sprint 1。

端点设计(curl-only,无前端):
- GET  /api/v1/connectors/status         所有 connector 状态
- POST /api/v1/connectors/{name}/sync-now 手动触发一次 sync,返回 SyncResult
- GET  /api/v1/connectors/{name}         单个 connector 状态详情
"""

from fastapi import APIRouter, HTTPException

from app.datasource.connectors.sync_orchestrator import registry


router = APIRouter(prefix="/api/v1/connectors", tags=["connectors"])


@router.get("/status")
def list_connectors():
    """所有 connector 状态摘要。"""
    return {
        "connectors": [c.status() for c in registry.all()],
        "total": len(registry.all()),
    }


@router.get("/{name}")
def get_connector(name: str):
    c = registry.get(name)
    if c is None:
        raise HTTPException(status_code=404, detail=f"connector not found: {name}")
    return c.status()


@router.post("/{name}/sync-now")
async def sync_now(name: str):
    c = registry.get(name)
    if c is None:
        raise HTTPException(status_code=404, detail=f"connector not found: {name}")
    result = await c.trigger_sync_now()
    return {
        "connector": name,
        "result": {
            "nodes_added": result.nodes_added,
            "nodes_updated": result.nodes_updated,
            "nodes_removed": result.nodes_removed,
            "edges_added": result.edges_added,
            "edges_updated": result.edges_updated,
            "edges_removed": result.edges_removed,
            "metrics_added": result.metrics_added,
            "events_added": result.events_added,
            "duration_ms": result.duration_ms,
            "notes": result.notes,
        },
    }
