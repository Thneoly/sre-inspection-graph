"""报告任务内存存储 — PRD-003 Sprint 1。

对标 DSS `store` 的单例模式:一个进程级 `report_store` 持有所有 ReportTask。
报告 Markdown 既存在 task.markdown(供 API 直返),也落盘到 backend/reports/{id}.md
(供 FileResponse 下载)。uvicorn 重启后任务丢失 —— 审计产物在磁盘,Sprint 2 再做持久化索引。
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Optional


VALID_STATUSES = ("pending", "generating", "completed", "failed")

# Sprint 1 只支持一个模板
VALID_TEMPLATES = ("application_health",)

# 5 大模块(每个在模板里可选启用)
ALL_MODULES = (
    "health_score",
    "seven_views",
    "risk_list",
    "recommended_actions",
    "historical_trends",
)


@dataclass
class ReportTask:
    """一次报告生成任务。"""

    report_id: str
    template_id: str
    scope: dict[str, Any]                          # {application_id, cluster_id, time_range_start, time_range_end}
    modules: list[str] = field(default_factory=lambda: list(ALL_MODULES))
    format: str = "markdown"
    status: str = "pending"                        # pending | generating | completed | failed
    progress: int = 0                              # 0-100
    current_step: str = ""
    error_message: Optional[str] = None
    markdown: Optional[str] = None
    file_path: Optional[str] = None
    created_at: str = ""
    completed_at: str = ""

    def to_dict(self) -> dict[str, Any]:
        """API 序列化。markdown 不随列表返回(可能很长),只在详情按需取。"""
        return {
            "report_id": self.report_id,
            "template_id": self.template_id,
            "scope": self.scope,
            "modules": self.modules,
            "format": self.format,
            "status": self.status,
            "progress": self.progress,
            "current_step": self.current_step,
            "error_message": self.error_message,
            "has_markdown": self.markdown is not None,
            "file_path": self.file_path,
            "created_at": self.created_at,
            "completed_at": self.completed_at,
        }


class ReportStore:
    """报告任务单例存储。"""

    def __init__(self) -> None:
        self.tasks: dict[str, ReportTask] = {}

    def add_task(self, task: ReportTask) -> ReportTask:
        self.tasks[task.report_id] = task
        return task

    def get_task(self, report_id: str) -> Optional[ReportTask]:
        return self.tasks.get(report_id)

    def update_task(self, report_id: str, **fields: Any) -> Optional[ReportTask]:
        task = self.tasks.get(report_id)
        if task is None:
            return None
        for k, v in fields.items():
            if hasattr(task, k):
                setattr(task, k, v)
        return task

    def list_tasks(
        self,
        template_id: Optional[str] = None,
        application_id: Optional[str] = None,
    ) -> list[ReportTask]:
        tasks = list(self.tasks.values())
        if template_id:
            tasks = [t for t in tasks if t.template_id == template_id]
        if application_id:
            tasks = [t for t in tasks if t.scope.get("application_id") == application_id]
        tasks.sort(key=lambda t: t.created_at, reverse=True)
        return tasks

    def clear(self) -> None:
        self.tasks.clear()


# 进程级单例
report_store = ReportStore()
