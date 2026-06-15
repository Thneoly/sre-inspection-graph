"""Pydantic 数据模型"""

from pydantic import BaseModel
from typing import Any


class GraphNode(BaseModel):
    id: str
    label: str
    type: str
    properties: dict[str, Any]


class GraphEdge(BaseModel):
    id: str
    source: str
    target: str
    type: str
    properties: dict[str, Any]


class GraphSummary(BaseModel):
    total_nodes: int
    total_edges: int
    risk_counts: dict[str, int]
    health_counts: dict[str, int]


class GraphResponse(BaseModel):
    nodes: list[GraphNode]
    edges: list[GraphEdge]
    summary: GraphSummary
