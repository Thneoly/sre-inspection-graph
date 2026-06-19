"""Change Event API — PRD-002 Sprint 1。

端点:
- POST  /api/v1/change-events            录入(测试 / 模拟用)
- GET   /api/v1/change-events            列表(过滤)
- GET   /api/v1/change-events/correlated 故障关联查询
- GET   /api/v1/change-events/timeline   应用维度时间线
- GET   /api/v1/change-events/{id}       单个详情
- GET   /api/v1/change-events/{id}/impact  影响范围 + 路径

注意路由顺序:`/correlated` 和 `/timeline` 必须放在 `/{id}` 之前,否则会被
path 参数捕获。
"""

from typing import Optional

from fastapi import APIRouter, HTTPException, Query
from pydantic import BaseModel, Field

from app.changes.event_service import (
    ChangeEventError,
    application_timeline,
    correlated_changes,
    get_impact,
    record_change,
    serialize,
)
from app.datasource.store import store


router = APIRouter(prefix="/api/v1/change-events", tags=["Change Events"])


# ============================================================
# Pydantic
# ============================================================

class ChangeEventCreate(BaseModel):
    change_type: str = Field(..., description="configmap_updated / secret_rotated / deployment_rolled / image_pushed")
    target_resource_id: str = Field(..., description="被变更资源的 DSS node_id")
    changed_by: str = Field("", description="用户 / 服务账号(可空)")
    source: str = Field("manual", description="k8s_api / argo_cd / gitops / manual / unknown")
    description: str = Field("", description="人读描述")
    diff_summary: dict = Field(default_factory=dict, description="简化 diff,例如 {key: {old, new}}")
    related_commit: str = Field("", description="Git commit hash")
    related_pr: str = Field("", description="PR URL")
    changed_at: Optional[str] = Field(None, description="ISO8601;省略则用当前时刻")


# ============================================================
# 端点
# ============================================================

@router.post("", status_code=201)
def create_change_event(req: ChangeEventCreate):
    try:
        event = record_change(
            change_type=req.change_type,
            target_resource_id=req.target_resource_id,
            changed_by=req.changed_by,
            source=req.source,
            description=req.description,
            diff_summary=req.diff_summary,
            related_commit=req.related_commit,
            related_pr=req.related_pr,
            changed_at=req.changed_at,
        )
    except ChangeEventError as e:
        raise HTTPException(status_code=e.code, detail=str(e))
    return serialize(event)


@router.get("")
def list_change_events(
    change_type: Optional[str] = Query(None),
    target_resource_id: Optional[str] = Query(None),
    source: Optional[str] = Query(None),
    since: Optional[str] = Query(None, description="ISO8601 起始(含)"),
    until: Optional[str] = Query(None, description="ISO8601 终止(含)"),
    limit: int = Query(100, ge=1, le=1000),
):
    events = store.list_change_events(
        change_type=change_type,
        target_resource_id=target_resource_id,
        source=source,
        since=since,
        until=until,
    )
    events.sort(key=lambda e: e.changed_at, reverse=True)
    sliced = events[:limit]
    return {
        "events": [serialize(e) for e in sliced],
        "total": len(events),
        "returned": len(sliced),
    }


@router.get("/correlated")
def correlated(
    target_resource_id: str = Query(..., description="目标资源 ID(被查的资源,不是变更资源)"),
    window: int = Query(300, ge=1, le=86400, description="时间窗口秒数"),
    since: Optional[str] = Query(None, description="ISO8601 起始;省略则用 now-window"),
    until: Optional[str] = Query(None, description="ISO8601 终止;省略则用 since+window 或 now"),
    include_propagated: bool = Query(True, description="是否包含通过依赖路径间接影响的事件"),
):
    return correlated_changes(
        target_resource_id=target_resource_id,
        window_seconds=window,
        since=since,
        until=until,
        include_propagated=include_propagated,
    )


@router.get("/timeline")
def timeline(
    application_id: str = Query(..., description="Application 节点 ID"),
    since: Optional[str] = Query(None),
    until: Optional[str] = Query(None),
):
    try:
        return application_timeline(application_id, since=since, until=until)
    except ChangeEventError as e:
        raise HTTPException(status_code=e.code, detail=str(e))


@router.get("/{change_event_id}")
def get_change_event(change_event_id: str):
    event = store.get_change_event(change_event_id)
    if event is None:
        raise HTTPException(status_code=404, detail=f"change_event not found: {change_event_id}")
    return serialize(event)


@router.get("/{change_event_id}/impact")
def event_impact(change_event_id: str):
    try:
        return get_impact(change_event_id)
    except ChangeEventError as e:
        raise HTTPException(status_code=e.code, detail=str(e))
