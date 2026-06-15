"""DSS 数据模型"""
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any, Optional


@dataclass
class DataNode:
    id: str
    type: str
    name: str
    properties: dict[str, Any] = field(default_factory=dict)


@dataclass
class DataEdge:
    id: str
    source_id: str
    target_id: str
    relationship_type: str
    relationship_name: str = ""
    properties: dict[str, Any] = field(default_factory=dict)


@dataclass
class MetricSnapshot:
    snapshot_id: str
    resource_id: str
    metric_name: str
    current_value: float
    unit: str = "percent"
    fetched_at: str = ""
    warning_breached: bool = False
    critical_breached: bool = False


@dataclass
class FaultStage:
    sequence: int
    offset_seconds: int
    health: str
    risk: str
    metric_name: str = ""
    metric_value: float = 0.0
    unit: str = "percent"
    triggers_alert: bool = False
    triggers_finding: bool = False


@dataclass
class FaultInjection:
    injection_id: str
    fault_type: str
    target_id: str
    current_stage: int = 0
    total_stages: int = 0
    status: str = "injected"
    injected_at: str = ""
    stages: list[FaultStage] = field(default_factory=list)
