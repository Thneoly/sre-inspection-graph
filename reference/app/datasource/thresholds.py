"""资源类型容错阈值 — 不同类型的节点对故障的敏感度不同"""

from dataclasses import dataclass

@dataclass
class FaultThreshold:
    """某资源类型对特定故障的响应曲线"""
    degradation_delay: int      # 相对于故障源的延迟（秒）
    warning_at_pct: int         # 开始告警的故障百分比
    critical_at_pct: int        # 变为严重的故障百分比
    risk_multiplier: float      # 风险等级倍率（<1 表示比源节点更耐受）

# ═══════════════════════════════════════════════════
# 每种资源类型 × 每种故障类型的阈值
# ═══════════════════════════════════════════════════

NODE_THRESHOLDS: dict[str, dict[str, FaultThreshold]] = {
    # ── KubernetesNode ──
    "KubernetesNode": {
        "disk_pressure": FaultThreshold(degradation_delay=0,   warning_at_pct=70, critical_at_pct=90, risk_multiplier=1.0),
        "default":       FaultThreshold(degradation_delay=0,   warning_at_pct=60, critical_at_pct=85, risk_multiplier=1.0),
    },
    # ── Pod ──
    "Pod": {
        "disk_pressure": FaultThreshold(degradation_delay=120, warning_at_pct=50, critical_at_pct=80, risk_multiplier=0.8),
        "default":       FaultThreshold(degradation_delay=30,  warning_at_pct=50, critical_at_pct=80, risk_multiplier=1.0),
    },
    # ── Deployment ──
    "Deployment": {
        "disk_pressure": FaultThreshold(degradation_delay=300, warning_at_pct=40, critical_at_pct=75, risk_multiplier=0.6),
        "default":       FaultThreshold(degradation_delay=60,  warning_at_pct=30, critical_at_pct=60, risk_multiplier=0.7),
    },
    # ── ApplicationComponent ──
    "ApplicationComponent": {
        "disk_pressure": FaultThreshold(degradation_delay=600, warning_at_pct=30, critical_at_pct=65, risk_multiplier=0.5),
        "default":       FaultThreshold(degradation_delay=120, warning_at_pct=25, critical_at_pct=55, risk_multiplier=0.5),
    },
    # ── Application ──
    "Application": {
        "disk_pressure": FaultThreshold(degradation_delay=900, warning_at_pct=20, critical_at_pct=60, risk_multiplier=0.3),
        "default":       FaultThreshold(degradation_delay=300, warning_at_pct=20, critical_at_pct=50, risk_multiplier=0.3),
    },
    # ── MySQL (most sensitive to disk) ──
    "MySQL": {
        "disk_pressure": FaultThreshold(degradation_delay=60,  warning_at_pct=40, critical_at_pct=70, risk_multiplier=1.2),
        "default":       FaultThreshold(degradation_delay=30,  warning_at_pct=30, critical_at_pct=60, risk_multiplier=1.0),
    },
    # ── Redis (moderately sensitive) ──
    "Redis": {
        "disk_pressure": FaultThreshold(degradation_delay=180, warning_at_pct=50, critical_at_pct=80, risk_multiplier=0.9),
        "default":       FaultThreshold(degradation_delay=60,  warning_at_pct=40, critical_at_pct=70, risk_multiplier=0.8),
    },
    # ── Kafka (more tolerant) ──
    "Kafka": {
        "disk_pressure": FaultThreshold(degradation_delay=300, warning_at_pct=60, critical_at_pct=85, risk_multiplier=0.6),
        "default":       FaultThreshold(degradation_delay=60,  warning_at_pct=40, critical_at_pct=70, risk_multiplier=0.7),
    },
    # ── Service / Ingress / ELB / Gateway / APIG ──
    "Service": {
        "default": FaultThreshold(degradation_delay=180, warning_at_pct=30, critical_at_pct=65, risk_multiplier=0.4),
    },
    "Ingress": {
        "default": FaultThreshold(degradation_delay=240, warning_at_pct=25, critical_at_pct=60, risk_multiplier=0.3),
    },
    "ELB": {
        "default": FaultThreshold(degradation_delay=300, warning_at_pct=20, critical_at_pct=55, risk_multiplier=0.3),
    },
    "Gateway": {
        "default": FaultThreshold(degradation_delay=120, warning_at_pct=30, critical_at_pct=65, risk_multiplier=0.4),
    },
    "APIG": {
        "default": FaultThreshold(degradation_delay=180, warning_at_pct=25, critical_at_pct=60, risk_multiplier=0.3),
    },
    "Nacos": {
        "default": FaultThreshold(degradation_delay=60,  warning_at_pct=35, critical_at_pct=70, risk_multiplier=0.6),
    },
    # ── Storage types ──
    "ContainerImage": {
        "default": FaultThreshold(degradation_delay=600, warning_at_pct=15, critical_at_pct=50, risk_multiplier=0.2),
    },
    "ConfigMap": {
        "default": FaultThreshold(degradation_delay=300, warning_at_pct=20, critical_at_pct=50, risk_multiplier=0.2),
    },
    "Secret": {
        "default": FaultThreshold(degradation_delay=300, warning_at_pct=20, critical_at_pct=50, risk_multiplier=0.2),
    },
    "ContainerRegistry": {
        "default": FaultThreshold(degradation_delay=600, warning_at_pct=15, critical_at_pct=50, risk_multiplier=0.2),
    },
}


def get_threshold(node_type: str, fault_type: str) -> FaultThreshold:
    """Get the appropriate threshold for a node type + fault combination."""
    type_thresholds = NODE_THRESHOLDS.get(node_type, {})
    return type_thresholds.get(fault_type) or type_thresholds.get("default") or FaultThreshold(
        degradation_delay=60, warning_at_pct=40, critical_at_pct=70, risk_multiplier=0.5
    )


def compute_health(node_type: str, fault_type: str, source_health: str, source_risk: str, elapsed_seconds: int) -> tuple[str, str]:
    """基于节点类型的容错阈值，计算该节点此时应呈现的健康状态。

    Returns: (health_status, risk_level)
    """
    thr = get_threshold(node_type, fault_type)

    # Delay: if not enough time has passed, stay normal
    if elapsed_seconds < thr.degradation_delay:
        return ("normal", "low")

    # Source severity → percentage (0–100)
    severity_pct = {"normal": 20, "warning": 65, "critical": 100}.get(source_health, 20)

    # Apply risk multiplier (MySQL is MORE sensitive, Application LESS sensitive)
    effective_pct = min(100, severity_pct * thr.risk_multiplier)

    if effective_pct >= thr.critical_at_pct:
        h = "critical"
    elif effective_pct >= thr.warning_at_pct:
        h = "warning"
    else:
        h = "normal"

    # Risk level
    if effective_pct >= thr.critical_at_pct:
        r = source_risk if source_risk in ("high", "critical") else "high"
    elif effective_pct >= thr.warning_at_pct:
        r = "medium"
    else:
        r = "low"

    return (h, r)
