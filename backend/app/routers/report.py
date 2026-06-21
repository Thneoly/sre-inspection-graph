"""自检报告 API — PRD-003 Sprint 1。

端点(prefix /api/v1/reports):
- POST /generate           触发生成(异步,立即返回 pending)
- GET  /{id}/status        查询生成状态
- GET  /{id}/download      下载 Markdown(.md 文件)
- GET  /                   列表(过滤 template / application)

Sprint 1 只支持 application_health 模板 + Markdown 输出。PDF / 订阅 / 其它模板留 Sprint 2。
"""

from __future__ import annotations

from typing import Optional

from fastapi import APIRouter, HTTPException, Query
from fastapi.responses import FileResponse
from pydantic import BaseModel, Field

from app.reports.generator import new_report_id, run_generation_background
from app.reports.store import (
    ReportTask,
    VALID_TEMPLATES,
    modules_for_template,
    report_store,
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
# Helpers
# ============================================================

def _now_iso() -> str:
    from datetime import datetime, timezone
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
