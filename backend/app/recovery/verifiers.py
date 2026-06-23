"""Recovery action verifiers — PRD-001 Phase 2 余项。

每个 verifier 是 sync `def verify(target_id, params, exec_result, context) -> dict`:
- mock 模式(默认):查 DSS 节点 properties 是否符合 predicate
- real 模式:查真实 K8s/MySQL/Redis 状态(目前留 SKIPPED,Phase 3 接 read API)

返回 `{passed: bool, predicate: str, actual: any, expected: any, message: str}`。
verifier 抛异常会被 `_verify_and_maybe_rollback` 兜底转 `passed=False + error`。

注册器 VERIFIERS 与 HANDLERS 平行。新增 verifier 步骤:
  1. 写一个 verify_<action> 函数
  2. 在 VERIFIERS dict 注册
"""
from __future__ import annotations

from typing import Any, Callable, Optional

from app.config import settings
from app.datasource.store import store


# ============================================================
# 通用工具
# ============================================================

def _make(passed: bool, predicate: str, actual: Any = None, expected: Any = None,
          message: str = "") -> dict:
    return {
        "passed": passed,
        "predicate": predicate,
        "actual": actual,
        "expected": expected,
        "message": message,
    }


def _not_supported(action_id: str) -> dict:
    return _make(True, "not_supported", message=f"{action_id} has no observable side effect to verify")


# ============================================================
# 8 个 verifier
# ============================================================

def verify_scale_deployment(target_id: str, params: dict, exec_result: dict, context: dict) -> dict:
    """期望:Deployment desired_replicas == available_replicas == exec_result.new_replicas。"""
    expected = exec_result.get("new_replicas")
    if expected is None:
        return _make(False, "scale_deployment", message="exec_result missing new_replicas")
    node = store.get_node(target_id)
    if node is None:
        return _make(False, "scale_deployment", message=f"target not found: {target_id}")
    actual_desired = int(node.properties.get("desired_replicas", -1))
    actual_avail = int(node.properties.get("available_replicas", -1))
    passed = actual_desired == expected and actual_avail == expected
    return _make(passed, "scale_deployment",
                 actual={"desired": actual_desired, "available": actual_avail},
                 expected={"desired": expected, "available": expected},
                 message="" if passed else
                 f"replicas mismatch (desired={actual_desired}/{expected}, available={actual_avail}/{expected})")


def verify_restart_pod(target_id: str, params: dict, exec_result: dict, context: dict) -> dict:
    """期望:restart_count > 上次值;health_status 不再是 warning。"""
    expected_count = exec_result.get("new_restart_count")
    node = store.get_node(target_id)
    if node is None:
        return _make(False, "restart_pod", message=f"target not found: {target_id}")
    actual_count = int(node.properties.get("restart_count", 0))
    health = node.properties.get("health_status", "")
    # 真实模式:Pod 删除后 K8s 重建,DSS 端不能即时反映 restart_count;预期是 ≥ exec_result.new_restart_count
    passed = (expected_count is not None and actual_count >= expected_count
              and health in ("", "normal", "healthy"))
    return _make(passed, "restart_pod",
                 actual={"restart_count": actual_count, "health_status": health},
                 expected={"restart_count_min": expected_count, "health_status_not": "warning"},
                 message="" if passed else
                 f"pod not yet restarted or still warning (count={actual_count}/{expected_count}, health={health})")


def verify_restart_service(target_id: str, params: dict, exec_result: dict, context: dict) -> dict:
    """期望:endpoints_refresh_count 自增。"""
    expected = exec_result.get("endpoints_refresh_count")
    node = store.get_node(target_id)
    if node is None:
        return _make(False, "restart_service", message=f"target not found: {target_id}")
    actual = int(node.properties.get("endpoints_refresh_count", 0))
    passed = expected is not None and actual >= expected
    return _make(passed, "restart_service",
                 actual=actual, expected=expected,
                 message="" if passed else
                 f"endpoints not refreshed (count={actual}/{expected})")


def verify_refresh_secret(target_id: str, params: dict, exec_result: dict, context: dict) -> dict:
    """期望:secret_version 自增。"""
    expected = exec_result.get("new_version")
    node = store.get_node(target_id)
    if node is None:
        return _make(False, "refresh_secret", message=f"target not found: {target_id}")
    actual = int(node.properties.get("secret_version", -1))
    passed = expected is not None and actual >= expected
    return _make(passed, "refresh_secret",
                 actual=actual, expected=expected,
                 message="" if passed else f"secret_version mismatch ({actual}/{expected})")


def verify_rollback_deployment(target_id: str, params: dict, exec_result: dict, context: dict) -> dict:
    """期望:current_revision == exec_result.new_revision。"""
    expected = exec_result.get("new_revision")
    node = store.get_node(target_id)
    if node is None:
        return _make(False, "rollback_deployment", message=f"target not found: {target_id}")
    actual = int(node.properties.get("current_revision", -1))
    passed = expected is not None and actual == expected
    return _make(passed, "rollback_deployment",
                 actual=actual, expected=expected,
                 message="" if passed else f"revision mismatch ({actual} != {expected})")


def verify_drain_node(target_id: str, params: dict, exec_result: dict, context: dict) -> dict:
    """期望:cordoned=True。"""
    node = store.get_node(target_id)
    if node is None:
        return _make(False, "drain_node", message=f"target not found: {target_id}")
    cordoned = bool(node.properties.get("cordoned"))
    passed = cordoned is True
    return _make(passed, "drain_node",
                 actual=cordoned, expected=True,
                 message="" if passed else "node not cordoned")


def verify_kill_query(target_id: str, params: dict, exec_result: dict, context: dict) -> dict:
    """kill_query 一次性,无持续副作用可观测。"""
    return _not_supported("kill_query")


def verify_clear_cache(target_id: str, params: dict, exec_result: dict, context: dict) -> dict:
    """clear_cache 一次性,缓存清空后无持续副作用可观测。"""
    return _not_supported("clear_cache")


# ============================================================
# 注册器
# ============================================================

VERIFIERS: dict[str, Callable[..., dict]] = {
    "scale_deployment": verify_scale_deployment,
    "restart_pod": verify_restart_pod,
    "restart_service": verify_restart_service,
    "refresh_secret": verify_refresh_secret,
    "rollback_deployment": verify_rollback_deployment,
    "drain_node": verify_drain_node,
    "kill_query": verify_kill_query,
    "clear_cache": verify_clear_cache,
}


def get_verifier(action_id: str) -> Optional[Callable[..., dict]]:
    """获取 verifier。返回 None 表示该 action 没注册 verifier(verify_status=skipped)。"""
    return VERIFIERS.get(action_id)


def run_verifier(action_id: str, target_id: str, params: dict,
                 exec_result: dict, context: dict) -> dict:
    """统一入口,捕获 verifier 内部异常。"""
    verifier = get_verifier(action_id)
    if verifier is None:
        return _make(True, "skipped", message=f"no verifier registered for {action_id}")
    try:
        return verifier(target_id, params, exec_result, context)
    except Exception as e:  # noqa: BLE001
        return _make(False, "error",
                     message=f"verifier raised: {type(e).__name__}: {e}")
