"""配置影响视图 API"""
from fastapi import APIRouter

from app.db.neo4j_client import run_query
from app.db.queries.view4_config_impact import get_config_impact
from app.services.graph_service import format_graph_response
from app.models.graph import GraphResponse

router = APIRouter(tags=["Config Impact"])


@router.get("/config-impact/{resource_id:path}", response_model=GraphResponse)
def config_impact(resource_id: str):
    cypher, params = get_config_impact(resource_id)
    records = run_query(cypher, params)
    return format_graph_response(records)
