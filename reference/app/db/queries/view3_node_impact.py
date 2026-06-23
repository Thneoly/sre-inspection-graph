"""View 3: 节点影响视图 — Node → Pod → Deployment → Application"""

NODE_IMPACT_QUERY = """
MATCH path = (node:ResourceInstance {label: 'KubernetesNode', node_id: $node_id})
  <-[r:RELATES_TO*1..$depth]-(affected:ResourceInstance)
WHERE ALL(rel IN r WHERE rel.relationship_type IN [
  'SCHEDULED_ON','CONTAINS','DEPLOYED_AS','BELONGS_TO',
  'RUNS','CONTROLLED_BY','AFFECTS','FIRED_ON'
])
RETURN path
LIMIT $limit
"""


def get_node_impact(node_id: str, depth: int = 4, limit: int = 200) -> tuple[str, dict]:
    query = NODE_IMPACT_QUERY.replace("$depth", str(depth))
    return query, {
        "node_id": node_id,
        "limit": limit,
    }
