"""访问链路视图 API"""
from fastapi import APIRouter

from app.db.neo4j_client import run_query
from app.db.queries.view2_access_link import get_access_link
from app.services.graph_service import format_graph_response
from app.models.graph import GraphResponse

router = APIRouter(tags=["Access Link"])


@router.get("/access-link/{app_code}", response_model=GraphResponse)
def access_link(app_code: str):
    cypher, params = get_access_link(app_code)
    records = run_query(cypher, params)
    return format_graph_response(records)
