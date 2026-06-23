"""ChangeEvent 影响范围反向 BFS。

设计思路与 `app.recovery.cascade._walk` 同源 —— 都基于 DSS edges 反向走"被谁依赖"。
PRD-002 简化为单一规则:固定 PROPAGATION_EDGES + 固定深度上限,不带每条规则的
target_type / impact 严重度推导。

返回受影响资源 ID 列表(不含 target 自身)。
"""

from collections import deque
from typing import Optional

from app.datasource.store import store


# 影响传播沿这些"强依赖"关系。复用 view4_config_impact 的同款集合 ——
# 配置变更和"谁依赖配置"是同一张图。
PROPAGATION_EDGES: tuple[str, ...] = (
    "USES",
    "CONTAINS",
    "DEPLOYED_AS",
    "BELONGS_TO",
    "RUNS",
    "SCHEDULED_ON",
    "EXPOSES",
    "ROUTES_TO",
)


def derive_propagation(
    target_resource_id: str,
    max_depth: int = 4,
    edge_types: Optional[tuple[str, ...]] = None,
) -> list[str]:
    """从 target 沿 PROPAGATION_EDGES 反向 BFS,返回所有受影响的资源 ID。

    "反向"语义:edge.target_id == current → next = edge.source_id。
    例如 (Pod) -[USES]-> (ConfigMap),从 ConfigMap 反向走能命中 Pod。

    返回不含 target 自身。target 不在 DSS 时返回 []。
    """
    if store.get_node(target_resource_id) is None:
        return []

    edges = edge_types or PROPAGATION_EDGES
    edges_set = set(edges)

    # 一次性按 target_id 建索引,后续 BFS O(1) 查邻居
    incoming: dict[str, list[str]] = {}
    for edge in store.get_all_edges():
        if edge.relationship_type not in edges_set:
            continue
        incoming.setdefault(edge.target_id, []).append(edge.source_id)

    visited: set[str] = {target_resource_id}
    propagated: list[str] = []
    frontier: deque[tuple[str, int]] = deque([(target_resource_id, 0)])

    while frontier:
        node_id, depth = frontier.popleft()
        if depth >= max_depth:
            continue
        for neighbor in incoming.get(node_id, []):
            if neighbor in visited:
                continue
            visited.add(neighbor)
            propagated.append(neighbor)
            frontier.append((neighbor, depth + 1))

    return propagated


def find_propagation_path(
    source_event_target: str,
    affected_id: str,
    max_depth: int = 4,
    edge_types: Optional[tuple[str, ...]] = None,
) -> list[str]:
    """重建从 source_event_target → affected_id 的反向 BFS 最短路径(节点 ID 序列)。

    用于 /correlated 返回 propagation_distance,以及 /impact 返回路径。
    返回 [] 表示 affected_id 不可达(或 == source_event_target)。
    """
    if source_event_target == affected_id:
        return []
    if store.get_node(source_event_target) is None:
        return []

    edges = edge_types or PROPAGATION_EDGES
    edges_set = set(edges)

    incoming: dict[str, list[str]] = {}
    for edge in store.get_all_edges():
        if edge.relationship_type not in edges_set:
            continue
        incoming.setdefault(edge.target_id, []).append(edge.source_id)

    parents: dict[str, str] = {source_event_target: ""}
    frontier: deque[tuple[str, int]] = deque([(source_event_target, 0)])

    while frontier:
        node_id, depth = frontier.popleft()
        if depth >= max_depth:
            continue
        for neighbor in incoming.get(node_id, []):
            if neighbor in parents:
                continue
            parents[neighbor] = node_id
            if neighbor == affected_id:
                # 回溯路径
                path = [neighbor]
                cur = node_id
                while cur:
                    path.append(cur)
                    cur = parents.get(cur, "")
                return list(reversed(path))
            frontier.append((neighbor, depth + 1))

    return []


def find_descendants(
    start_id: str,
    max_depth: int = 6,
    edge_types: Optional[tuple[str, ...]] = None,
) -> list[str]:
    """从 start FORWARD BFS,返回所有"下属"资源 ID。

    与 derive_propagation 是镜像:
    - derive_propagation 走反向边("谁依赖我") — 用于 ConfigMap 影响范围
    - find_descendants 走正向边("我下属是谁") — 用于 application 的子树范围

    例: app -CONTAINS-> comp -DEPLOYED_AS-> deploy -CONTAINS-> pod -USES-> cm。
    从 app 起 forward 走能找全 comp / deploy / pod / cm。

    返回不含 start_id 自身。start 不在 DSS 时返回 []。
    """
    if store.get_node(start_id) is None:
        return []

    edges = edge_types or PROPAGATION_EDGES
    edges_set = set(edges)

    outgoing: dict[str, list[str]] = {}
    for edge in store.get_all_edges():
        if edge.relationship_type not in edges_set:
            continue
        outgoing.setdefault(edge.source_id, []).append(edge.target_id)

    visited: set[str] = {start_id}
    descendants: list[str] = []
    frontier: deque[tuple[str, int]] = deque([(start_id, 0)])

    while frontier:
        node_id, depth = frontier.popleft()
        if depth >= max_depth:
            continue
        for neighbor in outgoing.get(node_id, []):
            if neighbor in visited:
                continue
            visited.add(neighbor)
            descendants.append(neighbor)
            frontier.append((neighbor, depth + 1))

    return descendants
