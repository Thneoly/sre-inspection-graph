"""自检报告 API — PRD-003 Sprint 1 + Sprint 2。

端点(prefix /api/v1/reports):

报告:
- POST   /generate                生成报告(异步,立即返回 pending)
- GET    /{id}/status             查询生成状态
- GET    /{id}/download            下载 Markdown
- GET    /                         报告列表

订阅(Sprint 2):
- POST   /subscriptions           创建订阅 + 注册 scheduler + Neo4j dual-write
- GET    /subscriptions            订阅列表
- GET    /subscriptions/{id}       订阅详情
- PATCH  /subscriptions/{id}       改 cron / 启停 / 收件人
- DELETE /subscriptions/{id}       注销 + Neo4j delete
- POST   /subscriptions/{id}/trigger  立即跑一次
- GET    /sent-emails              调试 — 仅 InMemoryEmailSender 模式

Sprint 1/2 只支持 Markdown 输出。PDF / IM 推送 / weasyprint / matplotlib 留 Phase 2。
"""

from __future__ import annotations

from typing import Optional

from fastapi import APIRouter, HTTPException, Query
from fastapi.responses import FileResponse
from pydantic import BaseModel, Field

from app.reports.email_sender import InMemoryEmailSender, get_email_sender
from app.reports.generator import new_report_id, run_generation_background
from app.reports.persistence import (
    _delete_subscription_node,
    _persist_subscription,
)
from app.reports.scheduler import report_scheduler
from app.reports.store import (
    ReportTask,
    VALID_TEMPLATES,
    modules_for_template,
    report_store,
)
from app.reports.subscription_store import (
    ReportSubscription,
    new_subscription_id,
    subscription_store,
)


router = APIRouter(prefix="/api/v1/reports", tags=["Reports"])


# ============================================================
# Pydantic
# ============================================================

class ReportScope(BaseModel):
    application_id: Optional[str] = None
    cluster_id: Optional[str] = None
    fault_id: Optional[str] = Field(None, description="incident_report 锚点(二选一)")
    change_event_id: Optional[str] = Field(None, description="incident_report 锚点(二选一)")
    time_range_start: Optional[str] = Field(None, description="ISO8601 起")
    time_range_end: Optional[str] = Field(None, description="ISO8601 止")


class GenerateRequest(BaseModel):
    template_id: str = Field(..., description="application_health | cluster_overview | incident_report")
    scope: ReportScope
    format: str = Field("markdown", description="Sprint 1/2 仅 markdown")
    modules: Optional[list[str]] = Field(
        default=None,
        description="启用的模块子集,默认按模板全选",
    )


# ============================================================
# 端点
# ============================================================

@router.post("/generate", status_code=202)
def generate(req: GenerateRequest):
    if req.template_id not in VALID_TEMPLATES:
        raise HTTPException(400, f"unsupported template_id: {req.template_id}; valid: {list(VALID_TEMPLATES)}")
    if req.format not in ("markdown",):
        raise HTTPException(400, f"unsupported format: {req.format}; Sprint 1/2 only supports markdown")

    valid_modules = modules_for_template(req.template_id)
    modules = req.modules if req.modules is not None else list(valid_modules)

    # 校验 modules 子集合法
    bad = [m for m in modules if m not in valid_modules]
    if bad:
        raise HTTPException(
            400,
            f"unknown modules for template '{req.template_id}': {bad}; valid: {list(valid_modules)}",
        )

    # 模板-specific scope 必填校验
    if req.template_id == "application_health" and not req.scope.application_id:
        raise HTTPException(400, "scope.application_id required for application_health template")
    if req.template_id == "incident_report" and not (req.scope.fault_id or req.scope.change_event_id):
        raise HTTPException(400, "scope.fault_id or scope.change_event_id required for incident_report template")

    report_id = new_report_id()
    task = ReportTask(
        report_id=report_id,
        template_id=req.template_id,
        scope=req.scope.model_dump(),
        modules=list(modules),
        format=req.format,
        status="pending",
        created_at=_now_iso(),
    )
    report_store.add_task(task)
    run_generation_background(report_id)

    return {
        "report_id": report_id,
        "status": "pending",
        "estimated_completion_seconds": 5,
    }


@router.get("/{report_id}/status")
def status(report_id: str):
    task = report_store.get_task(report_id)
    if task is None:
        raise HTTPException(404, f"report not found: {report_id}")
    return {
        "report_id": task.report_id,
        "status": task.status,
        "progress": task.progress,
        "current_step": task.current_step,
        "error_message": task.error_message,
    }


@router.get("/{report_id}/download")
def download(report_id: str, format: str = Query("markdown")):
    if format != "markdown":
        raise HTTPException(400, f"unsupported format: {format}; Sprint 1/2 only supports markdown")
    task = report_store.get_task(report_id)
    if task is None:
        raise HTTPException(404, f"report not found: {report_id}")
    if task.status != "completed":
        raise HTTPException(409, f"report not ready: status={task.status}")
    if not task.file_path:
        raise HTTPException(409, "report file missing")

    return FileResponse(
        task.file_path,
        media_type="text/markdown; charset=utf-8",
        filename=f"{report_id}.md",
    )


@router.get("")
def list_reports(
    template_id: Optional[str] = Query(None),
    application_id: Optional[str] = Query(None),
    limit: int = Query(100, ge=1, le=500),
):
    tasks = report_store.list_tasks(template_id=template_id, application_id=application_id)
    sliced = tasks[:limit]
    return {
        "reports": [t.to_dict() for t in sliced],
        "total": len(tasks),
        "returned": len(sliced),
    }


# ============================================================
# 订阅(Sprint 2)
# ============================================================

class SubscriptionRequest(BaseModel):
    template_id: str = Field(..., description="application_health | cluster_overview | incident_report")
    scope: ReportScope
    modules: Optional[list[str]] = Field(default=None)
    cron: str = Field(..., description="标准 5 字段 cron")
    recipients: list[str] = Field(default_factory=list, description="email 列表")
    enabled: bool = True


class SubscriptionPatch(BaseModel):
    cron: Optional[str] = None
    recipients: Optional[list[str]] = None
    enabled: Optional[bool] = None
    modules: Optional[list[str]] = None


def _validate_subscription_payload(template_id: str, scope: "ReportScope", modules: Optional[list[str]]):
    if template_id not in VALID_TEMPLATES:
        raise HTTPException(400, f"unsupported template_id: {template_id}; valid: {list(VALID_TEMPLATES)}")
    valid_modules = modules_for_template(template_id)
    mods = modules if modules is not None else list(valid_modules)
    bad = [m for m in mods if m not in valid_modules]
    if bad:
        raise HTTPException(400, f"unknown modules for template '{template_id}': {bad}")
    if template_id == "application_health" and not scope.application_id:
        raise HTTPException(400, "scope.application_id required for application_health")
    if template_id == "incident_report" and not (scope.fault_id or scope.change_event_id):
        raise HTTPException(400, "scope.fault_id or scope.change_event_id required for incident_report")
    return mods


@router.post("/subscriptions", status_code=201)
def create_subscription(req: SubscriptionRequest):
    mods = _validate_subscription_payload(req.template_id, req.scope, req.modules)
    if not req.recipients:
        raise HTTPException(400, "recipients must contain at least one email")

    sid = new_subscription_id()
    sub = ReportSubscription(
        subscription_id=sid,
        template_id=req.template_id,
        scope=req.scope.model_dump(),
        modules=list(mods),
        cron=req.cron,
        recipients=list(req.recipients),
        enabled=req.enabled,
        created_at=_now_iso(),
    )

    # 注册到 scheduler(cron 错误 → 400)
    try:
        report_scheduler.register_subscription(sub)
    except ValueError as e:
        raise HTTPException(400, str(e)) from e

    subscription_store.add(sub)
    _persist_subscription(sub)

    return sub.to_dict()


@router.get("/subscriptions")
def list_subscriptions(
    template_id: Optional[str] = Query(None),
    application_id: Optional[str] = Query(None),
):
    subs = subscription_store.list(template_id=template_id, application_id=application_id)
    return {
        "subscriptions": [s.to_dict() for s in subs],
        "total": len(subs),
    }


@router.get("/subscriptions/{sub_id}")
def get_subscription(sub_id: str):
    sub = subscription_store.get(sub_id)
    if sub is None:
        raise HTTPException(404, f"subscription not found: {sub_id}")
    return sub.to_dict()


@router.patch("/subscriptions/{sub_id}")
def patch_subscription(sub_id: str, patch: SubscriptionPatch):
    sub = subscription_store.get(sub_id)
    if sub is None:
        raise HTTPException(404, f"subscription not found: {sub_id}")

    update_fields: dict = {}
    if patch.cron is not None:
        update_fields["cron"] = patch.cron
    if patch.recipients is not None:
        if not patch.recipients:
            raise HTTPException(400, "recipients must contain at least one email")
        update_fields["recipients"] = list(patch.recipients)
    if patch.enabled is not None:
        update_fields["enabled"] = patch.enabled
    if patch.modules is not None:
        valid_modules = modules_for_template(sub.template_id)
        bad = [m for m in patch.modules if m not in valid_modules]
        if bad:
            raise HTTPException(400, f"unknown modules: {bad}")
        update_fields["modules"] = list(patch.modules)

    updated = subscription_store.update(sub_id, **update_fields)
    if updated is None:
        raise HTTPException(404, f"subscription not found: {sub_id}")

    # 重新注册 scheduler(cron / enabled 变化都要刷)
    try:
        report_scheduler.register_subscription(updated)
    except ValueError as e:
        raise HTTPException(400, str(e)) from e

    _persist_subscription(updated)
    return updated.to_dict()


@router.delete("/subscriptions/{sub_id}", status_code=204)
def delete_subscription(sub_id: str):
    sub = subscription_store.get(sub_id)
    if sub is None:
        raise HTTPException(404, f"subscription not found: {sub_id}")
    report_scheduler.unregister(sub_id)
    subscription_store.delete(sub_id)
    _delete_subscription_node(sub_id)


@router.post("/subscriptions/{sub_id}/trigger")
def trigger_subscription(sub_id: str):
    sub = subscription_store.get(sub_id)
    if sub is None:
        raise HTTPException(404, f"subscription not found: {sub_id}")
    result = report_scheduler.trigger_now(sub_id)
    return result.to_dict()


@router.get("/sent-emails")
def list_sent_emails():
    sender = get_email_sender()
    if not isinstance(sender, InMemoryEmailSender):
        raise HTTPException(501, "sent-emails endpoint is only available in in-memory mode")
    return {
        "total": len(sender.sent),
        "sent": list(sender.sent),
    }


# ============================================================
# Helpers
# ============================================================

def _now_iso() -> str:
    from datetime import datetime, timezone
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
