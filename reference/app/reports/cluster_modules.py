"""集群级报告模块 — PRD-003 Sprint 2 `cluster_overview` 模板。

输入 cluster_id(可空 = 全公司)。所有模块从 DSS 采集,与 Sprint 1 application_health
保持同源 — 真实数据走 PRD-004 connectors 写入的节点(`discovery_method=k8s_connector`),
mock 走基线数据。

cluster_id 过滤(简化):resource_id prefix 匹配,如 `comp:vm-cluster:*` 命中所有该集群
组件。L1 模型里 KubernetesCluster 不直接 CONTAINS Application,反向 BFS 走不通 ——
Phase 2 通过 namespace + cluster_id 索引精细化(已在 CLAUDE.md 标注)。
"""

from __future__ import annotations

from collections import Counter
from typing import Any, Optional

from app.datasource.store import store
from app.reports.health_score import compute_health_score


def _matches_cluster(resource_id: str, cluster_id: Optional[str]) -> bool:
    """cluster_id None → 全匹配;否则 prefix 匹配(`comp:vm-cluster:*` / `app:vm-cluster:*`)。"""
    if not cluster_id:
        return True
    # 同时容忍 cluster_id 形如 "vm-cluster" / "cluster:vm-cluster"
    needle = cluster_id.split(":", 1)[-1]
    return f":{needle}:" in f":{resource_id}:" or resource_id.startswith(f"{needle}:")


def _list_applications(cluster_id: Optional[str]) -> list:
    """返回所有 Application 类型节点(可选 cluster_id prefix 过滤)。"""
    return [
        n for n in store.get_all_nodes()
        if n.type == "Application" and _matches_cluster(n.id, cluster_id)
    ]


def gather_cluster_health(cluster_id: Optional[str] = None, **_: Any) -> dict[str, Any]:
    """模块 1:集群健康总览。

    每个 Application 算 score → 聚合 rating 分布 + 按 score 升序应用列表。
    """
    apps = _list_applications(cluster_id)
    results = []
    rating_counter: Counter[str] = Counter()

    for app in apps:
        score_info = compute_health_score(app.id)
        results.append({
            "application_id": app.id,
            "name": app.name,
            "score": score_info["score"],
            "rating": score_info["rating"],
        })
        rating_counter[score_info["rating"]] += 1

    results.sort(key=lambda r: r["score"])  # 风险高在前

    return {
        "cluster_id": cluster_id or "all",
        "total_apps": len(apps),
        "rating_counts": {
            "健康": rating_counter.get("健康", 0),
            "健康警告": rating_counter.get("健康警告", 0),
            "风险中": rating_counter.get("风险中", 0),
            "风险高": rating_counter.get("风险高", 0),
        },
        "apps": results,
    }


def gather_cluster_risk_top_n(cluster_id: Optional[str] = None, top_n: int = 10, **_: Any) -> dict[str, Any]:
    """模块 2:Top-N 风险应用 + 全局风险指标。"""
    health = gather_cluster_health(cluster_id)
    top_apps = health["apps"][:top_n]

    # 全局活跃故障 / 高危变更(仅在 cluster 范围内的)
    active_faults = [
        f for f in store.get_active_faults()
        if _matches_cluster(f.target_id, cluster_id)
    ]
    high_changes = [
        c for c in store.list_change_events()
        if c.severity_estimate == "high" and _matches_cluster(c.target_resource_id, cluster_id)
    ]

    return {
        "cluster_id": cluster_id or "all",
        "top_n": top_n,
        "top_apps": top_apps,
        "active_faults_total": len(active_faults),
        "high_severity_changes_total": len(high_changes),
    }


def gather_cluster_changes(
    cluster_id: Optional[str] = None,
    time_range: dict[str, Any] | None = None,
    **_: Any,
) -> dict[str, Any]:
    """模块 3:跨应用变更汇总。by_type + Top-5 受变更最多的应用。"""
    since = (time_range or {}).get("time_range_start")
    until = (time_range or {}).get("time_range_end")
    events = [
        c for c in store.list_change_events(since=since, until=until)
        if _matches_cluster(c.target_resource_id, cluster_id)
    ]

    by_type: Counter[str] = Counter(c.change_type for c in events)
    by_target: Counter[str] = Counter(c.target_resource_id for c in events)
    top_targets = [
        {"resource_id": rid, "changes": cnt}
        for rid, cnt in by_target.most_common(5)
    ]

    return {
        "cluster_id": cluster_id or "all",
        "total": len(events),
        "by_type": dict(by_type),
        "top_targets": top_targets,
    }


def gather_cluster_recoveries(
    cluster_id: Optional[str] = None,
    time_range: dict[str, Any] | None = None,
    **_: Any,
) -> dict[str, Any]:
    """模块 4:跨应用恢复执行汇总。状态分布 + 成功率。"""
    executions = [
        e for e in store.get_all_executions()
        if _matches_cluster(e.target_resource_id, cluster_id)
    ]
    # 可选时间过滤:initiated_at 在 [since, until]
    since = (time_range or {}).get("time_range_start")
    until = (time_range or {}).get("time_range_end")
    if since:
        executions = [e for e in executions if (e.initiated_at or "") >= since]
    if until:
        executions = [e for e in executions if (e.initiated_at or "") <= until]

    status_counts: Counter[str] = Counter(e.status for e in executions)
    succeeded = status_counts.get("succeeded", 0)
    total_terminal = sum(status_counts.get(s, 0) for s in ("succeeded", "failed", "rolled_back"))
    success_rate = round(succeeded / total_terminal, 3) if total_terminal else 0.0

    return {
        "cluster_id": cluster_id or "all",
        "total": len(executions),
        "status_counts": dict(status_counts),
        "success_rate": success_rate,
    }


# 模板模块名 → 采集函数
CLUSTER_MODULE_GATHERERS: dict[str, Any] = {
    "cluster_health": gather_cluster_health,
    "cluster_risk_top_n": gather_cluster_risk_top_n,
    "cluster_changes": gather_cluster_changes,
    "cluster_recoveries": gather_cluster_recoveries,
}
