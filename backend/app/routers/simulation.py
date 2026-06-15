"""Fault Simulation API v2 — inject data-dimension faults"""
import json
from datetime import datetime, timedelta, timezone
from fastapi import APIRouter, Query
from pydantic import BaseModel
from app.db.neo4j_client import get_driver

router = APIRouter(prefix="/api/v1/simulation", tags=["Simulation"])

FAULT_TYPES = {
    "cpu_spike": {
        "name": "Pod CPU 飙升", "category": "resource", "target_type": "Pod",
        "metric": "cpu_usage", "unit": "percent", "alert_rule": "Pod CPU 使用率过高",
        "stages": [
            {"offset_s": 0,   "health": "normal",   "risk": "low",     "val": 45.2, "threshold": "normal"},
            {"offset_s": 180, "health": "warning",  "risk": "medium",  "val": 86.5, "threshold": "warning", "alert": True},
            {"offset_s": 360, "health": "critical", "risk": "high",    "val": 93.0, "threshold": "critical", "alert": True},
            {"offset_s": 600, "health": "critical", "risk": "high",    "val": 96.2, "threshold": "critical", "alert": True, "finding": True},
            {"offset_s": 900, "health": "warning",  "risk": "medium",  "val": 65.0, "threshold": "normal"},
            {"offset_s": 1200,"health": "normal",   "risk": "low",     "val": 48.0, "threshold": "normal"},
        ],
        "propagate_to": ["Deployment", "ApplicationComponent"],
    },
    "memory_leak": {
        "name": "内存泄漏", "category": "resource", "target_type": "Pod",
        "metric": "memory_usage", "unit": "percent", "alert_rule": "Pod 内存使用率过高",
        "stages": [
            {"offset_s": 0,   "health": "normal",   "risk": "low",     "val": 55.0, "threshold": "normal"},
            {"offset_s": 300, "health": "warning",  "risk": "medium",  "val": 85.0, "threshold": "warning", "alert": True},
            {"offset_s": 600, "health": "critical", "risk": "high",    "val": 94.5, "threshold": "critical", "alert": True},
            {"offset_s": 900, "health": "critical", "risk": "critical", "val": 98.0, "threshold": "critical", "alert": True, "finding": True},
            {"offset_s": 1200,"health": "critical", "risk": "high",    "val": 90.0, "threshold": "critical", "alert": True},
            {"offset_s": 1500,"health": "normal",   "risk": "low",     "val": 55.0, "threshold": "normal"},
        ],
        "propagate_to": ["Deployment"],
    },
    "pod_crashloop": {
        "name": "Pod CrashLoop", "category": "availability", "target_type": "Pod",
        "metric": "restart_count", "unit": "count", "alert_rule": "Pod 频繁重启",
        "stages": [
            {"offset_s": 0,   "health": "normal",   "risk": "low",     "val": 0,  "threshold": "normal"},
            {"offset_s": 180, "health": "warning",  "risk": "medium",  "val": 5,  "threshold": "warning"},
            {"offset_s": 360, "health": "critical", "risk": "high",    "val": 15, "threshold": "critical", "alert": True, "finding": True},
            {"offset_s": 540, "health": "critical", "risk": "critical", "val": 28, "threshold": "critical", "alert": True},
            {"offset_s": 900, "health": "normal",   "risk": "low",     "val": 0,  "threshold": "normal"},
        ],
        "propagate_to": ["Deployment", "ApplicationComponent"],
    },
    "node_disk_pressure": {
        "name": "节点磁盘压力", "category": "resource", "target_type": "KubernetesNode",
        "metric": "disk_usage", "unit": "percent", "alert_rule": "节点磁盘空间不足",
        "stages": [
            {"offset_s": 0,    "health": "normal",   "risk": "low",     "val": 55.0, "threshold": "normal"},
            {"offset_s": 600,  "health": "warning",  "risk": "medium",  "val": 88.0, "threshold": "warning", "alert": True},
            {"offset_s": 1200, "health": "critical", "risk": "high",    "val": 94.0, "threshold": "critical", "alert": True},
            {"offset_s": 1800, "health": "critical", "risk": "critical", "val": 97.0, "threshold": "critical", "alert": True, "finding": True},
            {"offset_s": 3600, "health": "warning",  "risk": "medium",  "val": 75.0, "threshold": "normal"},
            {"offset_s": 7200, "health": "normal",   "risk": "low",     "val": 50.0, "threshold": "normal"},
        ],
        "propagate_to": ["Pod"],
    },
}


class InjectRequest(BaseModel):
    fault_type: str
    target_id: str


@router.get("/types")
def list_types():
    return {"types": {k: {"name": v["name"], "category": v["category"], "target_type": v["target_type"], "metric": v["metric"], "stages": len(v["stages"])} for k, v in FAULT_TYPES.items()}}


@router.post("/inject")
def inject(inj: InjectRequest):
    ft = FAULT_TYPES.get(inj.fault_type)
    if not ft:
        return {"error": f"Unknown type", "available": list(FAULT_TYPES)}
    driver = get_driver()
    sid = f"fault-{inj.fault_type}-{inj.target_id.replace(':', '-')}"
    now = datetime.now(timezone.utc)
    with driver.session() as s:
        s.run("MERGE (fs:FaultScenario {scenario_id:$sid}) SET fs.name=$n, fs.fault_type=$ft, fs.target_resource_id=$tid, fs.status='injected', fs.current_stage=0, fs.total_stages=$ts, fs.injected_at=datetime($b), fs.updated_at=datetime(), fs.description=$d, fs.version='v1'",
              sid=sid, n=ft["name"], ft=inj.fault_type, tid=inj.target_id, ts=len(ft["stages"]), b=now.isoformat(), d=f"{ft['name']} on {inj.target_id}")
        s.run("MATCH (fs:FaultScenario {scenario_id:$sid}) MATCH (r:ResourceInstance {node_id:$tid}) MERGE (fs)-[:AFFECTS]->(r)", sid=sid, tid=inj.target_id)
        _apply_stage(s, sid, inj.target_id, ft, 0, now, 0)
    return {"status": "ok", "scenario_id": sid, "stages": len(ft["stages"])}


@router.post("/step")
def step(seconds: int = Query(default=60, ge=10, le=3600)):
    driver = get_driver()
    now = datetime.now(timezone.utc)
    with driver.session() as s:
        recs = s.run("MATCH (fs:FaultScenario) WHERE fs.status IN ['injected','escalating','propagating'] RETURN fs.scenario_id AS sid, fs.fault_type AS ft, fs.target_resource_id AS tid, fs.current_stage AS stage, fs.injected_at AS base")
        updated = 0
        for r in recs:
            sid, ft_code, tid, stage = r["sid"], r["ft"], r["tid"], r["stage"]
            ft = FAULT_TYPES.get(ft_code)
            if not ft or stage >= len(ft["stages"]):
                continue
            ns = stage + 1
            base = r["base"]
            # base may be Neo4j DateTime — convert to Python datetime
            if base and hasattr(base, 'to_native'):
                base = base.to_native()
            # elif base is a string, parse it
            elif isinstance(base, str):
                base = datetime.fromisoformat(base)
            if ns >= len(ft["stages"]):
                s.run("MATCH (fs:FaultScenario {scenario_id:$sid}) SET fs.status='resolved', fs.current_stage=$st, fs.resolved_at=datetime()", sid=sid, st=ns)
                s.run("MATCH (r:ResourceInstance {node_id:$tid}) SET r.health_status='normal', r.risk_level='low'", tid=tid)
                _propagate(s, tid, "normal", "low", ft)
            else:
                stg = ft["stages"][ns]
                status = "escalating" if stg.get("alert") else "propagating"
                s.run("MATCH (fs:FaultScenario {scenario_id:$sid}) SET fs.status=$st, fs.current_stage=$ns, fs.updated_at=datetime()", sid=sid, st=status, ns=ns)
                _apply_stage(s, sid, tid, ft, ns, now, seconds)
                _propagate(s, tid, stg["health"], stg["risk"], ft)
            updated += 1
    return {"status": "ok", "seconds": seconds, "updated": updated}


@router.get("/status")
def status():
    from app.db.neo4j_client import run_query
    active = run_query("MATCH (fs:FaultScenario) WHERE fs.status<>'resolved' RETURN fs")
    resolved = run_query("MATCH (fs:FaultScenario {status:'resolved'}) RETURN fs")
    snaps = run_query("MATCH (ms:MetricSnapshot) WHERE ms.snapshot_id STARTS WITH 'fault_' RETURN count(ms) AS c")
    alerts = run_query("MATCH (ae:AlertEvent) WHERE ae.alert_event_id STARTS WITH 'fault_' RETURN count(ae) AS c")
    findings = run_query("MATCH (f:InspectionFinding) WHERE f.node_id STARTS WITH 'fault_' RETURN count(f) AS c")
    return {"active": len(active), "resolved": len(resolved), "metric_snapshots": snaps[0]["c"], "alert_events": alerts[0]["c"], "inspection_findings": findings[0]["c"]}


@router.post("/reset")
def reset():
    driver = get_driver()
    with driver.session() as s:
        s.run("MATCH (fs:FaultScenario) DETACH DELETE fs")
        s.run("MATCH (tl:FaultTimeline) DETACH DELETE tl")
        s.run("MATCH (ms:MetricSnapshot) WHERE ms.snapshot_id STARTS WITH 'fault_' DETACH DELETE ms")
        s.run("MATCH (ae:AlertEvent) WHERE ae.alert_event_id STARTS WITH 'fault_' DETACH DELETE ae")
        s.run("MATCH (f:InspectionFinding) WHERE f.node_id STARTS WITH 'fault_' DETACH DELETE f")
        s.run("MATCH (r:ResourceInstance) SET r.health_status='normal', r.risk_level='low'")
        s.run("MATCH ()-[rel:RELATES_TO]->() SET rel.health_status='normal', rel.risk_signal=''")
    return {"status": "ok"}


# ── Helpers ──

def _apply_stage(s, sid, tid, ft, stage_idx, now, offset_s):
    stg = ft["stages"][stage_idx]
    ts = (now + timedelta(seconds=offset_s)).isoformat()
    m = ft["metric"]; u = ft.get("unit", "percent")

    # Update node
    s.run("MATCH (r:ResourceInstance {node_id:$tid}) SET r.health_status=$h, r.risk_level=$r, r.updated_at=datetime()",
          tid=tid, h=stg["health"], r=stg["risk"])

    # MetricSnapshot
    snap_id = f"fault_snap_{sid}_{stage_idx}"
    s.run("MERGE (ms:MetricSnapshot {snapshot_id:$sid}) SET ms.resource_id=$rid, ms.metric_name=$m, ms.current_value=$v, ms.unit=$u, ms.fetched_at=datetime($ts), ms.ttl_seconds=600, ms.is_stale='false', ms.warning_breached=$wb, ms.critical_breached=$cb, ms.version='v1'",
          sid=snap_id, rid=tid, m=m, v=stg["val"], u=u, ts=ts,
          wb=str(stg["threshold"] in ("warning","critical")).lower(),
          cb=str(stg["threshold"] == "critical").lower())
    s.run("MATCH (ms:MetricSnapshot {snapshot_id:$sid}) MATCH (r:ResourceInstance {node_id:$rid}) MERGE (ms)-[:MEASURES]->(r)", sid=snap_id, rid=tid)

    # AlertEvent
    if stg.get("alert"):
        aid = f"fault_alert_{sid}_{stage_idx}"
        s.run("MERGE (ae:AlertEvent:ResourceInstance {alert_event_id:$aid}) SET ae.alert_name=$an, ae.severity=$sev, ae.status='firing', ae.fired_at=datetime($ts), ae.summary=$sum, ae.description=$desc, ae.resource_ref=$rid, ae.affected_labels=$labels, ae.version='v1'",
              aid=aid, an=ft.get("alert_rule",ft["name"]),
              sev="critical" if stg["threshold"]=="critical" else "warning",
              ts=ts, sum=f"{ft['name']}: {m}={stg['val']}{u}",
              desc=f"{ft['name']} on {tid}. {m}={stg['val']}{u}.",
              rid=tid, labels=json.dumps({"resource_id":tid,"metric":m,"value":stg["val"]}))
        s.run("MATCH (ae:AlertEvent {alert_event_id:$aid}) MATCH (r:ResourceInstance {node_id:$rid}) MERGE (ae)-[:FIRED_ON]->(r)", aid=aid, rid=tid)

    # InspectionFinding
    if stg.get("finding"):
        fid = f"fault_finding_{sid}_{stage_idx}"
        s.run("MERGE (f:InspectionFinding:ResourceInstance {node_id:$fid}) SET f.label='InspectionFinding', f.name=$n, f.severity=$sev, f.status='open', f.detected_at=datetime($ts), f.description=$desc, f.attrs_json=$attrs, f.version='v1'",
              fid=fid, n=f"{ft['name']} - 发现",
              sev="critical" if stg["threshold"]=="critical" else "warning",
              ts=ts, desc=f"{ft['name']}: {m}={stg['val']}{u} 超过阈值",
              attrs=json.dumps({"metric":m,"value":stg["val"],"unit":u}))


def _propagate(s, tid, health, risk, ft):
    for ptype in ft.get("propagate_to", []):
        s.run("MATCH (r:ResourceInstance {node_id:$tid})<-[rel:RELATES_TO]-(up:ResourceInstance) WHERE up.label=$ptype AND rel.relationship_type IN ['CONTAINS','DEPLOYED_AS','USES','DEPENDS_ON'] SET up.health_status=$h, up.risk_level=$r", tid=tid, ptype=ptype, h=health, r=risk)
