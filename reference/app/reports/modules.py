"""报告 5 大模块数据采集 — PRD-003 Sprint 1。

全部从 DSS store 采集(PRD §3.4 假设的 inspection_service / alert_service 不存在,
view routers 是纯 Neo4j、测试态 mock 返空 —— 详见 plan Context)。

每个 gather_* 返回一个 dict,直接喂给 Jinja2 模板对应 section。
"""

from __future__ import annotations

from collections import Counter
from typing import Any

from app.datasource.store import store
from app.recovery.action_defs import suggest_for_change
from app.reports.health_score import _app_subtree, _node_health, compute_health_score


# fault_type → 推荐 RecoveryAction(PRD-001 的 8 动作之一)
# 对齐 PRD §3.2 模块4「整合 PRD-001 的 RecoveryAction 推荐」
FAULT_ACTION_MAP: dict[str, str] = {
    "cpu_spike": "scale_deployment",
    "memory_leak": "restart_pod",
    "pod_crashloop": "rollback_deployment",
    "node_disk_pressure": "drain_node",
    "service_no_endpoints": "restart_service",
    "mysql_slow_query": "kill_query",
    "redis_unavailable": "clear_cache",
}


def gather_health_score(application_id: str, **_: Any) -> dict[str, Any]:
    """模块 1:健康度评分。"""
    return compute_health_score(application_id)


def gather_seven_views(application_id: str, time_range: dict[str, Any] | None = None, **_: Any) -> dict[str, Any]:
    """模块 2:7 视图结论汇总(DSS 适配版)。

    原 PRD 7 视图是 Neo4j 查询;Sprint 1 用 DSS 拓扑 + 健康 + 故障 + 变更 + 恢复统计
    等价覆盖"应用包含 N 组件/M Deployment/K Pod,健康分布,故障,变更,恢复"。
    """
    subtree = _app_subtree(application_id)
    nodes = [n for n in (store.get_node(nid) for nid in subtree) if n is not None]

    by_type: Counter[str] = Counter(n.type for n in nodes)
    health_counts: Counter[str] = Counter(_node_health(n) for n in nodes)

    # Deployment 就绪度
    not_ready_pods = [
        n for n in nodes
        if n.type == "Pod" and (n.properties or {}).get("phase") not in ("Running", None)
    ]

    active_faults = [
        f for f in store.get_active_faults() if f.target_id in subtree
    ]

    # 近 N 天变更统计
    since = (time_range or {}).get("time_range_start")
    until = (time_range or {}).get("time_range_end")
    changes = store.list_change_events(since=since, until=until)
    in_scope_changes = [c for c in changes if c.target_resource_id in subtree]
    change_by_type: Counter[str] = Counter(c.change_type for c in in_scope_changes)

    # 恢复执行统计
    executions = [
        e for e in store.get_all_executions() if e.target_resource_id in subtree
    ]
    exec_status: Counter[str] = Counter(e.status for e in executions)

    return {
        "application_id": application_id,
        "topology": {
            "components": by_type.get("ApplicationComponent", 0),
            "deployments": by_type.get("Deployment", 0),
            "pods": by_type.get("Pod", 0),
            "services": by_type.get("Service", 0),
            "total_nodes": len(nodes),
        },
        "health": {
            "normal": health_counts.get("normal", 0),
            "warning": health_counts.get("warning", 0),
            "critical": health_counts.get("critical", 0),
            "not_ready_pods": len(not_ready_pods),
        },
        "active_faults": [
            {
                "fault_type": f.fault_type,
                "target_id": f.target_id,
                "status": f.status,
                "stage": f"{f.current_stage}/{f.total_stages}",
            }
            for f in active_faults
        ],
        "changes": {
            "total": len(in_scope_changes),
            "by_type": dict(change_by_type),
        },
        "recoveries": {
            "total": len(executions),
            "succeeded": exec_status.get("succeeded", 0),
            "failed": exec_status.get("failed", 0),
            "rolled_back": exec_status.get("rolled_back", 0),
        },
    }


def gather_risk_list(application_id: str, **_: Any) -> dict[str, Any]:
    """模块 3:风险清单。按严重度分组(critical / warning / change)。

    critical = red-health 节点 + 活跃故障
    warning  = yellow-health 节点
    change   = high-severity ChangeEvent
    """
    subtree = _app_subtree(application_id)
    nodes = [n for n in (store.get_node(nid) for nid in subtree) if n is not None]

    critical: list[dict[str, Any]] = []
    warning: list[dict[str, Any]] = []

    for n in nodes:
        h = _node_health(n)
        entry = {
            "resource_id": n.id,
            "resource_type": n.type,
            "name": n.name,
            "reason": f"健康状态 {h}",
        }
        if h == "critical":
            critical.append(entry)
        elif h == "warning":
            warning.append(entry)

    # 活跃故障 → critical
    for f in store.get_active_faults():
        if f.target_id not in subtree:
            continue
        target = store.get_node(f.target_id)
        critical.append({
            "resource_id": f.target_id,
            "resource_type": target.type if target else "Unknown",
            "name": target.name if target else f.target_id,
            "reason": f"活跃故障 {f.fault_type} (阶段 {f.current_stage}/{f.total_stages})",
        })

    # high-severity 变更 → change 组
    changes = store.list_change_events()
    change_risks = [
        {
            "resource_id": c.target_resource_id,
            "resource_type": c.target_resource_type,
            "name": c.description or c.change_type,
            "reason": f"高危变更 {c.change_type} by {c.changed_by}",
            "changed_at": c.changed_at,
        }
        for c in changes
        if c.severity_estimate == "high" and c.target_resource_id in subtree
    ]

    return {
        "critical": critical,
        "warning": warning,
        "change": change_risks,
        "counts": {
            "critical": len(critical),
            "warning": len(warning),
            "change": len(change_risks),
        },
    }


def gather_recommended_actions(application_id: str, **_: Any) -> dict[str, Any]:
    """模块 4:推荐动作(整合 PRD-001)。

    对活跃故障按 fault_type 映射动作;对 high-severity 变更调 suggest_for_change。
    去重(同 action_id + target)。
    """
    subtree = _app_subtree(application_id)
    seen: set[tuple[str, str]] = set()
    actions: list[dict[str, Any]] = []

    # 故障 → 动作
    for f in store.get_active_faults():
        if f.target_id not in subtree:
            continue
        action_id = FAULT_ACTION_MAP.get(f.fault_type)
        if not action_id:
            continue
        key = (action_id, f.target_id)
        if key in seen:
            continue
        seen.add(key)
        actions.append({
            "action_id": action_id,
            "target_resource_id": f.target_id,
            "rationale": f"故障 {f.fault_type} → 推荐 {action_id}",
            "source": "fault",
        })

    # 高危变更 → 动作(复用 PRD-002 Phase 2 的 suggest_for_change)
    for c in store.list_change_events():
        if c.target_resource_id not in subtree or c.severity_estimate != "high":
            continue
        for sugg in suggest_for_change(c.change_type):
            key = (sugg["action_id"], c.target_resource_id)
            if key in seen:
                continue
            seen.add(key)
            actions.append({
                "action_id": sugg["action_id"],
                "target_resource_id": c.target_resource_id,
                "rationale": sugg.get("rationale", ""),
                "source": "change",
            })

    return {"actions": actions, "total": len(actions)}


def gather_historical_trends(application_id: str, days: int = 7, **_: Any) -> dict[str, Any]:
    """模块 5:历史趋势(文本表格)。

    近 N 天按天聚合:变更计数、恢复执行计数、每日健康度估算。
    无 matplotlib —— Sprint 1 渲染文本表格,图表留 Phase 2。
    """
    subtree = _app_subtree(application_id)

    # 按天分桶(ISO date 字典序 == 日期序)
    change_by_day: Counter[str] = Counter()
    for c in store.list_change_events():
        if c.target_resource_id not in subtree:
            continue
        day = c.changed_at[:10]  # YYYY-MM-DD
        if day:
            change_by_day[day] += 1

    recovery_by_day: Counter[str] = Counter()
    for e in store.get_all_executions():
        if e.target_resource_id not in subtree:
            continue
        day = (e.initiated_at or "")[:10]
        if day:
            recovery_by_day[day] += 1

    days_set = sorted(set(change_by_day) | set(recovery_by_day))
    rows = [
        {
            "date": d,
            "changes": change_by_day.get(d, 0),
            "recoveries": recovery_by_day.get(d, 0),
        }
        for d in days_set
    ]

    return {
        "application_id": application_id,
        "days": days,
        "rows": rows,
        "total_changes": sum(change_by_day.values()),
        "total_recoveries": sum(recovery_by_day.values()),
    }


# 模块名 → 采集函数(给 generator 顺序调用)
MODULE_GATHERERS: dict[str, Any] = {
    "health_score": gather_health_score,
    "seven_views": gather_seven_views,
    "risk_list": gather_risk_list,
    "recommended_actions": gather_recommended_actions,
    "historical_trends": gather_historical_trends,
}


# ============================================================
# 模板路由表 — generator 按 template_id 取对应 gatherer 字典
# ============================================================

def _get_template_gatherers() -> dict[str, dict[str, Any]]:
    """延迟 import 避免循环依赖(cluster_modules / incident_modules 反向引用 health_score)。"""
    from app.reports.cluster_modules import CLUSTER_MODULE_GATHERERS
    from app.reports.incident_modules import INCIDENT_MODULE_GATHERERS

    return {
        "application_health": MODULE_GATHERERS,
        "cluster_overview": CLUSTER_MODULE_GATHERERS,
        "incident_report": INCIDENT_MODULE_GATHERERS,
    }


def gatherers_for_template(template_id: str) -> dict[str, Any]:
    """按模板返回 {模块名: gather 函数} 字典。未知模板返回空。"""
    return _get_template_gatherers().get(template_id, {})
