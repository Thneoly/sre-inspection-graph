"""ChangeEvent 业务编排 — PRD-002 Sprint 1。

职责:
- record_change() : 创建事件 + 一次性算 propagated_to + 推导 severity_estimate + 写 DSS
- correlated_changes() : 故障关联查询(window-based 时间窗 + direct/propagated 双匹配)
- application_timeline() : 沿 BELONGS_TO 拉应用所有资源,聚合事件返回时间线
- get_impact() : 给定事件 ID,返回该事件影响的资源 + 路径

设计:
- ChangeEvent 仅写入 DSS 内存(Sprint 2 再考虑 Neo4j 双写)
- propagated_to 在 record 时算一次,/correlated 查询走 O(n) 扫描即可
- 时间窗口比较直接拿 ISO8601 字符串字典序(同时区下与时间戳序一致),不解析 datetime
"""

import uuid
from datetime import datetime, timedelta, timezone
from typing import Any, Optional

from app.changes.propagation import (
    derive_propagation,
    find_descendants,
    find_propagation_path,
    PROPAGATION_EDGES,
)
from app.datasource.models import ChangeEvent
from app.datasource.store import store


VALID_CHANGE_TYPES = {
    "configmap_updated",
    "secret_rotated",
    "deployment_rolled",
    "image_pushed",
}

VALID_SOURCES = {"k8s_api", "argo_cd", "gitops", "manual", "unknown"}


class ChangeEventError(Exception):
    """业务错误:类型不合法 / 找不到事件 / 参数缺失 etc."""

    def __init__(self, message: str, code: int = 400):
        super().__init__(message)
        self.code = code


# ============================================================
# 写入
# ============================================================

def record_change(
    change_type: str,
    target_resource_id: str,
    changed_by: str = "",
    source: str = "manual",
    description: str = "",
    diff_summary: Optional[dict[str, Any]] = None,
    related_commit: str = "",
    related_pr: str = "",
    changed_at: Optional[str] = None,
) -> ChangeEvent:
    """创建并入库一个 ChangeEvent。

    - target 不在 DSS 仍记录(propagated_to=[]),不抛错 —— Phase 2 真实 watcher 可能
      在节点同步前就推送变更
    - severity_estimate 由 propagated_to 大小三档分级
    """
    if change_type not in VALID_CHANGE_TYPES:
        raise ChangeEventError(
            f"unknown change_type: {change_type}; "
            f"expected one of {sorted(VALID_CHANGE_TYPES)}"
        )
    if source not in VALID_SOURCES:
        raise ChangeEventError(
            f"unknown source: {source}; expected one of {sorted(VALID_SOURCES)}"
        )

    target_node = store.get_node(target_resource_id)
    target_type_str = target_node.type if target_node else ""

    propagated = derive_propagation(target_resource_id) if target_node else []
    severity = _estimate_severity(len(propagated))

    event = ChangeEvent(
        change_event_id=f"ce-{uuid.uuid4().hex[:12]}",
        change_type=change_type,
        target_resource_id=target_resource_id,
        target_resource_type=target_type_str,
        changed_at=changed_at or _now_iso(),
        changed_by=changed_by,
        source=source,
        description=description,
        diff_summary=diff_summary or {},
        related_commit=related_commit,
        related_pr=related_pr,
        severity_estimate=severity,
        propagated_to=propagated,
    )
    store.add_change_event(event)
    return event


# ============================================================
# 查询 — 关联
# ============================================================

def correlated_changes(
    target_resource_id: str,
    window_seconds: int = 300,
    since: Optional[str] = None,
    until: Optional[str] = None,
    include_propagated: bool = True,
) -> dict[str, Any]:
    """查询 target 在指定时间窗口内的相关变更。

    时间窗口语义:
    - 给 since + until → [since, until] 闭区间
    - 给 since 不给 until → [since, since + window_seconds]
    - 都不给 → [now - window_seconds, now]
    """
    now = _now_iso()
    if since and until:
        win_start, win_end = since, until
    elif since:
        win_start = since
        win_end = _shift_iso(since, window_seconds)
    else:
        win_end = until or now
        win_start = _shift_iso(win_end, -window_seconds)

    matches: list[dict[str, Any]] = []
    for event in store.list_change_events(since=win_start, until=win_end):
        match_type: Optional[str] = None
        distance = 0
        if event.target_resource_id == target_resource_id:
            match_type = "direct"
        elif include_propagated and target_resource_id in event.propagated_to:
            match_type = "propagated"
            path = find_propagation_path(event.target_resource_id, target_resource_id)
            distance = max(len(path) - 1, 1)
        if match_type is None:
            continue
        matches.append({
            **_serialize_event(event),
            "match_type": match_type,
            "propagation_distance": distance,
        })

    matches.sort(key=lambda m: m["changed_at"], reverse=True)
    return {
        "target_resource_id": target_resource_id,
        "window_start": win_start,
        "window_end": win_end,
        "now": now,
        "include_propagated": include_propagated,
        "changes": matches,
        "total": len(matches),
    }


# ============================================================
# 查询 — 单事件影响
# ============================================================

def get_impact(event_id: str) -> dict[str, Any]:
    """给定事件 ID,返回完整影响树(含每个被影响资源的反向路径)。"""
    event = store.get_change_event(event_id)
    if event is None:
        raise ChangeEventError(f"change_event not found: {event_id}", code=404)

    affected: list[dict[str, Any]] = []
    for affected_id in event.propagated_to:
        node = store.get_node(affected_id)
        path = find_propagation_path(event.target_resource_id, affected_id)
        affected.append({
            "resource_id": affected_id,
            "resource_type": node.type if node else "",
            "resource_name": node.name if node else "",
            "path": path,
            "distance": max(len(path) - 1, 0),
        })

    affected.sort(key=lambda a: (a["distance"], a["resource_id"]))

    return {
        "change_event_id": event.change_event_id,
        "target_resource_id": event.target_resource_id,
        "target_resource_type": event.target_resource_type,
        "affected": affected,
        "affected_count": len(affected),
        "severity_estimate": event.severity_estimate,
    }


# ============================================================
# 查询 — 应用级时间线
# ============================================================

def application_timeline(
    application_id: str,
    since: Optional[str] = None,
    until: Optional[str] = None,
) -> dict[str, Any]:
    """前向 BFS 找应用下所有资源,聚合该集合的变更事件。

    项目约定 `app -CONTAINS-> comp -DEPLOYED_AS-> deploy -CONTAINS-> pod -USES-> cm`,
    都是 source→target 方向。所以从 application 起 forward 走能找到完整子树:
    component / deployment / pod / configmap / secret 等。
    """
    if store.get_node(application_id) is None:
        raise ChangeEventError(
            f"application not found in DSS: {application_id}", code=404
        )

    # 应用本身 + 所有正向可达资源
    related_ids = {application_id} | set(find_descendants(application_id, max_depth=6))

    events = store.list_change_events(since=since, until=until)
    timeline = [
        _serialize_event(event)
        for event in events
        if event.target_resource_id in related_ids
    ]
    timeline.sort(key=lambda e: e["changed_at"], reverse=True)

    # 按 type 聚合
    by_type: dict[str, int] = {}
    for entry in timeline:
        by_type[entry["change_type"]] = by_type.get(entry["change_type"], 0) + 1

    return {
        "application_id": application_id,
        "since": since,
        "until": until,
        "resources_in_scope": len(related_ids),
        "events": timeline,
        "total": len(timeline),
        "by_type": by_type,
    }


# ============================================================
# 序列化
# ============================================================

def serialize(event: ChangeEvent) -> dict[str, Any]:
    return _serialize_event(event)


def _serialize_event(event: ChangeEvent) -> dict[str, Any]:
    return {
        "change_event_id": event.change_event_id,
        "change_type": event.change_type,
        "target_resource_id": event.target_resource_id,
        "target_resource_type": event.target_resource_type,
        "changed_at": event.changed_at,
        "changed_by": event.changed_by,
        "source": event.source,
        "description": event.description,
        "diff_summary": event.diff_summary,
        "related_commit": event.related_commit,
        "related_pr": event.related_pr,
        "severity_estimate": event.severity_estimate,
        "propagated_to": event.propagated_to,
        "propagated_count": len(event.propagated_to),
    }


# ============================================================
# Helpers
# ============================================================

def _estimate_severity(propagated_count: int) -> str:
    if propagated_count >= 10:
        return "high"
    if propagated_count >= 5:
        return "medium"
    return "low"


def _now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _shift_iso(iso: str, delta_seconds: int) -> str:
    """把 ISO8601 字符串往前/后平移 N 秒,保持 'Z' 后缀。"""
    # 兼容尾部 Z 和带偏移
    if iso.endswith("Z"):
        dt = datetime.fromisoformat(iso[:-1] + "+00:00")
    else:
        dt = datetime.fromisoformat(iso)
    dt = dt + timedelta(seconds=delta_seconds)
    return dt.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
