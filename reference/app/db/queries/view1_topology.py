"""View 1: 应用拓扑视图 — Application → Pod → Container"""

TOPOLOGY_QUERY = """
MATCH path = (app:ResourceInstance {label: 'Application', node_id: $app_node_id})
  -[r:RELATES_TO*1..$depth]-(related:ResourceInstance)
WHERE ALL(rel IN r WHERE rel.relationship_type IN [
  'CONTAINS','DEPLOYED_AS','DEPLOYED_IN','BELONGS_TO',
  'EXPOSES','ROUTES_TO','USES','STORED_IN',
  'MONITORS','VISUALIZES','RUNS','SCHEDULED_ON',
  'DEPENDS_ON','REGISTERS_IN'
])
RETURN path
LIMIT $limit
"""


def get_app_topology(app_code: str, depth: int = 5, limit: int = 200) -> tuple[str, dict]:
    query = TOPOLOGY_QUERY.replace("$depth", str(depth))
    return query, {
        "app_node_id": f"app:{app_code}",
        "limit": limit,
    }
