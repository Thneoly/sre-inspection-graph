"""节点影响视图 API"""
from fastapi import APIRouter, Query

from app.db.neo4j_client import run_query
from app.db.queries.view3_node_impact import get_node_impact
from app.services.graph_service import format_graph_response
from app.models.graph import GraphResponse

router = APIRouter(tags=["Node Impact"])


@router.get("/node-impact/{node_id:path}", response_model=GraphResponse)
def node_impact(
    node_id: str,
    depth: int = Query(default=4, ge=1, le=10, description="遍历深度"),
):
    cypher, params = get_node_impact(node_id, depth)
    records = run_query(cypher, params)
    return format_graph_response(records)
