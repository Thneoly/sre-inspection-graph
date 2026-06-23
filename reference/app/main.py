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
    connectors,
    report,
    webhook,
    alert,
)

# Auto-init DSS on startup
from app.datasource.loader import load_baseline
from app.datasource.connectors.sync_orchestrator import (
    init_connectors,
    start_all_connectors,
    stop_all_connectors,
)
# PRD-003 Sprint 2 — 报告订阅调度器 + Neo4j hydrate
from app.reports.persistence import load_subscriptions_from_neo4j
from app.reports.scheduler import report_scheduler

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
app.include_router(connectors.router)
app.include_router(report.router)
app.include_router(webhook.router)
app.include_router(alert.router)


@app.on_event("startup")
async def startup():
    try:
        load_baseline()
    except Exception as e:
        print(f"DSS init warning: {e}")
    # PRD-004 — 注册 + 启动数据源 connectors(K8s / Prom / Jaeger / flagd)
    try:
        init_connectors()
        await start_all_connectors()
    except Exception as e:
        print(f"connectors startup warning: {e}")
    # PRD-004 Phase 2 — 从 health_rules 阈值生成 AlertRule 到 DSS
    try:
        from app.datasource.connectors.health_rules import sync_alert_rules_to_store
        n = sync_alert_rules_to_store()
        print(f"alert rules synced: {n}")
    except Exception as e:
        print(f"alert rules sync warning: {e}")
    # PRD-003 Sprint 2 — 报告订阅:hydrate + 启动调度器
    try:
        loaded = load_subscriptions_from_neo4j()
        if loaded:
            print(f"report subscriptions hydrated: {loaded}")
        report_scheduler.start()
        report_scheduler.reload_all()
    except Exception as e:
        print(f"report scheduler startup warning: {e}")


@app.on_event("shutdown")
async def shutdown():
    try:
        await stop_all_connectors()
    except Exception as e:
        print(f"connectors shutdown warning: {e}")
    try:
        report_scheduler.stop()
    except Exception as e:
        print(f"report scheduler shutdown warning: {e}")


@app.get("/")
async def root():
    return {
        "service": "SRE Inspection Graph API",
        "version": "1.0.0",
        "docs": "/docs",
    }
