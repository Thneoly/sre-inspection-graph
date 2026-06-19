"""clear_cache 执行器 — Sprint 3 mock 实现。

真实环境会调:
    redis-cli FLUSHALL                  # scope=all
    redis-cli -n <db> FLUSHDB           # scope=db
    redis-cli --scan --pattern P | xargs redis-cli DEL  # scope=pattern

Sprint 3 mock:更新 Redis.flush_count + cleared_at;
pattern 模式下要求 key_pattern 必填。
"""

from datetime import datetime, timezone
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

    now = datetime.now(timezone.utc).isoformat()
    old_flush_count = int(target.properties.get("flush_count", 0))
    new_flush_count = old_flush_count + 1

    store.update_node_props(
        target_id,
        flush_count=new_flush_count,
        cleared_at=now,
        cleared_by_execution=context.get("execution_id", ""),
        last_clear_scope=scope,
    )

    if scope == "all":
        note = f"Redis {target.name} FLUSHALL executed (mock)"
    elif scope == "db":
        note = f"Redis {target.name} FLUSHDB on db={db_index} (mock)"
    else:  # pattern
        note = f"Redis {target.name} keys matching '{key_pattern}' deleted (mock)"

    return {
        "success": True,
        "completed_at": now,
        "scope": scope,
        "db_index": db_index,
        "key_pattern": key_pattern,
        "flush_count": new_flush_count,
        "note": note,
    }
