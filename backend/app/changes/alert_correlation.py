"""ChangeEvent ↔ AlertEvent 关联 — PRD-002 Phase 2。

PRD §6 验收里唯一未勾的项:「变更与 AlertEvent / InspectionFinding 的时间相关性
可查询(CORRELATED_WITH)」。本模块补齐:

- correlate_alerts(change_event_id) : 找时间窗内 resource_ref 落在变更影响面
  (propagated_to ∪ {target})的 AlertEvent
- correlate_changes_for_alert(alert_id) : 反向,给定 AlertEvent 找窗口内对其
  resource 关联的 ChangeEvent
- persist_correlation : best-effort Neo4j 写 CORRELATED_WITH 边

数据来源:
- ChangeEvent 从 DSS(store)读 —— 主存储
- AlertEvent 从 Neo4j 读(:AlertEvent 节点,由 simulation.py 写入;resource_ref
  属性指向被告警资源)。AlertEvent 在 DSS 无模型,故只能走 Neo4j。
- Neo4j 离线 / 无 AlertEvent → 返空,不阻塞

CORRELATED_WITH 边语义:变更可能是告警的诱因(时间窗内 + 资源关联),单向
`(ChangeEvent)-[:CORRELATED_WITH]->(AlertEvent)`,relationship_type 标注。
"""
from __future__ import annotations

import logging
from datetime import datetime, timedelta, timezone
from typing import Any, Optional

from app.changes.event_service import ChangeEventError
from app.changes.event_service import _now_iso
from app.datasource.store import store
from app.db import neo4j_client as n4j


logger = logging.getLogger(__name__)

DEFAULT_WINDOW_SECONDS = 600


def _parse_iso_local(iso: str) -> Optional[datetime]:
    if not iso:
        return None
    try:
        if iso.endswith("Z"):
            dt = datetime.fromisoformat(iso[:-1] + "+00:00")
        else:
            dt = datetime.fromisoformat(iso)
        if dt.tzinfo is None:
            dt = dt.replace(tzinfo=timezone.utc)
        return dt
    except (ValueError, TypeError):
        return None


def _fetch_alerts_in_window(win_start: str, win_end: str) -> list[dict[str, Any]]:
    """读窗口内 AlertEvent。

    PRD-004 Phase 2 起 AlertEvent 有 DSS 模型(alert_service.record_alert 写入),
    优先从 DSS 读;DSS 为空时回退 Neo4j(legacy simulation.py 写的告警)。
    两源合并去重(alert_event_id)。
    """
    # (a) DSS —— PRD-004 Phase 2 起 connector 产的告警在这
    from app.datasource.store import store as _store
    dss_alerts: list[dict[str, Any]] = []
    for ev in _store.list_alert_events(since=win_start, until=win_end):
        dss_alerts.append({
            "alert_event_id": ev.alert_event_id,
            "alert_name": ev.alert_name,
            "severity": ev.severity,
            "fired_at": ev.fired_at,
            "resource_ref": ev.resource_ref,
            "summary": ev.summary,
        })

    # (b) Neo4j —— legacy simulation.py 写的告警 + dual-write 的 DSS 告警
    neo4j_alerts: list[dict[str, Any]] = []
    driver = n4j.get_driver()
    if driver is not None:
        try:
            with driver.session() as s:
                records = s.run(
                    """
                    MATCH (ae:AlertEvent)
                    WHERE ae.fired_at IS NOT NULL
                      AND ae.fired_at >= datetime($win_start)
                      AND ae.fired_at <= datetime($win_end)
                    RETURN ae.alert_event_id AS aid,
                           ae.alert_name AS name,
                           ae.severity AS severity,
                           ae.fired_at AS fired_at,
                           ae.resource_ref AS resource_ref,
                           ae.summary AS summary
                    """,
                    win_start=win_start,
                    win_end=win_end,
                )
                neo4j_alerts = [
                    {
                        "alert_event_id": r.get("aid") or "",
                        "alert_name": r.get("name") or "",
                        "severity": r.get("severity") or "",
                        "fired_at": str(r.get("fired_at") or ""),
                        "resource_ref": r.get("resource_ref") or "",
                        "summary": r.get("summary") or "",
                    }
                    for r in records
                ]
        except Exception as e:  # noqa: BLE001
            logger.warning("fetch alerts in window (neo4j) failed: %s: %s", type(e).__name__, e)

    # 合并去重(alert_event_id)
    seen: set[str] = set()
    merged: list[dict[str, Any]] = []
    for a in dss_alerts + neo4j_alerts:
        if a["alert_event_id"] and a["alert_event_id"] not in seen:
            seen.add(a["alert_event_id"])
            merged.append(a)
    return merged


def correlate_alerts(
    change_event_id: str,
    window_seconds: int = DEFAULT_WINDOW_SECONDS,
) -> dict[str, Any]:
    """给定变更事件,找窗口内资源关联的 AlertEvent。

    关联判定:AlertEvent.resource_ref ∈ {变更 target} ∪ 变更 propagated_to
    时间窗:[changed_at - window, changed_at + window](变更可能是告警诱因或后果)
    """
    event = store.get_change_event(change_event_id)
    if event is None:
        raise ChangeEventError(f"change_event not found: {change_event_id}", code=404)

    affected_ids = {event.target_resource_id, *event.propagated_to}

    changed_dt = _parse_iso_local(event.changed_at)
    if changed_dt is None:
        win_start = _now_iso()
        win_end = _now_iso()
    else:
        start_dt = changed_dt - timedelta(seconds=window_seconds)
        end_dt = changed_dt + timedelta(seconds=window_seconds)
        win_start = start_dt.strftime("%Y-%m-%dT%H:%M:%SZ")
        win_end = end_dt.strftime("%Y-%m-%dT%H:%M:%SZ")

    alerts = _fetch_alerts_in_window(win_start, win_end)
    matched = [a for a in alerts if a["resource_ref"] in affected_ids]

    return {
        "change_event_id": change_event_id,
        "changed_at": event.changed_at,
        "window_start": win_start,
        "window_end": win_end,
        "affected_resource_ids": sorted(affected_ids),
        "alerts": matched,
        "total": len(matched),
        "neo4j_available": n4j.get_driver() is not None,
    }


def correlate_changes_for_alert(
    alert_event_id: str,
    window_seconds: int = 300,
    resource_ref: str = "",
) -> dict[str, Any]:
    """反向:给定 AlertEvent,找窗口内对其 resource 关联的 ChangeEvent。

    resource_ref 可显式传入(不查 Neo4j);否则从 Neo4j 读该 AlertEvent 的 resource_ref。
    关联:ChangeEvent.target_resource_id == resource_ref 或 resource_ref 在 propagated_to。
    """
    ref = resource_ref
    fired_at = ""
    if not ref:
        driver = n4j.get_driver()
        if driver is not None:
            try:
                with driver.session() as s:
                    rec = s.run(
                        "MATCH (ae:AlertEvent {alert_event_id: $aid}) "
                        "RETURN ae.resource_ref AS ref, ae.fired_at AS fired",
                        aid=alert_event_id,
                    ).single()
                    if rec is not None:
                        ref = rec.get("ref") or ""
                        fired_at = str(rec.get("fired") or "")
            except Exception as e:  # noqa: BLE001
                logger.warning("fetch alert %s failed: %s", alert_event_id, e)

    if not ref:
        return {
            "alert_event_id": alert_event_id,
            "resource_ref": "",
            "changes": [],
            "total": 0,
            "neo4j_available": n4j.get_driver() is not None,
        }

    # 时间窗:[fired_at - window, fired_at + window],没 fired_at 则最近 window
    if fired_at:
        fired_dt = _parse_iso_local(fired_at)
    else:
        fired_dt = None

    matched = []
    for ev in store.list_change_events():
        hit = ev.target_resource_id == ref or ref in ev.propagated_to
        if not hit:
            continue
        if fired_dt is not None:
            ev_dt = _parse_iso_local(ev.changed_at)
            if ev_dt is None:
                continue
            delta = abs((ev_dt - fired_dt).total_seconds())
            if delta > window_seconds:
                continue
        matched.append({
            "change_event_id": ev.change_event_id,
            "change_type": ev.change_type,
            "target_resource_id": ev.target_resource_id,
            "changed_at": ev.changed_at,
            "source": ev.source,
            "severity_estimate": ev.severity_estimate,
        })

    matched.sort(key=lambda m: m["changed_at"], reverse=True)
    return {
        "alert_event_id": alert_event_id,
        "resource_ref": ref,
        "fired_at": fired_at,
        "changes": matched,
        "total": len(matched),
        "neo4j_available": n4j.get_driver() is not None,
    }


def persist_correlation(change_event_id: str, alert_event_id: str) -> bool:
    """best-effort Neo4j 写 CORRELATED_WITH 边。

    `(ChangeEvent)-[:CORRELATED_WITH]->(AlertEvent)`。失败返 False,不抛。
    """
    driver = n4j.get_driver()
    if driver is None:
        return False
    try:
        with driver.session() as s:
            s.run(
                """
                MATCH (ce:ChangeEvent {change_event_id: $ceid})
                MATCH (ae:AlertEvent {alert_event_id: $aeid})
                MERGE (ce)-[r:CORRELATED_WITH {edge_id: 'corr_' + $ceid + '_' + $aeid}]->(ae)
                SET r.relationship_type = 'CORRELATED_WITH',
                    r.relationship_name = '变更告警关联',
                    r.dependency_strength = '中',
                    r.last_verified_at = datetime(),
                    r.version = 'v1'
                """,
                ceid=change_event_id,
                aeid=alert_event_id,
            )
        return True
    except Exception as e:  # noqa: BLE001
        logger.warning(
            "persist CORRELATED_WITH %s -> %s failed: %s: %s",
            change_event_id, alert_event_id, type(e).__name__, e,
        )
        return False


def correlate_and_persist(change_event_id: str, window_seconds: int = DEFAULT_WINDOW_SECONDS) -> int:
    """record_change 后调:找窗口内关联 AlertEvent + 写 CORRELATED_WITH 边。

    返回关联上的 AlertEvent 数。Neo4j 离线 → 0,不阻塞。
    """
    try:
        result = correlate_alerts(change_event_id, window_seconds)
    except ChangeEventError:
        return 0
    count = 0
    for alert in result["alerts"]:
        if persist_correlation(change_event_id, alert["alert_event_id"]):
            count += 1
    return count
