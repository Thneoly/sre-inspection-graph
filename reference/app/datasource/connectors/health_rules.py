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


# ============================================================
# PRD-004 Phase 2 — AlertRule 生成(从 QueryDef 阈值)
# ============================================================

def generate_alert_rules() -> list:
    """从 QUERIES 的 warning / critical 阈值生成 AlertRule 列表。

    每个 QueryDef 产出 2 条 rule(warning + critical),request_rate 这种
    阈值设成天文数字(1e9)的实际不告警但仍生成 rule(enabled=True,只是永不触发)。
    """
    from app.datasource.models import AlertRule

    rules: list[AlertRule] = []
    for q in QUERIES:
        for sev, threshold in (("critical", q.critical), ("warning", q.warning)):
            rules.append(AlertRule(
                rule_id=f"alert_rule:{q.name}:{sev}",
                metric_name=q.name,
                severity=sev,
                threshold=float(threshold),
                direction=q.direction,
                unit=q.unit,
                description=f"{q.name} {sev} breach (>= {threshold} {q.unit})",
                enabled=True,
            ))
    return rules


def sync_alert_rules_to_store() -> int:
    """把 generate_alert_rules() 的结果 upsert 到 DSS store。返回规则数。

    幂等:rule_id 固定,重复调用覆盖。启动时调一次。
    """
    from app.datasource.store import store
    rules = generate_alert_rules()
    for r in rules:
        store.upsert_alert_rule(r)
    return len(rules)
