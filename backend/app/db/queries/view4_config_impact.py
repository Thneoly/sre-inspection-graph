"""View 4: 配置影响视图 — Secret/ConfigMap → Deployment → Application"""

CONFIG_IMPACT_QUERY = """
MATCH path = (config:ResourceInstance {node_id: $resource_id})
  <-[r:RELATES_TO*1..$depth]-(related:ResourceInstance)
WHERE config.label IN ['Secret', 'ConfigMap']
AND ALL(rel IN r WHERE rel.relationship_type IN [
  'USES','CONTAINS','DEPLOYED_AS','BELONGS_TO',
  'RUNS','SCHEDULED_ON','EXPOSES','ROUTES_TO'
])
RETURN path
LIMIT $limit
"""


def get_config_impact(resource_id: str, depth: int = 4, limit: int = 200) -> tuple[str, dict]:
    query = CONFIG_IMPACT_QUERY.replace("$depth", str(depth))
    return query, {
        "resource_id": resource_id,
        "limit": limit,
    }
