"""Metrics Service — 指标值处理和阈值检查"""
from datetime import datetime, date


def _serialize(val):
    """Convert Neo4j types to JSON-safe Python types"""
    if val is None:
        return ""
    if isinstance(val, (datetime, date)):
        return val.isoformat()
    if hasattr(val, 'iso_format'):
        return val.iso_format()
    if hasattr(val, 'to_native'):
        n = val.to_native()
        if isinstance(n, (datetime, date)):
            return n.isoformat()
        return str(n)
    return val


def format_metrics_from_snapshots(snapshots: list) -> list[dict]:
    """将 MetricSnapshot 查询结果格式化为 API 响应"""
    metrics = []
    for snap in snapshots:
        metrics.append({
            "id": str(snap.get("snapshot_id", "")),
            "metric_name": str(snap.get("metric_name", "")),
            "current_value": float(snap.get("current_value", 0)),
            "unit": str(snap.get("unit", "")),
            "fetched_at": _serialize(snap.get("fetched_at")),
            "is_stale": str(snap.get("is_stale", "false")) == "true",
            "warning_breached": str(snap.get("warning_breached", "false")) == "true",
            "critical_breached": str(snap.get("critical_breached", "false")) == "true",
            "warning_threshold": _safe_float(snap.get("warning_threshold")),
            "critical_threshold": _safe_float(snap.get("critical_threshold")),
        })
    return metrics


def check_threshold(value: float, warning: float | None, critical: float | None) -> str:
    """检查值是否超过阈值，返回 'normal' | 'warning' | 'critical'"""
    if critical is not None and value >= critical:
        return "critical"
    if warning is not None and value >= warning:
        return "warning"
    return "normal"


def _safe_float(val):
    """安全转换为 float"""
    if val is None:
        return None
    try:
        return float(val)
    except (ValueError, TypeError):
        return None
