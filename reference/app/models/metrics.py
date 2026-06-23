"""Metric 相关模型"""

from pydantic import BaseModel
from typing import Optional
from datetime import datetime


class MetricSnapshotOut(BaseModel):
    id: str
    metric_name: str
    current_value: float
    unit: str
    fetched_at: datetime
    is_stale: bool
    warning_breached: bool
    critical_breached: bool
    warning_threshold: Optional[float] = None
    critical_threshold: Optional[float] = None


class ResourceMetricsResponse(BaseModel):
    resource_id: str
    metrics: list[MetricSnapshotOut]
