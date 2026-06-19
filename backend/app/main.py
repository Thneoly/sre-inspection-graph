"""FastAPI 应用入口"""
from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware

from app.routers import (
    topology,
    access_link,
    node_impact,
    config_impact,
    image_risk,
    alert_aggregation,
    health,
    simulation,
    datasource,
    recovery,
    change_event,
)

# Auto-init DSS on startup
from app.datasource.loader import load_baseline

app = FastAPI(
    title="SRE Inspection Graph API",
    description="云原生巡检图谱平台 — 四层模型 API",
    version="1.0.0",
)

# CORS
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

# Register routers
app.include_router(topology.router, prefix="/api/v1")
app.include_router(access_link.router, prefix="/api/v1")
app.include_router(node_impact.router, prefix="/api/v1")
app.include_router(config_impact.router, prefix="/api/v1")
app.include_router(image_risk.router, prefix="/api/v1")
app.include_router(alert_aggregation.router, prefix="/api/v1")
app.include_router(health.router, prefix="/api/v1")
app.include_router(simulation.router)
app.include_router(datasource.router)
app.include_router(recovery.router)
app.include_router(change_event.router)


@app.on_event("startup")
def startup():
    try:
        load_baseline()
    except Exception as e:
        print(f"DSS init warning: {e}")


@app.get("/")
async def root():
    return {
        "service": "SRE Inspection Graph API",
        "version": "1.0.0",
        "docs": "/docs",
    }
