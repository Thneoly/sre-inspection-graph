"""AlertEvent API — PRD-004 Phase 2。

端点(prefix `/api/v1/alerts`):
- GET /             列表(过滤 resource_ref / severity / status / 时间窗)
- GET /rules        AlertRule 列表(从 health_rules 阈值生成)
- GET /{id}         单个详情
- POST /{id}/resolve  标记告警 resolved

AlertEvent 由 connector(prometheus critical breach)自动产出,也可手工录入测试。
"""
from __future__ import annotations

from typing import Optional

from fastapi import APIRouter, HTTPException, Query
from pydantic import BaseModel, Field

from app.alerts.alert_service import record_alert, resolve_alert, serialize
from app.datasource.connectors.health_rules import sync_alert_rules_to_store
from app.datasource.store import store


router = APIRouter(prefix="/api/v1/alerts", tags=["Alerts"])


class AlertCreate(BaseModel):
    alert_name: str = Field(..., description="告警名")
    resource_ref: str = Field(..., description="被告警资源 DSS node_id")
    severity: str = Field("critical", description="warning | critical")
    rule_id: str = Field("", description="触发的 AlertRule ID")
    metric_name: str = Field("")
    metric_value: float = Field(0.0)
    summary: str = Field("")
    description: str = Field("")
    cluster_id: str = Field("")


@router.post("", status_code=201)
def create_alert(req: AlertCreate):
    """手工录入告警(测试 / 模拟用)。connector 自动产出的走 alert_service.record_alert。"""
    ev = record_alert(
        alert_name=req.alert_name,
        resource_ref=req.resource_ref,
        severity=req.severity,
        rule_id=req.rule_id,
        metric_name=req.metric_name,
        metric_value=req.metric_value,
        summary=req.summary,
        description=req.description,
        cluster_id=req.cluster_id,
    )
    if ev is None:
        return {"alert_event_id": None, "deduped": True}
    return serialize(ev)


@router.get("")
def list_alerts(
    resource_ref: Optional[str] = Query(None),
    severity: Optional[str] = Query(None),
    status: Optional[str] = Query(None),
    since: Optional[str] = Query(None),
    until: Optional[str] = Query(None),
    limit: int = Query(100, ge=1, le=1000),
):
    events = store.list_alert_events(
        resource_ref=resource_ref,
        severity=severity,
        status=status,
        since=since,
        until=until,
    )
    events.sort(key=lambda e: e.fired_at, reverse=True)
    sliced = events[:limit]
    return {
        "alerts": [serialize(e) for e in sliced],
        "total": len(events),
        "returned": len(sliced),
    }


@router.get("/rules")
def list_rules(enabled: Optional[bool] = Query(None)):
    """AlertRule 列表。首次访问时懒加载到 store。"""
    if not store.alert_rules:
        sync_alert_rules_to_store()
    rules = store.list_alert_rules(enabled=enabled)
    return {
        "rules": [
            {
                "rule_id": r.rule_id,
                "metric_name": r.metric_name,
                "severity": r.severity,
                "threshold": r.threshold,
                "direction": r.direction,
                "unit": r.unit,
                "description": r.description,
                "enabled": r.enabled,
            }
            for r in rules
        ],
        "total": len(rules),
    }


@router.get("/{alert_id}")
def get_alert(alert_id: str):
    event = store.get_alert_event(alert_id)
    if event is None:
        raise HTTPException(status_code=404, detail=f"alert not found: {alert_id}")
    return serialize(event)


@router.post("/{alert_id}/resolve")
def resolve(alert_id: str):
    ev = resolve_alert(alert_id)
    if ev is None:
        raise HTTPException(status_code=404, detail=f"alert not found: {alert_id}")
    return serialize(ev)
