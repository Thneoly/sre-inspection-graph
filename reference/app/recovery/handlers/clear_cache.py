"""clear_cache 执行器 — Phase 2 真实 Redis + mock 双模式。

real:`RedisClient.flush_all / flush_db / delete_pattern`。成功后更新 DSS flush_count。
mock(默认):仅记 DSS。
"""

from datetime import datetime, timezone

from app.config import settings
from app.datasource.store import store


def execute(target_id: str, params: dict, context: dict) -> dict:
    """清空 Redis 缓存。"""
    target = store.get_node(target_id)
    if not target:
        return {"success": False, "error": f"target not found: {target_id}"}
    if target.type != "Redis":
        return {"success": False, "error": f"target is {target.type}, not Redis"}

    scope = params.get("scope", "pattern")
    if scope not in ("all", "db", "pattern"):
        return {"success": False, "error": f"invalid scope: {scope}"}

    db_index = params.get("db_index", 0)
    if not isinstance(db_index, int) or db_index < 0 or db_index > 15:
        return {"success": False, "error": f"db_index out of range: {db_index}"}

    key_pattern = params.get("key_pattern", "")
    if scope == "pattern" and not key_pattern:
        return {"success": False, "error": "key_pattern required when scope=pattern"}

    old_flush_count = int(target.properties.get("flush_count", 0))
    new_flush_count = old_flush_count + 1

    if settings.recovery_handler_mode == "real":
        return _execute_real(target_id, target, scope, db_index, key_pattern,
                             new_flush_count, context)
    return _execute_mock(target_id, target, scope, db_index, key_pattern,
                         new_flush_count, context)


def _apply_dss(target_id, scope, new_flush_count, context):
    now = datetime.now(timezone.utc).isoformat()
    store.update_node_props(
        target_id,
        flush_count=new_flush_count,
        cleared_at=now,
        cleared_by_execution=context.get("execution_id", ""),
        last_clear_scope=scope,
    )
    return now


def _note(target, scope, db_index, key_pattern, mode):
    if scope == "all":
        return f"Redis {target.name} FLUSHALL executed ({mode})"
    if scope == "db":
        return f"Redis {target.name} FLUSHDB on db={db_index} ({mode})"
    return f"Redis {target.name} keys matching '{key_pattern}' deleted ({mode})"


def _execute_mock(target_id, target, scope, db_index, key_pattern, new_flush_count, context) -> dict:
    now = _apply_dss(target_id, scope, new_flush_count, context)
    return {
        "success": True,
        "completed_at": now,
        "scope": scope,
        "db_index": db_index,
        "key_pattern": key_pattern,
        "flush_count": new_flush_count,
        "note": _note(target, scope, db_index, key_pattern, "mock"),
    }


def _execute_real(target_id, target, scope, db_index, key_pattern, new_flush_count, context) -> dict:
    from app.recovery.clients.redis_client import RedisClient

    try:
        client = RedisClient.from_node(target)
    except ValueError as e:
        return {"success": False, "error": str(e)}

    deleted: int | None = None
    try:
        client.connect()
        if scope == "all":
            deleted = client.flush_all()
        elif scope == "db":
            deleted = client.flush_db(db_index)
        else:
            deleted = client.delete_pattern(key_pattern)
    except Exception as e:  # noqa: BLE001
        return {"success": False, "error": f"redis clear failed: {type(e).__name__}: {e}"}
    finally:
        client.close()

    now = _apply_dss(target_id, scope, new_flush_count, context)
    return {
        "success": True,
        "completed_at": now,
        "scope": scope,
        "db_index": db_index,
        "key_pattern": key_pattern,
        "flush_count": new_flush_count,
        "deleted": deleted,
        "host": client.host,
        "note": _note(target, scope, db_index, key_pattern, "real redis"),
    }
