"""
Fault Simulation Engine v2 — data-dimension injection.

Each fault step generates REAL data in Neo4j:
  - MetricSnapshot records (with timestamps)
  - AlertEvent nodes (when thresholds breached)
  - InspectionFinding nodes (when detected)
  - Updated node properties (health_status, risk_level, metric values)

Usage (from backend dir):
  uv run python ../scripts/fault_simulation.py inject cpu_spike pod:cce-prod-01:order:order-api-6fd9c8b7c9-abcdf
  uv run python ../scripts/fault_simulation.py step 300
  uv run python ../scripts/fault_simulation.py status
  uv run python ../scripts/fault_simulation.py reset
"""

import sys, os, json
from datetime import datetime, timedelta, timezone

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', 'backend'))
from app.db.neo4j_client import get_driver

NOW = datetime.now(timezone.utc)

FAULT_TYPES = {
    "cpu_spike": {
        "name": "Pod CPU 飙升",
        "category": "resource",
        "target_type": "Pod",
        "metric": "cpu_usage",
        "unit": "percent",
        "alert_rule": "Pod CPU 使用率过高",
        "finding_rule": "rule-001",
        "stages": [
            {"offset_s": 0,   "health": "warning",  "risk": "medium",   "val": 45.2, "threshold": "normal"},
            {"offset_s": 180, "health": "warning",  "risk": "medium",   "val": 72.0, "threshold": "warning"},
            {"offset_s": 360, "health": "warning",  "risk": "medium",   "val": 86.5, "threshold": "warning", "alert": True},
            {"offset_s": 600, "health": "critical", "risk": "high",     "val": 93.0, "threshold": "critical", "alert": True},
            {"offset_s": 900, "health": "critical", "risk": "high",     "val": 96.2, "threshold": "critical", "alert": True, "finding": True},
            {"offset_s": 1200,"health": "critical", "risk": "high",     "val": 88.0, "threshold": "warning", "alert": True},
            {"offset_s": 1500,"health": "warning",  "risk": "medium",   "val": 65.0, "threshold": "normal"},
            {"offset_s": 1800,"health": "normal",   "risk": "low",      "val": 48.0, "threshold": "normal"},
        ],
        "propagate_to": ["Deployment", "ApplicationComponent"],
    },
    "memory_leak": {
        "name": "内存泄漏",
        "category": "resource",
        "target_type": "Pod",
        "metric": "memory_usage",
        "unit": "percent",
        "alert_rule": "Pod 内存使用率过高",
        "finding_rule": "rule-001",
        "stages": [
            {"offset_s": 0,   "health": "warning",  "risk": "medium",   "val": 70.0, "threshold": "normal"},
            {"offset_s": 300, "health": "warning",  "risk": "medium",   "val": 85.0, "threshold": "warning", "alert": True},
            {"offset_s": 600, "health": "critical", "risk": "high",     "val": 94.5, "threshold": "critical", "alert": True},
            {"offset_s": 900, "health": "critical", "risk": "critical", "val": 98.0, "threshold": "critical", "alert": True, "finding": True},
            {"offset_s": 1200,"health": "critical", "risk": "high",     "val": 90.0, "threshold": "critical", "alert": True},
            {"offset_s": 1500,"health": "normal",   "risk": "low",      "val": 55.0, "threshold": "normal"},
        ],
        "propagate_to": ["Deployment"],
    },
    "pod_crashloop": {
        "name": "Pod 频繁重启",
        "category": "availability",
        "target_type": "Pod",
        "metric": "restart_count",
        "unit": "count",
        "alert_rule": "Pod 频繁重启",
        "finding_rule": "rule-002",
        "stages": [
            {"offset_s": 0,   "health": "warning",  "risk": "medium",   "val": 3,  "threshold": "warning"},
            {"offset_s": 180, "health": "warning",  "risk": "medium",   "val": 8,  "threshold": "warning"},
            {"offset_s": 360, "health": "critical", "risk": "high",     "val": 15, "threshold": "critical", "alert": True, "finding": True},
            {"offset_s": 540, "health": "critical", "risk": "critical", "val": 28, "threshold": "critical", "alert": True},
            {"offset_s": 900, "health": "normal",   "risk": "low",      "val": 0,  "threshold": "normal"},
        ],
        "propagate_to": ["Deployment", "ApplicationComponent"],
    },
    "node_disk_pressure": {
        "name": "节点磁盘压力",
        "category": "resource",
        "target_type": "KubernetesNode",
        "metric": "disk_usage",
        "unit": "percent",
        "alert_rule": "节点磁盘空间不足",
        "finding_rule": "rule-008",
        "stages": [
            {"offset_s": 0,    "health": "warning",  "risk": "medium",   "val": 80.0, "threshold": "normal"},
            {"offset_s": 600,  "health": "warning",  "risk": "medium",   "val": 88.0, "threshold": "warning", "alert": True},
            {"offset_s": 1200, "health": "critical", "risk": "high",     "val": 94.0, "threshold": "critical", "alert": True},
            {"offset_s": 1800, "health": "critical", "risk": "critical", "val": 97.0, "threshold": "critical", "alert": True, "finding": True},
            {"offset_s": 3600, "health": "warning",  "risk": "medium",   "val": 78.0, "threshold": "normal"},
            {"offset_s": 7200, "health": "normal",   "risk": "low",      "val": 55.0, "threshold": "normal"},
        ],
        "propagate_to": ["Pod"],
    },
}

# Relation direction: how to find "upstream" from a target
# Pod ← Deployment, Deployment ← Component ← Application
# So from Pod, we walk: (up)-[:RELATES_TO]->(target) where relationship_type matches


def inject_fault(fault_type: str, target_id: str):
    ft = FAULT_TYPES.get(fault_type)
    if not ft:
        print(f"Unknown: {fault_type}. Available: {list(FAULT_TYPES)}")
        return

    driver = get_driver()
    scenario_id = f"fault-{fault_type}-{target_id.replace(':', '-')}"
    base_time = NOW

    with driver.session() as s:
        # Create scenario
        s.run("""
            MERGE (fs:FaultScenario {scenario_id: $sid})
            SET fs.name=$name, fs.fault_type=$ft, fs.target_resource_id=$tid,
                fs.status='injected', fs.current_stage=0, fs.total_stages=$ts,
                fs.injected_at=datetime($base), fs.updated_at=datetime(),
                fs.description=$desc, fs.version='v1'
        """, sid=scenario_id, name=ft["name"], ft=fault_type, tid=target_id,
             ts=len(ft["stages"]), base=base_time.isoformat(),
             desc=f"{ft['name']} on {target_id}")

        # Apply stage 0
        _apply_stage(s, scenario_id, target_id, ft, 0, base_time, 0)

        # Link scenario → target
        s.run("""
            MATCH (fs:FaultScenario {scenario_id:$sid})
            MATCH (r:ResourceInstance {node_id:$tid})
            MERGE (fs)-[:AFFECTS]->(r)
        """, sid=scenario_id, tid=target_id)

    print(f"Injected: {scenario_id}")


def step_faults(step_seconds: int = 60):
    driver = get_driver()
    sim_time = NOW + timedelta(seconds=step_seconds)
    base_time = NOW

    with driver.session() as s:
        result = s.run("""
            MATCH (fs:FaultScenario) WHERE fs.status IN ['injected','escalating','propagating']
            RETURN fs.scenario_id AS sid, fs.fault_type AS ft,
                   fs.target_resource_id AS tid, fs.current_stage AS stage,
                   fs.injected_at AS base
        """)

        updated = 0
        for rec in result:
            sid, ft_code, tid, stage = rec["sid"], rec["ft"], rec["tid"], rec["stage"]
            ft = FAULT_TYPES.get(ft_code)
            if not ft or stage >= len(ft["stages"]):
                continue

            base = rec["base"]  # actual injection time
            next_stage = stage + 1

            if next_stage >= len(ft["stages"]):
                s.run("MATCH (fs:FaultScenario {scenario_id:$sid}) SET fs.status='resolved', fs.current_stage=$st, fs.resolved_at=datetime()",
                      sid=sid, st=next_stage)
                s.run("MATCH (r:ResourceInstance {node_id:$tid}) SET r.health_status='normal', r.risk_level='low'", tid=tid)
                _propagate(s, tid, "normal", "low", ft)
                print(f"  Resolved: {sid}")
            else:
                stg = ft["stages"][next_stage]
                status = "escalating" if stg.get("alert") else "propagating"
                s.run("MATCH (fs:FaultScenario {scenario_id:$sid}) SET fs.status=$status, fs.current_stage=$st, fs.updated_at=datetime()",
                      sid=sid, status=status, st=next_stage)
                _apply_stage(s, sid, tid, ft, next_stage, base_time, step_seconds)
                _propagate(s, tid, stg["health"], stg["risk"], ft)
                print(f"  Step {sid} → stage {next_stage}: {stg['health']}/{stg['risk']} val={stg['val']}")
            updated += 1

    print(f"Advanced {step_seconds}s | Updated {updated} faults" if updated else "No active faults")


def _apply_stage(s, scenario_id, target_id, ft, stage_idx, now, offset_s):
    """Create MetricSnapshot, AlertEvent, InspectionFinding for a stage."""
    stg = ft["stages"][stage_idx]
    stage_time = now + timedelta(seconds=offset_s)
    metric_name = ft["metric"]
    unit = ft.get("unit", "percent")

    # 1. Update node health/risk + metric value
    s.run("""
        MATCH (r:ResourceInstance {node_id:$tid})
        SET r.health_status=$h, r.risk_level=$r, r.updated_at=datetime()
    """, tid=target_id, h=stg["health"], r=stg["risk"])

    # Also update the metric property on the node
    try:
        s.run(f"MATCH (r:ResourceInstance {{node_id:$tid}}) SET r.{metric_name}_percent = $v", tid=target_id, v=stg["val"])
    except Exception:
        pass  # property might not exist

    # 2. Create MetricSnapshot
    snap_id = f"fault_snap_{scenario_id}_{stage_idx}"
    s.run("""
        MERGE (ms:MetricSnapshot {snapshot_id: $sid})
        SET ms.resource_id=$rid, ms.metric_name=$m, ms.current_value=$v,
            ms.unit=$u, ms.fetched_at=datetime($ts), ms.ttl_seconds=600,
            ms.is_stale='false',
            ms.warning_breached=$wb, ms.critical_breached=$cb,
            ms.version='v1', ms.updated_at=datetime()
    """, sid=snap_id, rid=target_id, m=metric_name, v=stg["val"], u=unit,
         ts=stage_time.isoformat(),
         wb=str(stg["threshold"] in ("warning","critical")).lower(),
         cb=str(stg["threshold"] == "critical").lower())

    # Link snapshot → resource
    s.run("""
        MATCH (ms:MetricSnapshot {snapshot_id:$sid})
        MATCH (r:ResourceInstance {node_id:$rid})
        MERGE (ms)-[:MEASURES]->(r)
    """, sid=snap_id, rid=target_id)

    # 3. Create AlertEvent if threshold breached
    if stg.get("alert"):
        alert_id = f"fault_alert_{scenario_id}_{stage_idx}"
        s.run("""
            MERGE (ae:AlertEvent {alert_event_id: $aid})
            SET ae.alert_name=$an, ae.severity=$sev, ae.status='firing',
                ae.fired_at=datetime($ts), ae.summary=$summary,
                ae.description=$desc, ae.resource_ref=$rid,
                ae.affected_labels=$labels,
                ae.version='v1', ae.updated_at=datetime()
        """, aid=alert_id, an=ft.get("alert_rule", ft["name"]),
             sev="critical" if stg["threshold"] == "critical" else "warning",
             ts=stage_time.isoformat(),
             summary=f"{ft['name']}: {metric_name}={stg['val']}{unit}",
             desc=f"{ft['name']} detected on {target_id}. Current {metric_name}: {stg['val']}{unit}.",
             rid=target_id,
             labels=json.dumps({"resource_id": target_id, "metric": metric_name, "value": stg["val"]}))

        s.run("""
            MATCH (ae:AlertEvent {alert_event_id:$aid})
            MATCH (r:ResourceInstance {node_id:$rid})
            MERGE (ae)-[:FIRED_ON]->(r)
        """, aid=alert_id, rid=target_id)

        # Mark alert as ResourceInstance too
        s.run("MATCH (ae:AlertEvent {alert_event_id:$aid}) SET ae:ResourceInstance", aid=alert_id)

    # 4. Create InspectionFinding if finding flag is set
    if stg.get("finding"):
        finding_id = f"fault_finding_{scenario_id}_{stage_idx}"
        rule_id = ft.get("finding_rule", "rule-001")
        s.run("""
            MERGE (f:InspectionFinding {node_id: $fid})
            SET f:ResourceInstance,
                f.label='InspectionFinding', f.name=$name,
                f.severity=$sev, f.status='open',
                f.affected_resource_id=$rid, f.detected_at=datetime($ts),
                f.description=$desc, f.recommendation=$rec,
                f.attrs_json=$attrs,
                f.version='v1', f.updated_at=datetime()
        """, fid=finding_id, name=f"{ft['name']} - 发现",
             sev="critical" if stg["threshold"] == "critical" else "warning",
             rid=target_id, ts=stage_time.isoformat(),
             desc=f"{ft['name']}: {metric_name}={stg['val']}{unit} 超过阈值",
             rec="检查资源使用情况，考虑扩容或限流",
             attrs=json.dumps({"rule_id": rule_id, "metric": metric_name, "value": stg["val"], "unit": unit}))


def _propagate(s, target_id, health, risk, ft):
    """Propagate health status to upstream nodes."""
    for ptype in ft.get("propagate_to", []):
        s.run("""
            MATCH (r:ResourceInstance {node_id:$tid})<-[rel:RELATES_TO]-(up:ResourceInstance)
            WHERE up.label=$ptype AND rel.relationship_type IN ['CONTAINS','DEPLOYED_AS','USES','DEPENDS_ON']
            SET up.health_status=$h, up.risk_level=$r, up.updated_at=datetime()
        """, tid=target_id, ptype=ptype, h=health, r=risk)


def reset_simulation():
    driver = get_driver()
    with driver.session() as s:
        s.run("MATCH (fs:FaultScenario) DETACH DELETE fs")
        s.run("MATCH (tl:FaultTimeline) DETACH DELETE tl")
        s.run("""
            MATCH (ms:MetricSnapshot) WHERE ms.snapshot_id STARTS WITH 'fault_'
            DETACH DELETE ms
        """)
        s.run("""
            MATCH (ae:AlertEvent) WHERE ae.alert_event_id STARTS WITH 'fault_'
            DETACH DELETE ae
        """)
        s.run("""
            MATCH (f:InspectionFinding) WHERE f.node_id STARTS WITH 'fault_'
            DETACH DELETE f
        """)
        s.run("MATCH (r:ResourceInstance) SET r.health_status='normal', r.risk_level='low'")
        s.run("MATCH ()-[r:RELATES_TO]->() SET r.health_status='normal', r.risk_signal=''")
    print("All faults and generated data cleared. Nodes restored to normal.")


def show_status():
    driver = get_driver()
    with driver.session() as s:
        active = list(s.run("MATCH (fs:FaultScenario) WHERE fs.status<>'resolved' RETURN fs"))
        resolved = list(s.run("MATCH (fs:FaultScenario {status:'resolved'}) RETURN fs"))
        snaps = list(s.run("MATCH (ms:MetricSnapshot) WHERE ms.snapshot_id STARTS WITH 'fault_' RETURN count(ms) AS c"))
        alerts = list(s.run("MATCH (ae:AlertEvent) WHERE ae.alert_event_id STARTS WITH 'fault_' RETURN count(ae) AS c"))
        findings = list(s.run("MATCH (f:InspectionFinding) WHERE f.node_id STARTS WITH 'fault_' RETURN count(f) AS c"))

    print(f"Active: {len(active)} | Resolved: {len(resolved)}")
    print(f"Generated data → MetricSnapshots: {snaps[0]['c']}, Alerts: {alerts[0]['c']}, Findings: {findings[0]['c']}")
    for r in active[:5]:
        fs = r["fs"]
        print(f"  [{fs.get('status')}] {fs.get('name')} → {fs.get('target_resource_id')} (stage {fs.get('current_stage')}/{fs.get('total_stages')})")


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "status"
    if cmd == "inject" and len(sys.argv) >= 4:
        inject_fault(sys.argv[2], sys.argv[3])
    elif cmd == "step":
        seconds = int(sys.argv[2]) if len(sys.argv) > 2 else 60
        step_faults(seconds)
    elif cmd == "status":
        show_status()
    elif cmd == "reset":
        reset_simulation()
    else:
        print("Usage: inject <type> <target> | step [seconds] | status | reset")
        print(f"Types: {list(FAULT_TYPES.keys())}")
