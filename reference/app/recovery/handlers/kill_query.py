"""kill_query 执行器 — Phase 2 真实 MySQL + mock 双模式。

real:`MySQLClient.kill(conn_id)` 发 `KILL <conn_id>`。成功后追加 DSS killed_queries 历史。
mock(默认):仅记 DSS。
"""

from datetime import datetime, timezone

from app.config import settings
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

    if settings.recovery_handler_mode == "real":
        return _execute_real(target_id, target, query_id, min_duration, context)
    return _execute_mock(target_id, target, query_id, min_duration, context)


def _append_history(target_id, target, query_id, min_duration, context):
    now = datetime.now(timezone.utc).isoformat()
    killed = list(target.properties.get("killed_queries", []))
    killed.append({
        "query_id": query_id,
        "killed_at": now,
        "min_duration_threshold": min_duration,
        "execution_id": context.get("execution_id", ""),
    })
    if len(killed) > 50:
        killed = killed[-50:]
    store.update_node_props(target_id, killed_queries=killed, last_kill_at=now)
    return now


def _execute_mock(target_id, target, query_id, min_duration, context) -> dict:
    now = _append_history(target_id, target, query_id, min_duration, context)
    return {
        "success": True,
        "query_id": query_id,
        "completed_at": now,
        "note": f"Query {query_id} terminated on {target.name} (mock execution)",
    }


def _execute_real(target_id, target, query_id, min_duration, context) -> dict:
    from app.recovery.clients.mysql_client import MySQLClient

    try:
        client = MySQLClient.from_node(target)
    except ValueError as e:
        return {"success": False, "error": str(e)}

    try:
        client.connect()
        # query_id 即 MySQL connection/process Id
        client.kill(int(query_id))
    except Exception as e:  # noqa: BLE001
        return {"success": False, "error": f"mysql kill failed: {type(e).__name__}: {e}"}
    finally:
        client.close()

    now = _append_history(target_id, target, query_id, min_duration, context)
    return {
        "success": True,
        "query_id": query_id,
        "completed_at": now,
        "host": client.host,
        "note": f"Query {query_id} terminated on {target.name} (real mysql execution)",
    }
