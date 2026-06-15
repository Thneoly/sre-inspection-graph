"""故障注入引擎 — 通过 DSS 注入故障数据。包含目标校验和多节点影响。"""
from datetime import datetime, timezone
from app.datasource.models import FaultInjection, FaultStage, MetricSnapshot
from app.datasource.store import store

FAULT_DEFS = {
    "cpu_spike": {
        "name": "Pod CPU 飙升", "target_type": "Pod",
        "propagate_to": ["Deployment", "ApplicationComponent"],
        "blast_radius": {"edge": "CONTAINS", "direction": "reverse", "target_type": "Pod", "max": 3},
        "blast_propagate_to": ["Deployment", "ApplicationComponent", "Application"],
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
        "blast_radius": {"edge": "CONTAINS", "direction": "reverse", "target_type": "Pod", "max": 2},
        "blast_propagate_to": ["Deployment"],
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
        "blast_radius": {"edge": "CONTAINS", "direction": "reverse", "target_type": "Pod", "max": 3},
        "blast_propagate_to": ["Deployment", "ApplicationComponent", "Application"],
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
        "blast_radius": {"edge": "SCHEDULED_ON", "direction": "reverse", "target_type": "Pod", "max": 10},
        # Multi-hop: blast nodes → their upstream (Pod→Deployment→Component→App)
        "blast_propagate_to": ["Deployment", "ApplicationComponent", "Application"],
        "stages": [
            {"s": 0,    "h": "normal",   "r": "low",     "v": 55.0, "m": "disk_usage", "u": "percent"},
            {"s": 600,  "h": "warning",  "r": "medium",  "v": 88.0, "m": "disk_usage", "u": "percent", "alert": True},
            {"s": 1200, "h": "critical", "r": "high",    "v": 94.0, "m": "disk_usage", "u": "percent", "alert": True},
            {"s": 1800, "h": "critical", "r": "critical","v": 97.0, "m": "disk_usage", "u": "percent", "alert": True, "finding": True},
            {"s": 3600, "h": "warning",  "r": "medium",  "v": 75.0, "m": "disk_usage", "u": "percent"},
            {"s": 7200, "h": "normal",   "r": "low",     "v": 50.0, "m": "disk_usage", "u": "percent"},
        ],
    },
    "redis_unavailable": {
        "name": "Redis 不可达", "target_type": "Redis",
        "propagate_to": ["Deployment", "ApplicationComponent"],
        "blast_radius": {"edge": "USES", "direction": "reverse", "target_type": "Deployment", "max": 5},
        "blast_propagate_to": ["ApplicationComponent", "Application"],
        "stages": [
            {"s": 0,   "h": "critical", "r": "high",    "v": 0.95, "m": "error_rate", "u": "fraction", "alert": True},
            {"s": 120, "h": "critical", "r": "critical", "v": 1.0,  "m": "error_rate", "u": "fraction", "alert": True, "finding": True},
            {"s": 600, "h": "warning",  "r": "medium",  "v": 0.15, "m": "error_rate", "u": "fraction"},
            {"s": 1200,"h": "normal",   "r": "low",     "v": 0.001,"m": "error_rate", "u": "fraction"},
        ],
    },
    "mysql_slow_query": {
        "name": "MySQL 慢查询", "target_type": "MySQL",
        "propagate_to": ["Deployment", "ApplicationComponent"],
        "blast_radius": {"edge": "USES", "direction": "reverse", "target_type": "Deployment", "max": 5},
        "blast_propagate_to": ["ApplicationComponent", "Application"],
        "stages": [
            {"s": 0,   "h": "warning",  "r": "medium",  "v": 45.0, "m": "qps", "u": "requests/s"},
            {"s": 300, "h": "critical", "r": "high",    "v": 12.0, "m": "qps", "u": "requests/s", "alert": True},
            {"s": 900, "h": "warning",  "r": "medium",  "v": 80.0, "m": "qps", "u": "requests/s"},
            {"s": 1800,"h": "normal",   "r": "low",     "v": 850.0,"m": "qps", "u": "requests/s"},
        ],
    },
}


def inject(fault_type: str, target_id: str) -> tuple[FaultInjection | None, str | None]:
    """注入故障。返回 (fault, error)。error 非空表示注入失败。"""
    ft = FAULT_DEFS.get(fault_type)
    if not ft:
        return None, f"未知故障类型: {fault_type}"

    # ── 目标校验：故障类型必须匹配目标节点类型 ──
    target = store.get_node(target_id)
    if not target:
        return None, f"目标节点不存在: {target_id}"
    if target.type != ft["target_type"]:
        return None, f"目标类型不匹配: 故障目标={ft['target_type']}, 实际={target.type} (节点={target_id})"

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

    # Apply stage 0 to target and blast radius
    _apply_stage(fault, 0)
    _apply_blast_radius(fault, 0)

    return fault, None


def step(step_seconds: int = 60) -> int:
    updated = 0
    for fault in store.get_active_faults():
        ns = fault.current_stage + 1
        ft = FAULT_DEFS.get(fault.fault_type, {})
        if ns >= fault.total_stages:
            fault.status = "resolved"
            _set_node_props(fault.target_id, health="normal", risk="low")
            _reset_blast_radius(fault)
            _propagate(fault.target_id, "normal", "low", ft)
        else:
            fault.status = "escalating" if fault.stages[ns].triggers_alert else "propagating"
            fault.current_stage = ns
            _apply_stage(fault, ns)
            _apply_blast_radius(fault, ns)
            stg = fault.stages[ns]
            _propagate(fault.target_id, stg.health, stg.risk, ft)
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

    node = store.get_node(fault.target_id)
    if node:
        node.properties[stg.metric_name] = stg.metric_value


def _apply_blast_radius(fault: FaultInjection, stage_idx: int):
    """Apply stage to all blast radius nodes AND cascade their upstream."""
    ft = FAULT_DEFS.get(fault.fault_type, {})
    br = ft.get("blast_radius")
    if not br:
        return
    stg = fault.stages[stage_idx]
    affected = _find_blast_targets(fault.target_id, br)
    for nid in affected:
        _set_node_props(nid, health=stg.health, risk=stg.risk)
        node = store.get_node(nid)
        if node:
            node.properties[stg.metric_name] = stg.metric_value
        # Cascade from each blast node to ITS upstream
        _cascade_upstream(nid, stg.health, stg.risk, ft.get("blast_propagate_to", []))


def _reset_blast_radius(fault: FaultInjection):
    """Reset blast radius nodes and their upstream."""
    ft = FAULT_DEFS.get(fault.fault_type, {})
    br = ft.get("blast_radius")
    if not br:
        return
    affected = _find_blast_targets(fault.target_id, br)
    for nid in affected:
        _set_node_props(nid, health="normal", risk="low")
        _cascade_upstream(nid, "normal", "low", ft.get("blast_propagate_to", []))


def _cascade_upstream(node_id: str, health: str, risk: str, chain: list[str]):
    """Propagate health/risk up the dependency chain from a node."""
    current = node_id
    for ptype in chain:
        found = False
        for edge in store.get_all_edges():
            # Look for edges where current is the target → walk upstream to source
            if edge.target_id == current and edge.relationship_type in (
                    "CONTAINS", "DEPLOYED_AS", "USES", "DEPENDS_ON", "SCHEDULED_ON", "BELONGS_TO", "ROUTES_TO", "EXPOSES"):
                src = store.get_node(edge.source_id)
                if src and src.type == ptype:
                    src.properties["health_status"] = health
                    src.properties["risk_level"] = risk
                    current = edge.source_id
                    found = True
                    break
        if not found:
            break


def _find_blast_targets(target_id: str, br: dict) -> set[str]:
    """Find nodes in the blast radius from the target."""
    result: set[str] = set()
    for edge in store.get_all_edges():
        if edge.source_id == target_id or edge.target_id == target_id:
            matched = False
            if edge.relationship_type == br["edge"]:
                if br["direction"] == "reverse":
                    # Target is the target of the edge → find source nodes
                    if edge.target_id == target_id:
                        matched = True
                else:
                    if edge.source_id == target_id:
                        matched = True
            if matched:
                other = edge.source_id if edge.target_id == target_id else edge.target_id
                other_node = store.get_node(other)
                if other_node and other_node.type == br["target_type"]:
                    result.add(other)
                    if len(result) >= br.get("max", 10):
                        break
    return result


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
