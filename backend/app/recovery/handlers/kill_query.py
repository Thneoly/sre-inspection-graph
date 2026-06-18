"""kill_query 执行器 — Sprint 2 mock 实现。

真实环境会调:
    MySQL: KILL QUERY <query_id>
    或通过 PyMySQL / SQLAlchemy 连接发送

Sprint 2 mock:仅记录在 DSS MySQL 节点的 properties.killed_queries 列表里。
"""

from datetime import datetime, timezone
from app.datasource.store import store


def execute(target_id: str, params: dict, context: dict) -> dict:
    """终止指定 MySQL 查询。"""
    query_id = params.get("query_id")
    if not query_id:
        return {"success": False, "error": "query_id is required"}

    target = store.get_node(target_id)
    if not target:
        return {"success": False, "error": f"target not found: {target_id}"}
    if target.type != "MySQL":
        return {"success": False, "error": f"target is {target.type}, not MySQL"}

    min_duration = params.get("min_duration_seconds", 30)
    now = datetime.now(timezone.utc).isoformat()

    # 维护一个 killed queries 历史(最近 50 条)
    killed = list(target.properties.get("killed_queries", []))
    killed.append({
        "query_id": query_id,
        "killed_at": now,
        "min_duration_threshold": min_duration,
        "execution_id": context.get("execution_id", ""),
    })
    if len(killed) > 50:
        killed = killed[-50:]

    store.update_node_props(target_id,
                            killed_queries=killed,
                            last_kill_at=now)

    return {
        "success": True,
        "query_id": query_id,
        "completed_at": now,
        "note": f"Query {query_id} terminated on {target.name} (mock execution)",
    }
