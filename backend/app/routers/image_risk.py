"""镜像风险视图 API"""
from fastapi import APIRouter

from app.db.neo4j_client import run_query
from app.db.queries.view5_image_risk import get_image_risk
from app.services.graph_service import format_graph_response
from app.models.graph import GraphResponse

router = APIRouter(tags=["Image Risk"])


@router.get("/image-risk/{image_id:path}", response_model=GraphResponse)
def image_risk(image_id: str):
    cypher, params = get_image_risk(image_id)
    records = run_query(cypher, params)
    return format_graph_response(records)
