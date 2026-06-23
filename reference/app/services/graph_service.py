"""Graph Service — Neo4j 查询结果转 GraphResponse"""

from collections import Counter

from app.models.graph import GraphNode, GraphEdge, GraphSummary, GraphResponse

# 有效的 relationship_type 值
VALID_REL_TYPES = {
    'CONTAINS', 'DEPLOYED_AS', 'DEPLOYED_IN', 'BELONGS_TO',
    'EXPOSES', 'ROUTES_TO', 'USES', 'STORED_IN',
    'MONITORS', 'VISUALIZES',
    'RUNS', 'SCHEDULED_ON',
    'GENERATED', 'VIOLATES', 'AFFECTS', 'PROPAGATES_TO',
    'FIRED_ON', 'AGGREGATES_TO',
    'MEASURES',
}


def format_graph_response(records: list[dict]) -> GraphResponse:
    """将 Neo4j 查询结果格式化为 GraphResponse"""

    nodes_map: dict[str, dict] = {}
    edges_map: dict[str, dict] = {}

    for record in records:
        # 处理 path 类型结果
        path = record.get("path")
        if path is not None:
            _extract_from_path(path, nodes_map, edges_map)
        elif record.get("nodes") is not None and record.get("edges") is not None:
            _extract_nodes(record.get("nodes"), nodes_map)
            _extract_edges(record.get("edges"), edges_map)
        else:
            # 尝试直接从 record 提取节点和关系
            for value in record.values():
                if hasattr(value, 'labels'):
                    _add_graph_node(value, nodes_map)
                elif hasattr(value, 'type') and hasattr(value, 'start_node'):
                    _add_graph_edge(value, edges_map)

    nodes = list(nodes_map.values())
    edges = list(edges_map.values())

    # 统计
    risk_counter = Counter()
    health_counter = Counter()
    for n in nodes:
        props = n.get("properties", {})
        risk = props.get("risk_level", "unknown")
        health = props.get("health_status", "unknown")
        risk_counter[risk] += 1
        health_counter[health] += 1

    summary = GraphSummary(
        total_nodes=len(nodes),
        total_edges=len(edges),
        risk_counts={
            "high": risk_counter.get("high", 0),
            "medium": risk_counter.get("medium", 0),
            "low": risk_counter.get("low", 0),
            "unknown": risk_counter.get("unknown", 0),
        },
        health_counts={
            "normal": health_counter.get("normal", 0),
            "warning": health_counter.get("warning", 0),
            "critical": health_counter.get("critical", 0),
            "unknown": health_counter.get("unknown", 0),
        },
    )

    return GraphResponse(nodes=nodes, edges=edges, summary=summary)


def _extract_from_path(path, nodes_map, edges_map):
    """从 Neo4j Path 对象提取节点和边"""
    for node in path.nodes:
        _add_graph_node(node, nodes_map)
    for rel in path.relationships:
        _add_graph_edge(rel, edges_map)


def _extract_nodes(nodes, nodes_map):
    for node in nodes:
        _add_graph_node(node, nodes_map)


def _extract_edges(edges, edges_map):
    for edge in edges:
        _add_graph_edge(edge, edges_map)


def _add_graph_node(node, nodes_map):
    node_id = str(node.get("node_id", node.get("id", node.element_id)))
    if node_id in nodes_map:
        return

    labels = list(node.labels) if hasattr(node, 'labels') else []
    resource_labels = [l for l in labels if l not in ("ResourceInstance", "ResourceType")]
    node_type = resource_labels[0] if resource_labels else (node.get("label", "Unknown"))

    props = {}
    for k, v in dict(node).items():
        if k.startswith("_"):
            continue
        props[k] = _serialize_value(v)

    nodes_map[node_id] = GraphNode(
        id=node_id,
        label=str(node.get("label", node_type)),
        type=node_type,
        properties=props,
    ).model_dump()


def _add_graph_edge(rel, edges_map):
    edge_id = str(rel.get("edge_id", rel.element_id))
    if edge_id in edges_map:
        return

    rel_type = str(rel.get("relationship_type", rel.type))
    source_id = str(rel.start_node.get("node_id", rel.start_node.element_id))
    target_id = str(rel.end_node.get("node_id", rel.end_node.element_id))

    props = {"relationship_name": str(rel.get("relationship_name", rel_type))}
    for k, v in dict(rel).items():
        if k.startswith("_") or k in ("relationship_type", "relationship_name", "edge_id"):
            continue
        props[k] = _serialize_value(v)

    edges_map[edge_id] = GraphEdge(
        id=edge_id,
        source=source_id,
        target=target_id,
        type=rel_type,
        properties=props,
    ).model_dump()


def _serialize_value(value):
    """处理 Neo4j 返回的特殊类型 → JSON-serializable Python types"""
    from datetime import datetime, date
    if isinstance(value, (datetime, date)):
        return value.isoformat()
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="replace")
    # Neo4j temporal types (DateTime, Date, Time, Duration)
    if hasattr(value, 'iso_format'):
        return value.iso_format()
    if hasattr(value, 'to_native'):
        native = value.to_native()
        if isinstance(native, (datetime, date)):
            return native.isoformat()
        return str(native)
    # Catch-all for other Neo4j types
    cls_name = type(value).__name__
    if 'neo4j' in str(type(value)).lower():
        return str(value)
    return value
