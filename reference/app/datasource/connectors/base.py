"""BaseConnector — 周期性数据源拉取的抽象。

设计要点:
- `sync_once()` 是子类必须实现的核心方法,返回 SyncResult。
- `start()` 起一个后台 asyncio task,按 sync_interval_seconds 间隔调 sync_once。
- 异常会被吞下并记录到 last_error_message,不让 connector 因一次失败永久挂掉。
- 上次同步摘要 / 错误计数都暴露给 /api/v1/connectors/status 端点。

不在本类:
- 具体的 K8s / Prometheus 客户端 — 子类自己持有。
- 写入 DSS 的逻辑 — 子类的 sync_once 自己做(diff + upsert)。
"""

from __future__ import annotations

import asyncio
import logging
from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Optional


logger = logging.getLogger(__name__)


@dataclass
class SyncResult:
    """一次 sync_once 的产出 — 给 /status 端点用,以及单元测试断言。"""
    nodes_added: int = 0
    nodes_updated: int = 0
    nodes_removed: int = 0
    edges_added: int = 0
    edges_updated: int = 0
    edges_removed: int = 0
    metrics_added: int = 0
    events_added: int = 0
    duration_ms: int = 0
    notes: list[str] = field(default_factory=list)

    def is_noop(self) -> bool:
        return (
            self.nodes_added == 0 and self.nodes_updated == 0 and self.nodes_removed == 0
            and self.edges_added == 0 and self.edges_updated == 0 and self.edges_removed == 0
            and self.metrics_added == 0 and self.events_added == 0
        )


class BaseConnector(ABC):
    """所有 connector 的基类。子类只需实现 sync_once + 设置 name。"""

    name: str = "base"
    sync_interval_seconds: int = 30

    def __init__(self):
        self._task: Optional[asyncio.Task] = None
        self._stop_event = asyncio.Event()
        self._last_sync_at: Optional[str] = None
        self._last_result: Optional[SyncResult] = None
        self._last_error_message: str = ""
        self._error_count_24h: int = 0
        self._sync_count: int = 0

    @abstractmethod
    async def sync_once(self) -> SyncResult:
        """子类实现:抓数据 → diff → 写 DSS,返回摘要。

        允许抛异常 — 由 _run_loop 捕获并记录。
        """

    async def start(self):
        """非阻塞:启动后台循环。"""
        if self._task is not None and not self._task.done():
            logger.warning("connector %s already running", self.name)
            return
        self._stop_event.clear()
        self._task = asyncio.create_task(self._run_loop(), name=f"connector-{self.name}")
        logger.info("connector %s started, interval=%ds", self.name, self.sync_interval_seconds)

    async def stop(self):
        self._stop_event.set()
        if self._task is not None:
            try:
                await asyncio.wait_for(self._task, timeout=5)
            except asyncio.TimeoutError:
                self._task.cancel()
            self._task = None
        logger.info("connector %s stopped", self.name)

    async def trigger_sync_now(self) -> SyncResult:
        """手动触发一次同步 — /api/v1/connectors/{name}/sync-now 用。"""
        return await self._run_once()

    # ============================================================
    # 状态暴露
    # ============================================================
    def status(self) -> dict:
        return {
            "name": self.name,
            "running": self._task is not None and not self._task.done(),
            "sync_interval_seconds": self.sync_interval_seconds,
            "last_sync_at": self._last_sync_at,
            "last_result": _result_to_dict(self._last_result),
            "last_error_message": self._last_error_message,
            "error_count_24h": self._error_count_24h,
            "sync_count": self._sync_count,
        }

    # ============================================================
    # 内部
    # ============================================================
    async def _run_loop(self):
        # 立即跑一次,不等第一个 interval
        await self._run_once()
        while not self._stop_event.is_set():
            try:
                await asyncio.wait_for(self._stop_event.wait(), timeout=self.sync_interval_seconds)
                # wait 没超时 → stop_event 被设置 → 退出
                return
            except asyncio.TimeoutError:
                # 正常路径:间隔到了
                pass
            await self._run_once()

    async def _run_once(self) -> SyncResult:
        start = datetime.now(timezone.utc)
        try:
            result = await self.sync_once()
            self._last_error_message = ""
        except Exception as e:  # noqa: BLE001 — 故意吞所有异常,connector 不能死
            logger.exception("connector %s sync failed", self.name)
            self._last_error_message = f"{type(e).__name__}: {e}"
            self._error_count_24h += 1
            result = SyncResult(notes=[f"error: {self._last_error_message}"])
        end = datetime.now(timezone.utc)
        result.duration_ms = int((end - start).total_seconds() * 1000)
        self._last_result = result
        self._last_sync_at = end.strftime("%Y-%m-%dT%H:%M:%SZ")
        self._sync_count += 1
        return result


def _result_to_dict(r: Optional[SyncResult]) -> Optional[dict]:
    if r is None:
        return None
    return {
        "nodes_added": r.nodes_added,
        "nodes_updated": r.nodes_updated,
        "nodes_removed": r.nodes_removed,
        "edges_added": r.edges_added,
        "edges_updated": r.edges_updated,
        "edges_removed": r.edges_removed,
        "metrics_added": r.metrics_added,
        "events_added": r.events_added,
        "duration_ms": r.duration_ms,
        "notes": r.notes,
    }
