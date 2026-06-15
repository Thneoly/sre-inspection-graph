"""View 6: 告警归并视图 — AlertEvent → Resource → Application"""

ALERT_AGGR_QUERY = """
MATCH (alert:ResourceInstance {label: 'AlertEvent'})
WHERE alert.lifecycle_status = 'active'
  AND ($severity IS NULL OR alert.health_status = $severity_health)
MATCH (alert)-[r_fired:RELATES_TO {relationship_type: 'FIRED_ON'}]->(resource:ResourceInstance)
OPTIONAL MATCH path = (resource)-[r_up:RELATES_TO*1..4]->(app:ResourceInstance {label: 'Application'})
WHERE ALL(rel IN r_up WHERE rel.relationship_type IN [
  'CONTAINS','DEPLOYED_AS','BELONGS_TO','SCHEDULED_ON','RUNS'
])
RETURN alert, resource, app, collect(path) AS paths
LIMIT $limit
"""

ALERT_AGGR_FILTERED_QUERY = """
MATCH (alert:ResourceInstance {label: 'AlertEvent'})
WHERE alert.lifecycle_status = 'active'
MATCH (alert)-[r_fired:RELATES_TO {relationship_type: 'FIRED_ON'}]->(resource:ResourceInstance)
OPTIONAL MATCH path = (resource)-[r_up:RELATES_TO*1..4]->(app:ResourceInstance {label: 'Application'})
WHERE ALL(rel IN r_up WHERE rel.relationship_type IN [
  'CONTAINS','DEPLOYED_AS','BELONGS_TO','SCHEDULED_ON','RUNS'
])
RETURN alert, resource, app, collect(path) AS paths
LIMIT $limit
"""


def get_alert_aggregation(severity: str | None = None,
                          limit: int = 200) -> tuple[str, dict]:
    if severity:
        severity_health = {"critical": "critical", "warning": "warning", "info": "normal"}.get(severity, "warning")
        return ALERT_AGGR_QUERY, {
            "severity": severity,
            "severity_health": severity_health,
            "limit": limit,
        }
    return ALERT_AGGR_FILTERED_QUERY, {"limit": limit}
