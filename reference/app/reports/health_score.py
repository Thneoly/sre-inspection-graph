"""健康度评分 — PRD-003 Sprint 1 模块 1。

PRD §3.2 模块1 原公式:
    基础 100;critical Finding -10/个;warning Finding -3/个;故障 Pod -2/个
    60-79 健康警告 / 40-59 风险中 / 0-39 风险高

**适配**:DSS 里没有 InspectionFinding 模型(那是 L4 Neo4j 节点,测试态 mock 返空)。
Sprint 1 用 DSS 可得的"节点健康度 + 活跃故障"等价映射:
    - critical = red-health 节点数 + 活跃 fault 目标数      (×10)
    - warning  = yellow-health 节点数                        (×3)
    - fault_pod = 活跃 fault 中 target 类型为 Pod 的数        (×2)
Phase 2 接入真实巡检 Finding 后切回原公式,接口不变。
"""

from __future__ import annotations

from typing import Any

from app.changes.propagation import find_descendants
from app.datasource.store import store


def _app_subtree(application_id: str) -> set[str]:
    """应用本身 + 正向 BFS 子树(find_descendants 沿 CONTAINS/DEPLOYED_AS/... 正向走)。"""
    if store.get_node(application_id) is None:
        return set()
    return {application_id} | set(find_descendants(application_id, max_depth=6))


def _node_health(node) -> str:
    """读 properties.health_status,归一到 normal/warning/critical。"""
    raw = (node.properties or {}).get("health_status", "normal")
    if raw in ("critical", "red"):
        return "critical"
    if raw in ("warning", "yellow"):
        return "warning"
    return "normal"


def compute_health_score(application_id: str) -> dict[str, Any]:
    """计算应用健康度评分。

    返回:
        {
          "application_id": ...,
          "score": int 0-100,
          "rating": "健康" | "健康警告" | "风险中" | "风险高",
          "breakdown": {"critical": int, "warning": int, "fault_pod": int, "total_nodes": int},
        }
    """
    subtree = _app_subtree(application_id)
    nodes = [store.get_node(nid) for nid in subtree]
    nodes = [n for n in nodes if n is not None]

    critical = 0
    warning = 0
    for n in nodes:
        h = _node_health(n)
        if h == "critical":
            critical += 1
        elif h == "warning":
            warning += 1

    # 活跃故障:目标在子树内的算到本应用;Pod 类目标额外计 fault_pod
    fault_pod = 0
    for fault in store.get_active_faults():
        if fault.target_id not in subtree:
            continue
        target = store.get_node(fault.target_id)
        if target is not None and target.type == "Pod":
            fault_pod += 1
        critical += 1  # 活跃故障本身视作 critical 项(对齐 PRD "故障 Pod" + critical 语义)

    score = max(0, 100 - critical * 10 - warning * 3 - fault_pod * 2)
    rating = _rating(score)

    return {
        "application_id": application_id,
        "score": score,
        "rating": rating,
        "breakdown": {
            "critical": critical,
            "warning": warning,
            "fault_pod": fault_pod,
            "total_nodes": len(nodes),
        },
    }


def _rating(score: int) -> str:
    if score >= 80:
        return "健康"
    if score >= 60:
        return "健康警告"
    if score >= 40:
        return "风险中"
    return "风险高"
