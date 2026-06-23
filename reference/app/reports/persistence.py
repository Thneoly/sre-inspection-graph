"""ReportSubscription Neo4j dual-write — PRD-003 Sprint 2。

镜像 `app.changes.event_service._persist_change_event` 的 best-effort 模式:
- MERGE :ReportSubscription:ResourceInstance 节点
- 失败 → logger.warning,不抛(API / 内存 store 不阻塞)
- 启动时 `load_subscriptions_from_neo4j` 反向 hydrate

Subscription 不带边(没有目标资源关联),仅作为一个独立配置节点。
scope / modules / recipients 是 dict / list,Neo4j 用 JSON str + list 原生属性混合存。
"""

from __future__ import annotations

import json
import logging
from typing import Optional

from app.db import neo4j_client as n4j
from app.reports.subscription_store import ReportSubscription, subscription_store


logger = logging.getLogger(__name__)


def _persist_subscription(sub: ReportSubscription) -> None:
    """订阅 → Neo4j。"""
    try:
        driver = n4j.get_driver()
    except Exception:
        logger.warning("neo4j driver unavailable, skip subscription persist")
        return
    if driver is None:
        return

    try:
        with driver.session() as s:
            s.run(
                """
                MERGE (s:ReportSubscription:ResourceInstance {node_id: $sid})
                SET s.subscription_id = $sid,
                    s.template_id = $tid,
                    s.scope_json = $scope,
                    s.modules = $modules,
                    s.cron = $cron,
                    s.recipients = $recipients,
                    s.enabled = $enabled,
                    s.created_at = $created,
                    s.last_run_at = $last_run,
                    s.last_status = $last_status,
                    s.last_error = $last_error,
                    s.last_report_id = $last_report_id,
                    s.label = 'ReportSubscription',
                    s.name = $tid,
                    s.health_status = 'green',
                    s.version = 'v1',
                    s.updated_at = datetime()
                """,
                sid=sub.subscription_id,
                tid=sub.template_id,
                scope=json.dumps(sub.scope, ensure_ascii=False, sort_keys=True),
                modules=list(sub.modules),
                cron=sub.cron,
                recipients=list(sub.recipients),
                enabled=sub.enabled,
                created=sub.created_at,
                last_run=sub.last_run_at,
                last_status=sub.last_status,
                last_error=sub.last_error,
                last_report_id=sub.last_report_id,
            )
    except Exception:
        logger.warning("subscription dual-write failed: %s", sub.subscription_id, exc_info=True)


def _delete_subscription_node(sub_id: str) -> None:
    try:
        driver = n4j.get_driver()
    except Exception:
        return
    if driver is None:
        return

    try:
        with driver.session() as s:
            s.run(
                "MATCH (s:ReportSubscription {node_id: $sid}) DETACH DELETE s",
                sid=sub_id,
            )
    except Exception:
        logger.warning("subscription delete failed: %s", sub_id, exc_info=True)


def load_subscriptions_from_neo4j() -> int:
    """启动时反向 hydrate subscription_store。返回加载数量。

    Neo4j 不可达 / 无订阅节点 → 返回 0,不抛。
    """
    try:
        rows = n4j.run_query(
            """
            MATCH (s:ReportSubscription)
            RETURN s.subscription_id AS sid,
                   s.template_id AS tid,
                   s.scope_json AS scope,
                   s.modules AS modules,
                   s.cron AS cron,
                   s.recipients AS recipients,
                   s.enabled AS enabled,
                   s.created_at AS created,
                   s.last_run_at AS last_run,
                   s.last_status AS last_status,
                   s.last_error AS last_error,
                   s.last_report_id AS last_report_id
            """,
        )
    except Exception:
        logger.warning("load subscriptions from neo4j failed", exc_info=True)
        return 0

    count = 0
    for row in rows or []:
        try:
            sid = _row(row, "sid")
            if not sid:
                continue
            scope_str = _row(row, "scope") or "{}"
            try:
                scope = json.loads(scope_str)
            except (TypeError, ValueError):
                scope = {}
            sub = ReportSubscription(
                subscription_id=sid,
                template_id=_row(row, "tid") or "",
                scope=scope,
                modules=list(_row(row, "modules") or []),
                cron=_row(row, "cron") or "",
                recipients=list(_row(row, "recipients") or []),
                enabled=bool(_row(row, "enabled", True)),
                created_at=_row(row, "created") or "",
                last_run_at=_row(row, "last_run") or "",
                last_status=_row(row, "last_status") or "never",
                last_error=_row(row, "last_error") or "",
                last_report_id=_row(row, "last_report_id") or "",
            )
            subscription_store.add(sub)
            count += 1
        except Exception:
            logger.warning("skip malformed subscription row", exc_info=True)
    return count


def _row(row, key: str, default=None):
    """兼容 dict / Neo4j Record / SimpleNamespace。"""
    if isinstance(row, dict):
        return row.get(key, default)
    getter = getattr(row, "get", None)
    if callable(getter):
        try:
            return getter(key) if getter(key) is not None else default  # type: ignore[arg-type]
        except Exception:
            pass
    return getattr(row, key, default)


__all__ = [
    "_persist_subscription",
    "_delete_subscription_node",
    "load_subscriptions_from_neo4j",
]


# 显式类型供其他模块 import 时静态检查
_T = Optional[ReportSubscription]
