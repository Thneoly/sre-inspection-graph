"""应用拓扑视图 API"""
from fastapi import APIRouter, Query

from app.db.neo4j_client import run_query
from app.db.queries.view1_topology import get_app_topology
from app.services.graph_service import format_graph_response
from app.models.graph import GraphResponse

router = APIRouter(tags=["Topology"])


@router.get("/topology/app/{app_code}", response_model=GraphResponse)
def app_topology(
    app_code: str,
    depth: int = Query(default=5, ge=1, le=10, description="遍历深度"),
):
    cypher, params = get_app_topology(app_code, depth)
    records = run_query(cypher, params)
    return format_graph_response(records)
