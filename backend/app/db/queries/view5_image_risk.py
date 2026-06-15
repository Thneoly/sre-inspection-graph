"""View 5: 镜像风险视图 — ContainerImage → Deployment → Application"""

IMAGE_RISK_QUERY = """
MATCH path = (image:ResourceInstance {label: 'ContainerImage', node_id: $image_id})
  <-[r:RELATES_TO*1..$depth]-(related:ResourceInstance)
WHERE ALL(rel IN r WHERE rel.relationship_type IN [
  'USES','CONTAINS','DEPLOYED_AS','BELONGS_TO',
  'RUNS','SCHEDULED_ON','STORED_IN'
])
RETURN path
LIMIT $limit
"""


def get_image_risk(image_id: str, depth: int = 4, limit: int = 200) -> tuple[str, dict]:
    query = IMAGE_RISK_QUERY.replace("$depth", str(depth))
    return query, {
        "image_id": image_id,
        "limit": limit,
    }
