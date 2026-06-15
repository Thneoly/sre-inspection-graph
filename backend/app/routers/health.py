"""健康检查 API"""
from fastapi import APIRouter
from app.db.neo4j_client import check_connection

router = APIRouter(tags=["Health"])


@router.get("/health")
def health():
    neo4j_ok = check_connection()
    return {
        "status": "ok" if neo4j_ok else "degraded",
        "neo4j": "connected" if neo4j_ok else "disconnected",
        "version": "1.0.0",
    }
