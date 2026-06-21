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
from app.reports.store import ALL_MODULES, ReportTask, VALID_TEMPLATES, report_store


router = APIRouter(prefix="/api/v1/reports", tags=["Reports"])


# ============================================================
# Pydantic
# ============================================================

class ReportScope(BaseModel):
    application_id: Optional[str] = None
    cluster_id: Optional[str] = None
    time_range_start: Optional[str] = Field(None, description="ISO8601 起")
    time_range_end: Optional[str] = Field(None, description="ISO8601 止")


class GenerateRequest(BaseModel):
    template_id: str = Field(..., description="application_health(Sprint 1 唯一)")
    scope: ReportScope
    format: str = Field("markdown", description="Sprint 1 仅 markdown")
    modules: list[str] = Field(
        default_factory=lambda: list(ALL_MODULES),
        description="启用的模块子集,默认全部 5 个",
    )


# ============================================================
# 端点
# ============================================================

@router.post("/generate", status_code=202)
def generate(req: GenerateRequest):
    if req.template_id not in VALID_TEMPLATES:
        raise HTTPException(400, f"unsupported template_id: {req.template_id}; Sprint 1 only supports {VALID_TEMPLATES}")
    if req.format not in ("markdown",):
        raise HTTPException(400, f"unsupported format: {req.format}; Sprint 1 only supports markdown")

    # 校验 modules
    bad = [m for m in req.modules if m not in ALL_MODULES]
    if bad:
        raise HTTPException(400, f"unknown modules: {bad}; valid: {list(ALL_MODULES)}")

    if not req.scope.application_id:
        raise HTTPException(400, "scope.application_id required for application_health template")

    report_id = new_report_id()
    task = ReportTask(
        report_id=report_id,
        template_id=req.template_id,
        scope=req.scope.model_dump(),
        modules=list(req.modules),
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
