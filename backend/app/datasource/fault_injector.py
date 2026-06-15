"""故障注入引擎 — 通过 DSS 注入故障数据"""
import json
from datetime import datetime, timezone
from app.datasource.models import FaultInjection, FaultStage, MetricSnapshot
from app.datasource.store import store

FAULT_DEFS = {
    "cpu_spike": {
        "name": "Pod CPU 飙升", "target_type": "Pod",
        "propagate_to": ["Deployment", "ApplicationComponent"],
        "stages": [
            {"s": 0,   "h": "normal",   "r": "low",     "v": 45.2, "m": "cpu_usage", "u": "percent"},
            {"s": 180, "h": "warning",  "r": "medium",  "v": 86.5, "m": "cpu_usage", "u": "percent", "alert": True},
            {"s": 360, "h": "critical", "r": "high",    "v": 93.0, "m": "cpu_usage", "u": "percent", "alert": True},
            {"s": 600, "h": "critical", "r": "high",    "v": 96.2, "m": "cpu_usage", "u": "percent", "alert": True, "finding": True},
            {"s": 900, "h": "warning",  "r": "medium",  "v": 65.0, "m": "cpu_usage", "u": "percent"},
            {"s": 1200,"h": "normal",   "r": "low",     "v": 48.0, "m": "cpu_usage", "u": "percent"},
        ],
    },
    "memory_leak": {
        "name": "内存泄漏", "target_type": "Pod",
        "propagate_to": ["Deployment"],
        "stages": [
            {"s": 0,   "h": "normal",   "r": "low",     "v": 55.0, "m": "memory_usage", "u": "percent"},
            {"s": 300, "h": "warning",  "r": "medium",  "v": 85.0, "m": "memory_usage", "u": "percent", "alert": True},
            {"s": 600, "h": "critical", "r": "high",    "v": 94.5, "m": "memory_usage", "u": "percent", "alert": True},
            {"s": 900, "h": "critical", "r": "critical", "v": 98.0, "m": "memory_usage", "u": "percent", "alert": True, "finding": True},
            {"s": 1200,"h": "critical", "r": "high",    "v": 90.0, "m": "memory_usage", "u": "percent", "alert": True},
            {"s": 1500,"h": "normal",   "r": "low",     "v": 55.0, "m": "memory_usage", "u": "percent"},
        ],
    },
    "pod_crashloop": {
        "name": "Pod CrashLoop", "target_type": "Pod",
        "propagate_to": ["Deployment", "ApplicationComponent"],
        "stages": [
            {"s": 0,   "h": "normal",   "r": "low",     "v": 0,  "m": "restart_count", "u": "count"},
            {"s": 180, "h": "warning",  "r": "medium",  "v": 5,  "m": "restart_count", "u": "count"},
            {"s": 360, "h": "critical", "r": "high",    "v": 15, "m": "restart_count", "u": "count", "alert": True, "finding": True},
            {"s": 540, "h": "critical", "r": "critical", "v": 28, "m": "restart_count", "u": "count", "alert": True},
            {"s": 900, "h": "normal",   "r": "low",     "v": 0,  "m": "restart_count", "u": "count"},
        ],
    },
    "node_disk_pressure": {
        "name": "节点磁盘压力", "target_type": "KubernetesNode",
        "propagate_to": ["Pod"],
        "stages": [
            {"s": 0,    "h": "normal",   "r": "low",     "v": 55.0, "m": "disk_usage", "u": "percent"},
            {"s": 600,  "h": "warning",  "r": "medium",  "v": 88.0, "m": "disk_usage", "u": "percent", "alert": True},
            {"s": 1200, "h": "critical", "r": "high",    "v": 94.0, "m": "disk_usage", "u": "percent", "alert": True},
            {"s": 1800, "h": "critical", "r": "critical","v": 97.0, "m": "disk_usage", "u": "percent", "alert": True, "finding": True},
            {"s": 3600, "h": "warning",  "r": "medium",  "v": 75.0, "m": "disk_usage", "u": "percent"},
            {"s": 7200, "h": "normal",   "r": "low",     "v": 50.0, "m": "disk_usage", "u": "percent"},
        ],
    },
}


def inject(fault_type: str, target_id: str) -> FaultInjection | None:
    ft = FAULT_DEFS.get(fault_type)
    if not ft:
        return None

    fid = f"fault-{fault_type}-{target_id.replace(':', '-')}"
    now = datetime.now(timezone.utc)
    stages = []
    for i, s in enumerate(ft["stages"]):
        stages.append(FaultStage(
            sequence=i,
            offset_seconds=s["s"],
            health=s["h"],
            risk=s["r"],
            metric_name=s.get("m", ""),
            metric_value=s.get("v", 0.0),
            unit=s.get("u", "percent"),
            triggers_alert=s.get("alert", False),
            triggers_finding=s.get("finding", False),
        ))

    fault = FaultInjection(
        injection_id=fid,
        fault_type=fault_type,
        target_id=target_id,
        current_stage=0,
        total_stages=len(stages),
        status="injected",
        injected_at=now.isoformat(),
        stages=stages,
    )
    store.add_fault(fault)

    # Apply stage 0
    _apply_stage(fault, 0)

    return fault


def step(step_seconds: int = 60) -> int:
    updated = 0
    for fault in store.get_active_faults():
        ns = fault.current_stage + 1
        if ns >= fault.total_stages:
            fault.status = "resolved"
            # Reset target to normal
            _set_node_props(fault.target_id, health="normal", risk="low")
            _propagate(fault.target_id, "normal", "low", FAULT_DEFS.get(fault.fault_type, {}))
        else:
            fault.status = "escalating" if fault.stages[ns].triggers_alert else "propagating"
            fault.current_stage = ns
            _apply_stage(fault, ns)
            stg = fault.stages[ns]
            _propagate(fault.target_id, stg.health, stg.risk, FAULT_DEFS.get(fault.fault_type, {}))
        updated += 1
    return updated


def reset():
    for node in store.get_all_nodes():
        node.properties["health_status"] = "normal"
        node.properties["risk_level"] = "low"
    for edge in store.get_all_edges():
        edge.properties["health_status"] = "normal"
        edge.properties["risk_signal"] = ""
    store.clear_fault_metrics()
    store.clear_faults()


def _apply_stage(fault: FaultInjection, stage_idx: int):
    stg = fault.stages[stage_idx]
    _set_node_props(fault.target_id, health=stg.health, risk=stg.risk)

    # MetricSnapshot
    now = datetime.now(timezone.utc).isoformat()
    snap = MetricSnapshot(
        snapshot_id=f"fault_snap_{fault.injection_id}_{stage_idx}",
        resource_id=fault.target_id,
        metric_name=stg.metric_name,
        current_value=stg.metric_value,
        unit=stg.unit,
        fetched_at=now,
        warning_breached=stg.health in ("warning", "critical"),
        critical_breached=stg.health == "critical",
    )
    store.add_metric(snap)

    # Update metric property on node
    node = store.get_node(fault.target_id)
    if node:
        node.properties[stg.metric_name] = stg.metric_value


def _set_node_props(node_id: str, **props):
    node = store.get_node(node_id)
    if node:
        node.properties.update(props)


def _propagate(target_id: str, health: str, risk: str, ft: dict):
    for ptype in ft.get("propagate_to", []):
        for edge in store.get_all_edges():
            if edge.target_id == target_id and edge.relationship_type in ("CONTAINS", "DEPLOYED_AS", "USES", "DEPENDS_ON"):
                src = store.get_node(edge.source_id)
                if src and src.type == ptype:
                    src.properties["health_status"] = health
                    src.properties["risk_level"] = risk
