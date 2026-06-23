"""报告生成器 — PRD-003 Sprint 1。

`generate_report(report_id)` 是同步函数:读 task → 按 modules 顺序采集 → Jinja2 渲染
→ 落盘 backend/reports/{id}.md → 标记 completed。异常 → failed + error_message。
`run_generation_background` 包一层 threading.Thread 给 API 异步触发。
测试直接调 generate_report 同步,不起线程(避免 flaky)。
"""

from __future__ import annotations

import logging
import threading
import uuid
from datetime import datetime, timezone
from pathlib import Path

from jinja2 import Environment, FileSystemLoader, select_autoescape

from app.reports.modules import gatherers_for_template
from app.reports.store import report_store


logger = logging.getLogger(__name__)

_TEMPLATES_DIR = Path(__file__).parent / "templates"
_OUTPUT_DIR = Path(__file__).resolve().parent.parent.parent / "reports"  # backend/reports/

_env = Environment(
    loader=FileSystemLoader(str(_TEMPLATES_DIR)),
    autoescape=select_autoescape(disabled_extensions=("md",), default=False),
    trim_blocks=True,
    lstrip_blocks=True,
)


def _now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _ensure_output_dir() -> None:
    _OUTPUT_DIR.mkdir(parents=True, exist_ok=True)


def generate_report(report_id: str) -> None:
    """同步生成报告。读 task → 采集 → 渲染 → 落盘。失败标记 failed。"""
    task = report_store.get_task(report_id)
    if task is None:
        logger.warning("generate_report: task not found %s", report_id)
        return

    report_store.update_task(
        report_id,
        status="generating",
        progress=0,
        current_step="采集数据",
        error_message=None,
    )

    try:
        app_id = task.scope.get("application_id", "")
        time_range = {
            "time_range_start": task.scope.get("time_range_start"),
            "time_range_end": task.scope.get("time_range_end"),
        }

        # 按 template_id 取对应模块表(cluster / incident / app)
        template_gatherers = gatherers_for_template(task.template_id)
        if not template_gatherers:
            raise ValueError(f"unsupported template_id: {task.template_id}")

        # 按 task.modules 顺序采集;未启用的模块不采(模板里也用 if 跳过)
        context: dict = {
            "report_id": report_id,
            "scope": task.scope,
            "generated_at": _now_iso(),
            "modules": {m: (m in task.modules) for m in template_gatherers},
        }

        total = len(task.modules)
        for i, module_name in enumerate(task.modules):
            gatherer = template_gatherers.get(module_name)
            if gatherer is None:
                continue
            report_store.update_task(
                report_id,
                current_step=f"采集 {module_name}",
                progress=int(i / max(total, 1) * 80),
            )
            # 应用级模块用 app_id 第一参数;集群/事件模块用 scope/cluster_id —— 统一传 kwargs
            if task.template_id == "application_health":
                context[module_name] = gatherer(app_id, time_range=time_range)
            elif task.template_id == "cluster_overview":
                context[module_name] = gatherer(
                    cluster_id=task.scope.get("cluster_id"),
                    time_range=time_range,
                )
            else:  # incident_report
                context[module_name] = gatherer(task.scope)

        report_store.update_task(report_id, current_step="渲染 Markdown", progress=90)

        template = _env.get_template(f"{task.template_id}.md")
        markdown = template.render(**context)

        _ensure_output_dir()
        file_path = _OUTPUT_DIR / f"{report_id}.md"
        file_path.write_text(markdown, encoding="utf-8")

        report_store.update_task(
            report_id,
            status="completed",
            progress=100,
            current_step="完成",
            markdown=markdown,
            file_path=str(file_path),
            completed_at=_now_iso(),
        )
    except Exception as e:  # noqa: BLE001
        logger.exception("report generation failed: %s", report_id)
        report_store.update_task(
            report_id,
            status="failed",
            current_step="失败",
            error_message=f"{type(e).__name__}: {e}",
            completed_at=_now_iso(),
        )


def run_generation_background(report_id: str) -> threading.Thread:
    """起后台线程生成报告。返回 thread 句柄(测试可 join)。"""
    thread = threading.Thread(
        target=generate_report, args=(report_id,), daemon=True, name=f"report-{report_id[:8]}"
    )
    thread.start()
    return thread


def new_report_id() -> str:
    return f"rpt-{uuid.uuid4().hex[:12]}"


def output_dir() -> Path:
    """暴露产物目录(测试 / router 用)。"""
    return _OUTPUT_DIR


def is_weasyprint_available() -> bool:
    """Sprint 1 恒 False(PDF 延后)。占位供未来 PDF 切换探测。"""
    return False
