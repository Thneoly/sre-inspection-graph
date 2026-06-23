"""PRD-001 Phase 2 余项 — 动作执行后自动验证测试。

覆盖:
1. 每个 verifier 的 happy path(mock + real 都查 DSS,目前 verifier 不区分模式)
2. verify_status=passed / failed / skipped / not_supported / error 五种
3. _verify_and_maybe_rollback:succeeded + verify_failed → auto rollback
4. auto_rollback=False 时只标 verify_status,不触发回滚
5. verify=False 跳过 verifier
6. POST /executions/{id}/verify reverify 端点
7. reverify 不会触发 auto rollback(防误触)
"""
from __future__ import annotations

from unittest.mock import patch

import pytest

from app.datasource.models import DataEdge, DataNode
from app.datasource.store import store
from app.recovery import verifiers


@pytest.fixture(autouse=True)
def _seed():
    store.nodes.clear()
    store.edges.clear()
    store.executions.clear()
    store.approvals.clear()

    nodes = [
        DataNode("deploy:vm-cluster:otel-demo:cart", "Deployment", "cart",
                 {"namespace": "otel-demo", "name": "cart", "cluster_id": "vm-cluster",
                  "desired_replicas": 3, "available_replicas": 3, "current_revision": 2,
                  "owner_team": "platform"}),
        DataNode("pod:vm-cluster:otel-demo:cart-1", "Pod", "cart-1",
                 {"namespace": "otel-demo", "name": "cart-1", "cluster_id": "vm-cluster",
                  "restart_count": 0, "health_status": "warning"}),
        DataNode("svc:vm-cluster:otel-demo:cart", "Service", "cart",
                 {"namespace": "otel-demo", "name": "cart", "cluster_id": "vm-cluster",
                  "endpoints_refresh_count": 0}),
        DataNode("secret:vm-cluster:otel-demo:cart-tls", "Secret", "cart-tls",
                 {"namespace": "otel-demo", "name": "cart-tls", "cluster_id": "vm-cluster",
                  "secret_version": 1}),
        DataNode("node:vm-cluster:worker-1", "KubernetesNode", "worker-1",
                 {"name": "worker-1", "cluster_id": "vm-cluster", "cordoned": False}),
        DataNode("mysql:vm-cluster:order-db", "MySQL", "order-db",
                 {"host": "mysql.local", "port": 3306}),
        DataNode("redis:vm-cluster:order-cache", "Redis", "order-cache",
                 {"host": "redis.local", "port": 6379}),
    ]
    for n in nodes:
        store.upsert_node(n)
    yield
    store.nodes.clear()
    store.edges.clear()
    store.executions.clear()
    store.approvals.clear()


# ============================================================
# 8 个 verifier 单测
# ============================================================

class TestVerifiers:
    def test_scale_passed(self):
        # DSS 已被 mock handler 改成 desired=available=5
        store.update_node_props("deploy:vm-cluster:otel-demo:cart",
                                 desired_replicas=5, available_replicas=5)
        v = verifiers.verify_scale_deployment(
            "deploy:vm-cluster:otel-demo:cart", {"replicas_delta": 2},
            {"new_replicas": 5}, {})
        assert v["passed"] is True

    def test_scale_failed_when_actual_mismatches(self):
        v = verifiers.verify_scale_deployment(
            "deploy:vm-cluster:otel-demo:cart", {"replicas_delta": 2},
            {"new_replicas": 5}, {})
        # DSS 还是 3/3 → 不等于 5 → failed
        assert v["passed"] is False
        assert "replicas mismatch" in v["message"]

    def test_restart_pod_passed_when_count_increased_and_health_recovered(self):
        store.update_node_props("pod:vm-cluster:otel-demo:cart-1",
                                 restart_count=1, health_status="normal")
        v = verifiers.verify_restart_pod(
            "pod:vm-cluster:otel-demo:cart-1", {}, {"new_restart_count": 1}, {})
        assert v["passed"] is True

    def test_restart_pod_failed_when_still_warning(self):
        store.update_node_props("pod:vm-cluster:otel-demo:cart-1",
                                 restart_count=1, health_status="warning")
        v = verifiers.verify_restart_pod(
            "pod:vm-cluster:otel-demo:cart-1", {}, {"new_restart_count": 1}, {})
        assert v["passed"] is False

    def test_restart_service_passed(self):
        store.update_node_props("svc:vm-cluster:otel-demo:cart", endpoints_refresh_count=1)
        v = verifiers.verify_restart_service(
            "svc:vm-cluster:otel-demo:cart", {}, {"endpoints_refresh_count": 1}, {})
        assert v["passed"] is True

    def test_refresh_secret_passed(self):
        store.update_node_props("secret:vm-cluster:otel-demo:cart-tls", secret_version=2)
        v = verifiers.verify_refresh_secret(
            "secret:vm-cluster:otel-demo:cart-tls", {}, {"new_version": 2}, {})
        assert v["passed"] is True

    def test_rollback_deployment_passed(self):
        store.update_node_props("deploy:vm-cluster:otel-demo:cart", current_revision=1)
        v = verifiers.verify_rollback_deployment(
            "deploy:vm-cluster:otel-demo:cart", {}, {"new_revision": 1}, {})
        assert v["passed"] is True

    def test_rollback_deployment_failed_on_wrong_revision(self):
        # DSS 还在 revision=2
        v = verifiers.verify_rollback_deployment(
            "deploy:vm-cluster:otel-demo:cart", {}, {"new_revision": 1}, {})
        assert v["passed"] is False

    def test_drain_node_passed(self):
        store.update_node_props("node:vm-cluster:worker-1", cordoned=True)
        v = verifiers.verify_drain_node("node:vm-cluster:worker-1", {}, {}, {})
        assert v["passed"] is True

    def test_drain_node_failed_when_not_cordoned(self):
        v = verifiers.verify_drain_node("node:vm-cluster:worker-1", {}, {}, {})
        assert v["passed"] is False

    def test_kill_query_not_supported(self):
        v = verifiers.verify_kill_query("mysql:vm-cluster:order-db", {}, {}, {})
        assert v["passed"] is True
        assert v["predicate"] == "not_supported"

    def test_clear_cache_not_supported(self):
        v = verifiers.verify_clear_cache("redis:vm-cluster:order-cache", {}, {}, {})
        assert v["predicate"] == "not_supported"

    def test_get_verifier(self):
        assert verifiers.get_verifier("scale_deployment") is not None
        assert verifiers.get_verifier("nonexistent") is None

    def test_run_verifier_handles_exception(self):
        # target_id 不存在 → verifier 内部捕获 → passed=False(restart_pod 走 not found 路径)
        v = verifiers.run_verifier(
            "scale_deployment", "nonexistent-target", {}, {"new_replicas": 5}, {})
        assert v["passed"] is False


# ============================================================
# 自动验证 + 自动回滚
# ============================================================

class TestAutoVerifyAndRollback:
    def test_succeeded_action_with_verify_passed(self, monkeypatch):
        """mock handler 正常更新 DSS → verifier passed → verify_status=passed。"""
        from app.config import settings
        monkeypatch.setattr(settings, "recovery_handler_mode", "mock")
        from app.recovery.execution import execute

        ex = execute(
            "scale_deployment",
            "deploy:vm-cluster:otel-demo:cart",
            {"replicas_delta": 2},
            initiated_by="alice",
        )
        assert ex.status == "succeeded"
        assert ex.verify_status == "passed"
        assert ex.verify_result["passed"] is True
        assert ex.verified_at != ""

    def test_verify_failed_triggers_auto_rollback(self, monkeypatch):
        """verifier 检查失败 → 自动调 rollback,原 execution.status=rolled_back。

        构造方法:让 verifier 返 passed=False(monkeypatch VERIFIERS)。
        """
        from app.config import settings
        from app.recovery import verifiers as ver_mod

        monkeypatch.setattr(settings, "recovery_handler_mode", "mock")

        def fake_verify(target_id, params, exec_result, context):
            return {"passed": False, "predicate": "scale_deployment",
                    "actual": 99, "expected": 5, "message": "fake mismatch"}

        monkeypatch.setitem(ver_mod.VERIFIERS, "scale_deployment", fake_verify)

        from app.recovery.execution import execute
        ex = execute(
            "scale_deployment",
            "deploy:vm-cluster:otel-demo:cart",
            {"replicas_delta": 2},
            initiated_by="alice",
        )
        # 原 execution 应被自动回滚 → status=rolled_back
        assert ex.verify_status == "failed"
        assert ex.status == "rolled_back"
        assert ex.rollback_execution_id is not None
        # 反向 execution 应存在
        rb = store.get_execution(ex.rollback_execution_id)
        assert rb is not None
        assert rb.reverses_execution_id == ex.execution_id
        assert rb.result.get("auto_rollback_origin") == ex.execution_id

    def test_verify_failed_without_rollback_action_warns(self, monkeypatch):
        """无 rollback_action_id 的动作 verify_failed → 加 warning,不创建反向 exec。"""
        from app.config import settings
        from app.recovery import verifiers as ver_mod

        monkeypatch.setattr(settings, "recovery_handler_mode", "mock")

        def fake_verify(target_id, params, exec_result, context):
            return {"passed": False, "predicate": "restart_service",
                    "actual": 0, "expected": 1, "message": "fake"}

        monkeypatch.setitem(ver_mod.VERIFIERS, "restart_service", fake_verify)

        from app.recovery.execution import execute
        # restart_service.rollback_action_id is None
        ex = execute(
            "restart_service",
            "svc:vm-cluster:otel-demo:cart",
            {},
            initiated_by="alice",
        )
        assert ex.verify_status == "failed"
        assert ex.status == "succeeded"  # 还是 succeeded,没回滚
        assert ex.rollback_execution_id is None
        assert any("manual intervention" in w for w in ex.result.get("warnings", []))

    def test_verify_false_skips_verifier(self, monkeypatch):
        """ExecuteRequest.verify=False → 不跑 verifier,verify_status="". """
        from app.config import settings
        monkeypatch.setattr(settings, "recovery_handler_mode", "mock")
        from app.recovery.execution import execute

        ex = execute(
            "scale_deployment",
            "deploy:vm-cluster:otel-demo:cart",
            {"replicas_delta": 1},
            initiated_by="alice",
            verify=False,
        )
        assert ex.status == "succeeded"
        assert ex.verify_status == ""

    def test_not_supported_verifier_sets_status(self, monkeypatch):
        """kill_query 默认 medium → 需审批;直接经审批通路跑完看 verify_status。"""
        from app.config import settings
        monkeypatch.setattr(settings, "recovery_handler_mode", "mock")
        from app.recovery.action_defs import ACTION_DEFS
        # 临时降到 low,避免走审批
        monkeypatch.setitem(ACTION_DEFS["kill_query"], "risk_level", "low")
        from app.recovery.execution import execute

        ex = execute(
            "kill_query",
            "mysql:vm-cluster:order-db",
            {"query_id": "42"},
            initiated_by="alice",
        )
        assert ex.status == "succeeded"
        assert ex.verify_status == "not_supported"

    def test_no_verifier_registered_sets_skipped(self, monkeypatch):
        """如果 action 没注册 verifier,run_verifier 返 predicate=skipped。"""
        from app.recovery import verifiers as ver_mod
        from app.config import settings
        monkeypatch.setattr(settings, "recovery_handler_mode", "mock")
        # 删掉 scale_deployment verifier
        monkeypatch.delitem(ver_mod.VERIFIERS, "scale_deployment")
        from app.recovery.execution import execute

        ex = execute(
            "scale_deployment",
            "deploy:vm-cluster:otel-demo:cart",
            {"replicas_delta": 1},
            initiated_by="alice",
        )
        assert ex.verify_status == "skipped"

    def test_rollback_execution_itself_does_not_re_verify(self, monkeypatch):
        """auto rollback 创建的反向 execution 自身 verify_status="",防递归。"""
        from app.config import settings
        from app.recovery import verifiers as ver_mod
        monkeypatch.setattr(settings, "recovery_handler_mode", "mock")

        def fake_verify(target_id, params, exec_result, context):
            return {"passed": False, "predicate": "scale_deployment",
                    "actual": 99, "expected": 5, "message": "fake"}

        monkeypatch.setitem(ver_mod.VERIFIERS, "scale_deployment", fake_verify)

        from app.recovery.execution import execute
        ex = execute(
            "scale_deployment",
            "deploy:vm-cluster:otel-demo:cart",
            {"replicas_delta": 2},
            initiated_by="alice",
        )
        rb = store.get_execution(ex.rollback_execution_id)
        assert rb.verify_status == ""  # 自动回滚的 execution 不再 verify


# ============================================================
# reverify 端点
# ============================================================

class TestReverifyEndpoint:
    def test_reverify_succeeded_execution(self, monkeypatch):
        from app.config import settings
        monkeypatch.setattr(settings, "recovery_handler_mode", "mock")
        from app.recovery.execution import execute, reverify

        ex = execute(
            "scale_deployment", "deploy:vm-cluster:otel-demo:cart",
            {"replicas_delta": 1}, initiated_by="alice", verify=False,
        )
        assert ex.verify_status == ""
        # 重新 verify
        ex2 = reverify(ex.execution_id)
        assert ex2.verify_status == "passed"

    def test_reverify_rejects_non_terminal_status(self, monkeypatch):
        from app.recovery.execution import ExecutionError, reverify
        from app.datasource.models import RecoveryExecution
        # 构造一个 awaiting_approval execution
        ex = RecoveryExecution(
            execution_id="x", action_id="scale_deployment",
            target_resource_id="deploy:vm-cluster:otel-demo:cart",
            target_resource_type="Deployment", status="awaiting_approval",
        )
        store.add_execution(ex)
        with pytest.raises(ExecutionError, match="reverify only allowed"):
            reverify("x")

    def test_reverify_does_not_trigger_auto_rollback(self, monkeypatch):
        """reverify 即便 verify_failed 也不触发 rollback(用户主动操作,不应有副作用)。"""
        from app.config import settings
        from app.recovery import verifiers as ver_mod
        monkeypatch.setattr(settings, "recovery_handler_mode", "mock")

        from app.recovery.execution import execute, reverify
        ex = execute(
            "scale_deployment", "deploy:vm-cluster:otel-demo:cart",
            {"replicas_delta": 1}, initiated_by="alice",
        )
        # 切 verifier 为 fail
        def fake_verify(target_id, params, exec_result, context):
            return {"passed": False, "predicate": "scale_deployment",
                    "message": "fake"}
        monkeypatch.setitem(ver_mod.VERIFIERS, "scale_deployment", fake_verify)

        ex2 = reverify(ex.execution_id)
        assert ex2.verify_status == "failed"
        # 状态保持 succeeded,没回滚
        assert ex2.status == "succeeded"
        assert ex2.rollback_execution_id is None
