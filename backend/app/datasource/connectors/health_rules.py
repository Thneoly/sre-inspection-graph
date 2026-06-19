"""节点健康度推导 — PRD-004 Sprint 2。

输入:DSS 节点 + 该节点的最新 MetricSnapshot 列表
输出:health = green / yellow / red

规则(优先级从高到低):
1. 任何 critical 阈值越线 → red
2. 任何 warning 阈值越线 → yellow
3. 没 metric 数据 → 保持节点原有 health(避免 Prom 暂停时把节点全刷绿)
4. 都正常 → green

设计要点:
- 阈值定义跟着 PromQL 模板走(在 prometheus_queries.QUERIES.warning/critical)
- breach 判定时要看是"高了不好"还是"低了不好"
  当前所有指标都是"高了不好"(CPU%、错误率、P99 延迟),如未来加 throughput 之类
  "低了不好"的指标,要在 QueryDef 里加 direction 字段
"""

from __future__ import annotations

import logging
from typing import Iterable, Optional

from app.datasource.connectors.prometheus_queries import QUERIES, QueryDef
from app.datasource.models import MetricSnapshot


logger = logging.getLogger(__name__)


_QUERY_BY_NAME: dict[str, QueryDef] = {q.name: q for q in QUERIES}


def derive_health(snapshots: Iterable[MetricSnapshot]) -> Optional[str]:
    """根据一组 metric snapshot 给出节点 health。

    返回 None 表示"没数据,不要刷新原 health";否则返回 "green" / "yellow" / "red"。
    """
    snaps = list(snapshots)
    if not snaps:
        return None

    has_critical = False
    has_warning = False
    for snap in snaps:
        q = _QUERY_BY_NAME.get(snap.metric_name)
        if q is None:
            continue
        if snap.current_value >= q.critical:
            has_critical = True
        elif snap.current_value >= q.warning:
            has_warning = True
    if has_critical:
        return "red"
    if has_warning:
        return "yellow"
    return "green"


def evaluate_breach(metric_name: str, value: float) -> tuple[bool, bool]:
    """判定单条 metric 的 (warning_breached, critical_breached)。"""
    q = _QUERY_BY_NAME.get(metric_name)
    if q is None:
        return False, False
    return value >= q.warning, value >= q.critical
