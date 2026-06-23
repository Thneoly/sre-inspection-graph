"""View 2: 访问链路视图 — Ingress → Service → Pod → Container"""

ACCESS_LINK_QUERY = """
MATCH path = (ing:ResourceInstance {label: 'Ingress'})
  -[r:RELATES_TO*1..$depth]-(related:ResourceInstance)
WHERE EXISTS {
  MATCH (ing)-[:RELATES_TO*1..5]-(app:ResourceInstance {label: 'Application', node_id: $app_node_id})
}
AND ALL(rel IN r WHERE rel.relationship_type IN [
  'ROUTES_TO','EXPOSES','DEPLOYED_IN','BELONGS_TO',
  'CONTAINS','DEPLOYED_AS','RUNS','SCHEDULED_ON'
])
RETURN path
LIMIT $limit
"""


def get_access_link(app_code: str, depth: int = 5, limit: int = 200) -> tuple[str, dict]:
    query = ACCESS_LINK_QUERY.replace("$depth", str(depth))
    return query, {
        "app_node_id": f"app:{app_code}",
        "limit": limit,
    }
