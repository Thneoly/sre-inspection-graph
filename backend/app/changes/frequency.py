"""变更频率告警 — PRD-002 Phase 2。

检测"过频变更":同一资源在指定时间窗内变更次数超过阈值。
record_change 写入后调 check_target_frequency,命中则把 severity 至少提到 medium
+ description 追加「[过频变更]」标记,给运维一个"这个资源改太勤了"的信号。

设计:
- 纯内存扫 store.list_change_events,O(n)。事件量级 <10k 可接受,
  Phase 3 再上滑动窗口索引
- 窗口默认 1h,阈值默认 5(可配)
- 命中判定:严格 > threshold(等于阈值不算过频)
"""
from __future__ import annotations

from collections import defaultdict
from datetime import datetime, timedelta, timezone
from typing import Any

from app.datasource.store import store


DEFAULT_WINDOW_SECONDS = 3600
DEFAULT_THRESHOLD = 5


def _now_utc() -> datetime:
    return datetime.now(timezone.utc)


def _parse_iso(iso: str) -> datetime | None:
    """宽松解析 ISO8601(带 Z 或带偏移)。失败返 None。"""
    if not iso:
        return None
    try:
        if iso.endswith("Z"):
            return datetime.fromisoformat(iso[:-1] + "+00:00")
        dt = datetime.fromisoformat(iso)
        if dt.tzinfo is None:
            dt = dt.replace(tzinfo=timezone.utc)
        return dt
    except (ValueError, TypeError):
        return None


def check_target_frequency(
    target_resource_id: str,
    window_seconds: int = DEFAULT_WINDOW_SECONDS,
    threshold: int = DEFAULT_THRESHOLD,
) -> dict[str, Any]:
    """检查单个资源在最近 window 内的变更频次。

    返回 {is_frequent, count, window_seconds, threshold, event_ids}。
    is_frequent = count > threshold。event_ids 按时间倒序。
    """
    now = _now_utc()
    win_start = now - timedelta(seconds=window_seconds)

    events = store.list_change_events(target_resource_id=target_resource_id)
    recent = []
    for ev in events:
        dt = _parse_iso(ev.changed_at)
        if dt is not None and dt >= win_start:
            recent.append(ev)

    recent.sort(key=lambda e: e.changed_at, reverse=True)
    count = len(recent)
    return {
        "is_frequent": count > threshold,
        "count": count,
        "window_seconds": window_seconds,
        "threshold": threshold,
        "event_ids": [e.change_event_id for e in recent],
    }


def detect_frequent_changes(
    window_seconds: int = DEFAULT_WINDOW_SECONDS,
    threshold: int = DEFAULT_THRESHOLD,
) -> list[dict[str, Any]]:
    """扫所有 ChangeEvent,按 target 分桶,返回过频变更列表。

    返回 [{target_resource_id, count, window_start, window_end, threshold, event_ids}],
    按 count 倒序。空列表表示无过频。
    """
    now = _now_utc()
    win_start = now - timedelta(seconds=window_seconds)
    win_start_iso = win_start.strftime("%Y-%m-%dT%H:%M:%SZ")
    win_end_iso = now.strftime("%Y-%m-%dT%H:%M:%SZ")

    buckets: dict[str, list] = defaultdict(list)
    for ev in store.list_change_events():
        dt = _parse_iso(ev.changed_at)
        if dt is not None and dt >= win_start:
            buckets[ev.target_resource_id].append(ev)

    frequent: list[dict[str, Any]] = []
    for target, evs in buckets.items():
        if len(evs) > threshold:
            evs.sort(key=lambda e: e.changed_at, reverse=True)
            frequent.append({
                "target_resource_id": target,
                "count": len(evs),
                "window_start": win_start_iso,
                "window_end": win_end_iso,
                "threshold": threshold,
                "event_ids": [e.change_event_id for e in evs],
            })

    frequent.sort(key=lambda f: f["count"], reverse=True)
    return frequent
