"""告警归并视图 API + 指标 + 巡检"""
from fastapi import APIRouter, Query

from app.db.neo4j_client import run_query
from app.db.queries.view6_alert_aggr import get_alert_aggregation
from app.services.graph_service import format_graph_response
from app.models.graph import GraphResponse
from app.models.inspection import InspectionRunsResponse, InspectionRunOut, InspectionFindingsResponse, InspectionFindingOut
from app.models.metrics import ResourceMetricsResponse
from app.services.metrics_service import format_metrics_from_snapshots

router = APIRouter(tags=["Alert Aggregation"])


@router.get("/alert-aggregation", response_model=GraphResponse)
def alert_aggregation(
    severity: str | None = Query(default=None, description="告警级别: critical/warning/info"),
):
    cypher, params = get_alert_aggregation(severity=severity)
    records = run_query(cypher, params)
    return format_graph_response(records)


@router.get("/metrics/{resource_id:path}", response_model=ResourceMetricsResponse)
def resource_metrics(resource_id: str):
    """获取资源的最新指标快照"""
    query = """
    MATCH (ms:MetricSnapshot {resource_id: $resource_id})
    RETURN ms.snapshot_id AS snapshot_id, ms.metric_name AS metric_name,
           ms.current_value AS current_value, ms.unit AS unit,
           ms.fetched_at AS fetched_at, ms.is_stale AS is_stale,
           ms.warning_breached AS warning_breached,
           ms.critical_breached AS critical_breached,
           ms.warning_threshold AS warning_threshold,
           ms.critical_threshold AS critical_threshold
    ORDER BY ms.fetched_at DESC
    """
    records = run_query(query, {"resource_id": resource_id})
    return ResourceMetricsResponse(
        resource_id=resource_id,
        metrics=format_metrics_from_snapshots(records),
    )


@router.get("/inspection/runs", response_model=InspectionRunsResponse)
def inspection_runs(
    status: str | None = Query(default=None),
    limit: int = Query(default=20),
    offset: int = Query(default=0),
):
    """巡检运行列表"""
    cypher = """
    MATCH (run:InspectionRun:ResourceInstance)
    WHERE $status IS NULL OR run.inspection_status = $status
    RETURN run
    ORDER BY run.last_inspected_at DESC
    SKIP $offset LIMIT $limit
    """
    count_cypher = """
    MATCH (run:InspectionRun:ResourceInstance)
    WHERE $status IS NULL OR run.inspection_status = $status
    RETURN count(run) AS total
    """
    records = run_query(cypher, {"status": status, "offset": offset, "limit": limit})
    count_records = run_query(count_cypher, {"status": status})
    total = count_records[0].get("total", 0) if count_records else 0

    runs = []
    for r in records:
        run_data = r.get("run", {})
        attrs = _parse_attrs(run_data.get("attrs_json", "{}"))
        runs.append(InspectionRunOut(
            id=run_data.get("node_id", ""),
            run_name=run_data.get("name", ""),
            run_type=attrs.get("run_type", ""),
            overall_status=attrs.get("overall_status", ""),
            started_at=attrs.get("started_at", ""),
            completed_at=attrs.get("completed_at", ""),
            total_rules=attrs.get("total_rules", 0),
            passed_rules=attrs.get("passed_rules", 0),
            failed_rules=attrs.get("failed_rules", 0),
            skipped_rules=attrs.get("skipped_rules", 0),
        ))
    return InspectionRunsResponse(runs=runs, total=total)


@router.get("/inspection/findings/{resource_id:path}", response_model=InspectionFindingsResponse)
def inspection_findings(resource_id: str):
    """获取资源关联的巡检发现"""
    query = """
    MATCH (f:InspectionFinding:ResourceInstance)-[r:RELATES_TO {relationship_type: 'AFFECTS'}]->(res:ResourceInstance {node_id: $resource_id})
    RETURN f
    """
    records = run_query(query, {"resource_id": resource_id})
    findings = []
    for rec in records:
        f = rec.get("f", {})
        attrs = _parse_attrs(f.get("attrs_json", "{}"))
        findings.append(InspectionFindingOut(
            id=f.get("node_id", ""),
            rule_name=attrs.get("rule_name", ""),
            severity=attrs.get("severity", ""),
            status=attrs.get("status", "open"),
            description=attrs.get("description", ""),
            detected_at=attrs.get("detected_at", ""),
            recommendation=attrs.get("recommendation", ""),
        ))
    return InspectionFindingsResponse(resource_id=resource_id, findings=findings)


@router.get("/resource/{node_id:path}", response_model=dict)
def resource_detail(node_id: str):
    """获取单个资源详情（含属性+指标+发现）"""
    node_query = """
    MATCH (n:ResourceInstance {node_id: $node_id})
    RETURN n
    """
    node_records = run_query(node_query, {"node_id": node_id})
    if not node_records:
        return {"error": "Resource not found"}

    node = node_records[0].get("n", {})
    props = {k: str(v) for k, v in dict(node).items() if not k.startswith("_")}

    # Metrics
    metrics_query = """
    MATCH (ms:MetricSnapshot {resource_id: $node_id})
    RETURN ms
    ORDER BY ms.fetched_at DESC
    """
    metrics_records = run_query(metrics_query, {"node_id": node_id})
    metrics = format_metrics_from_snapshots([r.get("ms", {}) for r in metrics_records])

    # Findings
    findings_query = """
    MATCH (f:InspectionFinding:ResourceInstance)-[r:RELATES_TO {relationship_type: 'AFFECTS'}]->(res:ResourceInstance {node_id: $node_id})
    RETURN f
    """
    findings_records = run_query(findings_query, {"node_id": node_id})
    findings = []
    for rec in findings_records:
        f = rec.get("f", {})
        attrs = _parse_attrs(f.get("attrs_json", "{}"))
        findings.append({
            "id": f.get("node_id"),
            "severity": attrs.get("severity"),
            "status": attrs.get("status"),
            "description": attrs.get("description"),
            "recommendation": attrs.get("recommendation"),
        })

    return {
        "node": {"id": props.get("node_id", ""), "label": props.get("label", ""), "type": props.get("label", ""), "properties": props},
        "metrics": metrics,
        "findings": findings,
    }


def _parse_attrs(attrs_json: str) -> dict:
    import json
    try:
        return json.loads(attrs_json)
    except (json.JSONDecodeError, TypeError):
        return {}
