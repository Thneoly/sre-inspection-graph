"""Recovery Action Dry-Run Cascade — 反向影响范围计算。

输入:动作模板 + 目标资源
输出:受影响资源列表 + 严重度 + 估算 SLA

设计借鉴 `app.datasource.fault_injector._find_blast_targets` 的图遍历思路,
但有两个本质区别:

1. **不修改图状态** — fault_injector 注入故障会改 health/risk;cascade 只读不写
2. **多规则叠加** — 一个动作的 propagation 列表可能含 N 条规则,每条沿不同关系遍历;
   结果合并并去重

复用 `app.datasource.store` 作为图源(运行时孪生体),不直接查 Neo4j——
这样 dry-run 响应可以 < 100ms。
"""

from typing import Optional

from app.datasource.store import store
from app.datasource.models import DataNode
from app.recovery.action_defs import ACTION_DEFS, get_action


# 严重度优先级:high > medium > low > minimal(用于多规则命中同一节点时取最大值)
_IMPACT_RANK = {"minimal": 0, "low": 1, "medium": 2, "high": 3}
_REVERSE_IMPACT_RANK = {v: k for k, v in _IMPACT_RANK.items()}


def dry_run(action_id: str, target_resource_id: str,
            input_params: Optional[dict] = None) -> dict:
    """对一个 (动作 + 目标) 计算影响范围。

    返回结构:
    {
        "action_id": "scale_deployment",
        "target_resource_id": "deploy:...",
        "target_valid": True,
        "validation_error": None,
        "affected_resources": [
            {"resource_id": "...", "type": "Pod", "name": "...",
             "impact_severity": "minimal",
             "via_relations": ["CONTAINS"],
             "notes": ["新增/减少 Pod 副本"]},
            ...
        ],
        "estimated_duration_seconds": 90,
        "estimated_sla_impact": "< 0.1%",
        "warnings": [...],
        "rollback_action_id": "scale_deployment",
        "rollback_input_params": {"replicas_delta": -1},
    }

    若动作不存在或目标类型不匹配,target_valid=False,返回原因不抛异常
    (这样前端可以友好提示)。
    """
    action = get_action(action_id)
    if action is None:
        return _invalid_result(action_id, target_resource_id,
                               f"unknown action_id: {action_id}")

    target_node = store.get_node(target_resource_id)
    if target_node is None:
        return _invalid_result(action_id, target_resource_id,
                               f"target resource not found in DSS: {target_resource_id}",
                               action=action)

    if target_node.type != action["target_type"]:
        return _invalid_result(action_id, target_resource_id,
                               f"action targets {action['target_type']} but resource is {target_node.type}",
                               action=action)

    # 沿每条 propagation 规则遍历,合并结果
    affected: dict[str, dict] = {}    # resource_id -> impact dict
    for rule in action.get("propagation", []):
        for hit in _walk(target_node, rule):
            rid = hit["resource_id"]
            existing = affected.get(rid)
            if existing is None:
                affected[rid] = hit
            else:
                # 严重度取较大值
                if _IMPACT_RANK[hit["impact_severity"]] > _IMPACT_RANK[existing["impact_severity"]]:
                    existing["impact_severity"] = hit["impact_severity"]
                # 关系类型 / 注释合并
                for rel in hit["via_relations"]:
                    if rel not in existing["via_relations"]:
                        existing["via_relations"].append(rel)
                for note in hit["notes"]:
                    if note not in existing["notes"]:
                        existing["notes"].append(note)

    # 排除自身(目标节点)
    affected.pop(target_resource_id, None)

    # 按严重度倒序 + resource_id 排序,稳定输出
    affected_list = sorted(
        affected.values(),
        key=lambda x: (-_IMPACT_RANK[x["impact_severity"]], x["resource_id"]),
    )

    # 计算回滚参数(如果是 scale_deployment 的反向 delta)
    rollback_params = _compute_rollback_params(action, input_params or {})

    return {
        "action_id": action_id,
        "action_name": action["name"],
        "target_resource_id": target_resource_id,
        "target_resource_type": target_node.type,
        "target_resource_name": target_node.name,
        "target_valid": True,
        "validation_error": None,
        "affected_resources": affected_list,
        "affected_count": len(affected_list),
        "estimated_duration_seconds": action["estimated_duration_seconds"],
        "estimated_sla_impact": action["sla_impact_estimate"],
        "warnings": list(action.get("warnings", [])),
        "rollback_action_id": action.get("rollback_action_id"),
        "rollback_input_params": rollback_params,
        "risk_level": action["risk_level"],
        "requires_approval": action["requires_approval"],
    }


def _walk(start: DataNode, rule: dict) -> list[dict]:
    """从 start 节点出发沿 rule 描述的关系遍历,返回所有命中节点。

    BFS,避免深度遍历重复访问。
    """
    edge_type = rule["edge"]
    direction = rule["direction"]    # "forward" | "reverse"
    max_depth = rule.get("max_depth", 3)
    target_type = rule.get("target_type")
    impact = rule.get("impact", "low")
    note = rule.get("note", "")

    visited: set[str] = {start.id}
    frontier: list[str] = [start.id]
    hits: list[dict] = []

    for _ in range(max_depth):
        next_frontier: list[str] = []
        for node_id in frontier:
            for edge in store.get_all_edges():
                if edge.relationship_type != edge_type:
                    continue

                # 选下一跳:forward = source→target,reverse = target→source
                if direction == "forward" and edge.source_id == node_id:
                    next_id = edge.target_id
                elif direction == "reverse" and edge.target_id == node_id:
                    next_id = edge.source_id
                else:
                    continue

                if next_id in visited:
                    continue
                visited.add(next_id)
                next_frontier.append(next_id)

                # 类型筛选
                next_node = store.get_node(next_id)
                if next_node is None:
                    continue
                if target_type and next_node.type != target_type:
                    continue

                hits.append({
                    "resource_id": next_id,
                    "type": next_node.type,
                    "name": next_node.name,
                    "impact_severity": impact,
                    "via_relations": [edge_type],
                    "notes": [note] if note else [],
                })

        if not next_frontier:
            break
        frontier = next_frontier

    return hits


def _compute_rollback_params(action: dict, input_params: dict) -> dict | None:
    """对支持回滚的动作,计算回滚参数。

    Sprint 1 只处理 scale_deployment 的对称回滚:正负 delta 互换。
    其他动作的 rollback_action_id 为 None(不可逆)或 self(再 rollout 一次)。
    """
    rollback_id = action.get("rollback_action_id")
    if not rollback_id:
        return None

    if action["category"] == "scale" and "replicas_delta" in input_params:
        return {"replicas_delta": -input_params["replicas_delta"]}

    if rollback_id == action["name"] or rollback_id in ("rollback_deployment",):
        # 回滚动作自身没有简单参数(rollout undo 默认行为)
        return {}

    return None


def _invalid_result(action_id: str, target_resource_id: str,
                    error: str, action: dict | None = None) -> dict:
    """构造 target_valid=False 的响应。"""
    return {
        "action_id": action_id,
        "action_name": action["name"] if action else None,
        "target_resource_id": target_resource_id,
        "target_resource_type": None,
        "target_resource_name": None,
        "target_valid": False,
        "validation_error": error,
        "affected_resources": [],
        "affected_count": 0,
        "estimated_duration_seconds": action["estimated_duration_seconds"] if action else 0,
        "estimated_sla_impact": action["sla_impact_estimate"] if action else "n/a",
        "warnings": list(action.get("warnings", [])) if action else [],
        "rollback_action_id": action.get("rollback_action_id") if action else None,
        "rollback_input_params": None,
        "risk_level": action["risk_level"] if action else None,
        "requires_approval": action["requires_approval"] if action else None,
    }
