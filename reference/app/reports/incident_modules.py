"""事件级报告模块 — PRD-003 Sprint 2 `incident_report` 模板。

围绕一个"锚点(anchor)"展开:
- scope.fault_id → DSS FaultInjection
- scope.change_event_id → DSS ChangeEvent

两者二选一,若都缺/无解析 → ValueError(让 generator 落到 failed 分支)。

事件 = 锚点 + 反向 BFS 受影响节点 + 时间窗内交叉的变更与恢复。
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from app.changes.propagation import derive_propagation
from app.datasource.store import store
from app.recovery.action_defs import suggest_for_change


@dataclass
class _Anchor:
    """统一锚点表示,屏蔽 fault / change 两种来源。"""
    kind: str  # "fault" | "change"
    anchor_id: str
    target_id: str
    target_type: str
    timestamp: str
    description: str
    severity: str = ""
    raw: Any = None


def _resolve_anchor(scope: dict[str, Any]) -> _Anchor:
    """从 scope 解析锚点。优先 fault_id,其次 change_event_id。

    任一存在但解析失败,或两者都没有 → ValueError。
    """
    fault_id = scope.get("fault_id")
    if fault_id:
        fault = store.get_fault(fault_id)
        if fault is None:
            raise ValueError(f"fault_id not found: {fault_id}")
        target = store.get_node(fault.target_id)
        return _Anchor(
            kind="fault",
            anchor_id=fault.injection_id,
            target_id=fault.target_id,
            target_type=target.type if target else "Unknown",
            timestamp=fault.injected_at,
            description=f"故障 {fault.fault_type} 注入到 {fault.target_id}",
            severity=fault.status,
            raw=fault,
        )

    change_event_id = scope.get("change_event_id")
    if change_event_id:
        event = store.get_change_event(change_event_id)
        if event is None:
            raise ValueError(f"change_event_id not found: {change_event_id}")
        return _Anchor(
            kind="change",
            anchor_id=event.change_event_id,
            target_id=event.target_resource_id,
            target_type=event.target_resource_type,
            timestamp=event.changed_at,
            description=event.description or event.change_type,
            severity=event.severity_estimate or "",
            raw=event,
        )

    raise ValueError("incident scope requires fault_id or change_event_id")


def _within_window(ts: str, anchor_ts: str, window_seconds: int) -> bool:
    """字符串 ISO8601 时间窗比较(同时区 / Z 后缀下字典序与时序一致)。"""
    if not ts or not anchor_ts:
        return False
    # 字符串字典序对 Z 时间足够,但锚点窗口需要"秒精度",仍用 datetime 解析
    from datetime import datetime

    def _parse(s: str):
        try:
            return datetime.fromisoformat(s.replace("Z", "+00:00"))
        except ValueError:
            return None

    a = _parse(anchor_ts)
    t = _parse(ts)
    if a is None or t is None:
        return False
    return abs((t - a).total_seconds()) <= window_seconds


def gather_incident_summary(scope: dict[str, Any], **_: Any) -> dict[str, Any]:
    """模块 1:事件摘要 + 受影响节点(反向 BFS)。"""
    anchor = _resolve_anchor(scope)
    propagated = derive_propagation(anchor.target_id, max_depth=4)

    # 按类型聚合受影响节点
    from collections import Counter
    affected_types: Counter[str] = Counter()
    affected: list[dict[str, Any]] = []
    for nid in propagated:
        node = store.get_node(nid)
        if node is None:
            continue
        affected_types[node.type] += 1
        affected.append({
            "resource_id": node.id,
            "resource_type": node.type,
            "name": node.name,
        })

    return {
        "kind": anchor.kind,
        "anchor_id": anchor.anchor_id,
        "target_id": anchor.target_id,
        "target_type": anchor.target_type,
        "timestamp": anchor.timestamp,
        "description": anchor.description,
        "severity": anchor.severity,
        "affected_total": len(affected),
        "affected_by_type": dict(affected_types),
        "affected_nodes": affected,
    }


def gather_incident_timeline(
    scope: dict[str, Any],
    window_seconds: int = 3600,
    **_: Any,
) -> dict[str, Any]:
    """模块 2:锚点 ±window 内 ChangeEvent + RecoveryExecution,按时间排序。"""
    anchor = _resolve_anchor(scope)
    propagated = set(derive_propagation(anchor.target_id, max_depth=4)) | {anchor.target_id}

    items: list[dict[str, Any]] = []

    # 范围内 ChangeEvent(与锚点 target / 受影响节点相关)
    for c in store.list_change_events():
        if c.target_resource_id not in propagated:
            continue
        if not _within_window(c.changed_at, anchor.timestamp, window_seconds):
            continue
        items.append({
            "kind": "change",
            "timestamp": c.changed_at,
            "type": c.change_type,
            "target_id": c.target_resource_id,
            "actor": c.changed_by,
            "description": c.description or "",
            "severity": c.severity_estimate or "",
        })

    # 范围内 RecoveryExecution
    for e in store.get_all_executions():
        if e.target_resource_id not in propagated:
            continue
        if not _within_window(e.initiated_at or "", anchor.timestamp, window_seconds):
            continue
        items.append({
            "kind": "recovery",
            "timestamp": e.initiated_at or "",
            "type": e.action_id,
            "target_id": e.target_resource_id,
            "actor": e.initiated_by or "",
            "description": e.request_reason or "",
            "severity": e.status,
        })

    items.sort(key=lambda x: x["timestamp"])
    return {
        "anchor_id": anchor.anchor_id,
        "anchor_timestamp": anchor.timestamp,
        "window_seconds": window_seconds,
        "total": len(items),
        "events": items,  # 不用 "items" 这个 key — 与 Jinja2 dict.items() 方法冲突
    }


def gather_incident_recoveries(scope: dict[str, Any], **_: Any) -> dict[str, Any]:
    """模块 3:本事件相关的 RecoveryExecution + 推荐后续动作。"""
    anchor = _resolve_anchor(scope)
    propagated = set(derive_propagation(anchor.target_id, max_depth=4)) | {anchor.target_id}

    executed: list[dict[str, Any]] = []
    for e in store.get_all_executions():
        if e.target_resource_id not in propagated:
            continue
        executed.append({
            "execution_id": e.execution_id,
            "action_id": e.action_id,
            "target_id": e.target_resource_id,
            "status": e.status,
            "initiated_by": e.initiated_by or "",
            "initiated_at": e.initiated_at or "",
            "completed_at": e.completed_at or "",
        })

    # 推荐后续:若是 change 锚点,复用 PRD-002 Phase 2 的 suggest_for_change
    recommended: list[dict[str, Any]] = []
    if anchor.kind == "change":
        change_type = anchor.raw.change_type if anchor.raw else ""
        for sugg in suggest_for_change(change_type):
            recommended.append({
                "action_id": sugg["action_id"],
                "target_id": anchor.target_id,
                "rationale": sugg.get("rationale", ""),
            })

    return {
        "anchor_id": anchor.anchor_id,
        "executed_total": len(executed),
        "executed": executed,
        "recommended_total": len(recommended),
        "recommended": recommended,
    }


# 模板模块名 → 采集函数
INCIDENT_MODULE_GATHERERS: dict[str, Any] = {
    "incident_summary": gather_incident_summary,
    "incident_timeline": gather_incident_timeline,
    "incident_recoveries": gather_incident_recoveries,
}
