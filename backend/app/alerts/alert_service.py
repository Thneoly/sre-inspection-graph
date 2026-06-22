"""AlertEvent 业务编排 — PRD-004 Phase 2。

镜像 PRD-002 的 changes/event_service.py 模式:
- record_alert() : 创建 AlertEvent + 写 DSS + best-effort dual-write Neo4j
- 序列化 / 查询辅助
- connector(prometheus)检测 critical breach 时调 record_alert

设计:
- DSS 是主存储 — Neo4j 写入失败只 logger.warning,不影响 API
- AlertEvent 落地后,PRD-002 Phase 2 的 record_change → correlate_and_persist
  会自动把窗口内变更关联上(CORRELATED_WITH 边),无需本模块主动关联
- 镜像 simulation.py 的 :AlertEvent:ResourceInstance + FIRED_ON 边结构,
  使 legacy view6 告警归并查询同时能看到 DSS 产的告警
- 同一 resource_ref + rule_id 的 firing 告警不重复产出(去重,见 _existing_firing)
"""
from __future__ import annotations

import json
import logging
import uuid
from datetime import datetime, timezone
from typing import Any, Optional

from app.datasource.models import AlertEvent
from app.datasource.store import store
from app.db import neo4j_client as n4j


logger = logging.getLogger(__name__)


def record_alert(
    alert_name: str,
    resource_ref: str,
    severity: str = "critical",
    rule_id: str = "",
    metric_name: str = "",
    metric_value: float = 0.0,
    summary: str = "",
    description: str = "",
    cluster_id: str = "",
    fired_at: Optional[str] = None,
    dedupe: bool = True,
) -> AlertEvent | None:
    """创建并入库一个 AlertEvent。

    - dedupe=True 时,若该 resource_ref + rule_id 已有 firing 告警,返回 None(不重复)
    - severity 必须是 warning | critical
    - best-effort dual-write Neo4j(失败只 warning)
    """
    if severity not in ("warning", "critical"):
        logger.warning("invalid alert severity: %s (skip)", severity)
        return None

    if dedupe and rule_id and _existing_firing(resource_ref, rule_id):
        logger.info(
            "alert %s on %s already firing, skip dedupe",
            alert_name, resource_ref,
        )
        return None

    event = AlertEvent(
        alert_event_id=f"ae-{uuid.uuid4().hex[:12]}",
        alert_name=alert_name,
        severity=severity,
        status="firing",
        fired_at=fired_at or _now_iso(),
        resource_ref=resource_ref,
        rule_id=rule_id,
        metric_name=metric_name,
        metric_value=float(metric_value),
        summary=summary or f"{alert_name} on {resource_ref}: {metric_name}={metric_value}",
        description=description,
        cluster_id=cluster_id,
    )
    store.add_alert_event(event)

    try:
        _persist_alert_event(event)
    except Exception as e:  # noqa: BLE001
        logger.warning(
            "AlertEvent Neo4j persist failed for %s: %s: %s",
            event.alert_event_id, type(e).__name__, e,
        )
    return event


def resolve_alert(alert_id: str, resolved_at: Optional[str] = None) -> AlertEvent | None:
    """把 firing 告警标记为 resolved。幂等。"""
    event = store.get_alert_event(alert_id)
    if event is None:
        return None
    if event.status == "resolved":
        return event
    event.status = "resolved"
    event.resolved_at = resolved_at or _now_iso()
    try:
        _persist_alert_event(event)
    except Exception as e:  # noqa: BLE001
        logger.warning("AlertEvent resolve persist failed for %s: %s", alert_id, e)
    return event


def _existing_firing(resource_ref: str, rule_id: str) -> bool:
    for ev in store.list_alert_events(resource_ref=resource_ref, status="firing"):
        if ev.rule_id == rule_id:
            return True
    return False


def serialize(event: AlertEvent) -> dict[str, Any]:
    return {
        "alert_event_id": event.alert_event_id,
        "alert_name": event.alert_name,
        "severity": event.severity,
        "status": event.status,
        "fired_at": event.fired_at,
        "resolved_at": event.resolved_at,
        "resource_ref": event.resource_ref,
        "rule_id": event.rule_id,
        "metric_name": event.metric_name,
        "metric_value": event.metric_value,
        "summary": event.summary,
        "description": event.description,
        "cluster_id": event.cluster_id,
    }


def _now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


# ============================================================
# Neo4j dual-write — 镜像 simulation.py 的 :AlertEvent 结构
# ============================================================

def _persist_alert_event(event: AlertEvent) -> None:
    """AlertEvent → Neo4j。

    结构对标 simulation.py:_apply_stage(legacy 故障模拟写 AlertEvent 的地方),
    使 view6 告警归并查询同时能看到 connector 产的告警:
    - MERGE :AlertEvent:ResourceInstance 节点(node_id = alert_event_id)
    - MATCH target ResourceInstance + MERGE FIRED_ON 边(target 不存在则跳过)
    """
    driver = n4j.get_driver()
    if driver is None:
        return  # test mode / 未配置 Neo4j

    with driver.session() as s:
        s.run(
            """
            MERGE (ae:AlertEvent:ResourceInstance {node_id: $aid})
            SET ae.alert_event_id = $aid,
                ae.alert_name = $name,
                ae.severity = $sev,
                ae.status = $status,
                ae.fired_at = datetime($fired),
                ae.resolved_at = CASE WHEN $resolved = '' THEN null ELSE datetime($resolved) END,
                ae.resource_ref = $ref,
                ae.rule_id = $rule,
                ae.metric_name = $metric,
                ae.metric_value = $value,
                ae.summary = $summary,
                ae.description = $desc,
                ae.cluster_id = $cluster,
                ae.label = 'AlertEvent',
                ae.health_status = 'red',
                ae.version = 'v1',
                ae.updated_at = datetime()
            """,
            aid=event.alert_event_id,
            name=event.alert_name,
            sev=event.severity,
            status=event.status,
            fired=event.fired_at,
            resolved=event.resolved_at,
            ref=event.resource_ref,
            rule=event.rule_id,
            metric=event.metric_name,
            value=event.metric_value,
            summary=event.summary,
            desc=event.description,
            cluster=event.cluster_id,
        )

        # FIRED_ON 边 → target(不存在则不建,避免 stub 污染图)
        if event.resource_ref:
            s.run(
                """
                MATCH (ae:AlertEvent {alert_event_id: $aid})
                MATCH (t:ResourceInstance {node_id: $rid})
                MERGE (ae)-[r:RELATES_TO {edge_id: 'alert_fired_' + $aid}]->(t)
                SET r.relationship_type = 'FIRED_ON',
                    r.relationship_name = '告警',
                    r.dependency_strength = '强',
                    r.last_verified_at = datetime(),
                    r.version = 'v1'
                """,
                aid=event.alert_event_id,
                rid=event.resource_ref,
            )
