"""APScheduler 报告调度器 — PRD-003 Sprint 2。

包装 `BackgroundScheduler`,把 ReportSubscription 注册成 cron job。
进程级单例 `report_scheduler`,FastAPI lifespan 控制 start/stop。

job 触发时:读 sub → 创建一次性 ReportTask → 同步 generate_report → 邮件发送 → 更新 last_*。
`trigger_now(sub_id)` 暴露给 API + 测试,免起线程,直接同步跑。
"""

from __future__ import annotations

import logging
import threading
from datetime import datetime, timezone
from typing import Optional

from apscheduler.schedulers.background import BackgroundScheduler
from apscheduler.triggers.cron import CronTrigger

from app.reports.email_sender import get_email_sender
from app.reports.generator import generate_report, new_report_id
from app.reports.store import ReportTask, report_store
from app.reports.subscription_store import ReportSubscription, subscription_store


logger = logging.getLogger(__name__)


def _now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


class ReportScheduler:
    """ReportSubscription cron 调度器。"""

    def __init__(self) -> None:
        self.scheduler = BackgroundScheduler()
        self._lock = threading.RLock()
        self._started = False

    def start(self) -> None:
        with self._lock:
            if self._started:
                return
            if not self.scheduler.running:
                self.scheduler.start()
            self._started = True

    def stop(self) -> None:
        with self._lock:
            if not self._started:
                return
            try:
                self.scheduler.shutdown(wait=False)
            except Exception:
                logger.exception("scheduler shutdown error")
            self._started = False

    def register_subscription(self, sub: ReportSubscription) -> None:
        """加 / 替换一个 cron job。cron 串错误 → 抛 ValueError。"""
        try:
            trigger = CronTrigger.from_crontab(sub.cron)
        except Exception as e:
            raise ValueError(f"invalid cron expression: {sub.cron}; {e}") from e

        if not sub.enabled:
            self.unregister(sub.subscription_id)
            return

        self.scheduler.add_job(
            _run_subscription_safely,
            trigger=trigger,
            args=[sub.subscription_id],
            id=sub.subscription_id,
            replace_existing=True,
            misfire_grace_time=300,
            coalesce=True,
        )

    def unregister(self, sub_id: str) -> None:
        try:
            self.scheduler.remove_job(sub_id)
        except Exception:
            pass

    def reload_all(self) -> None:
        """启动后把 subscription_store 全量注册到 scheduler。"""
        for sub in subscription_store.list():
            try:
                self.register_subscription(sub)
            except ValueError:
                logger.warning("skip invalid cron sub: %s", sub.subscription_id)

    def trigger_now(self, sub_id: str) -> ReportSubscription:
        """同步直跑一次(API + 测试用)。"""
        return _run_subscription_safely(sub_id)


def _run_subscription_safely(sub_id: str) -> ReportSubscription:
    """读 sub → 创建 ReportTask → 同步生成 → 发邮件 → 更新 last_*。"""
    sub = subscription_store.get(sub_id)
    if sub is None:
        logger.warning("subscription not found: %s", sub_id)
        return ReportSubscription(  # 占位返回(供测试断言)
            subscription_id=sub_id, template_id="", scope={}, modules=[],
            cron="", recipients=[], last_status="failed",
            last_error="subscription not found",
        )

    if not sub.enabled:
        logger.info("subscription disabled, skip: %s", sub_id)
        return sub

    try:
        rid = new_report_id()
        report_store.add_task(ReportTask(
            report_id=rid, template_id=sub.template_id,
            scope=dict(sub.scope), modules=list(sub.modules),
            format="markdown", status="pending", created_at=_now_iso(),
        ))
        generate_report(rid)
        task = report_store.get_task(rid)
        if task is None or task.status != "completed":
            raise RuntimeError(f"report {rid} generation failed: {task.error_message if task else 'lost'}")

        sender = get_email_sender()
        subject = f"[SRE 巡检报告] {sub.template_id} — {sub.scope.get('application_id') or sub.scope.get('cluster_id') or '总览'}"
        sender.send(
            recipients=list(sub.recipients),
            subject=subject,
            body=task.markdown or "",
            attachments=[{
                "filename": f"{rid}.md",
                "content": task.markdown or "",
                "mimetype": "text/markdown",
            }],
        )

        subscription_store.update(
            sub_id,
            last_run_at=_now_iso(),
            last_status="ok",
            last_error="",
            last_report_id=rid,
        )
    except Exception as e:  # noqa: BLE001
        logger.exception("subscription run failed: %s", sub_id)
        subscription_store.update(
            sub_id,
            last_run_at=_now_iso(),
            last_status="failed",
            last_error=f"{type(e).__name__}: {e}",
        )

    return subscription_store.get(sub_id) or sub


# 进程级单例
report_scheduler = ReportScheduler()


def reset_scheduler() -> None:
    """测试用 — 重建调度器实例。"""
    global report_scheduler
    try:
        report_scheduler.stop()
    except Exception:
        pass
    report_scheduler = ReportScheduler()


def get_scheduler() -> Optional[ReportScheduler]:
    return report_scheduler
