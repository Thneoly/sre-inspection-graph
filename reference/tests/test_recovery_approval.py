"""Recovery Approval Flow 测试 — PRD-001 Sprint 3。

覆盖:
- approval module: request_approval / approve / reject / list_approvals / 24h 过期
- _derive_approver_team:owner_team 直接命中 / 沿 BELONGS_TO 上溯 / 默认值
- execution.rollback:scale_deployment 反向回滚 / 幂等 / 失败处理
- 5 个新 handler 各自的 happy/sad path
- API 端点:POST /approvals/{id}/approve|reject + GET /approvals + POST /executions/{id}/rollback

风格沿用 test_recovery_execute.py:class-based + module-scope seed + autouse clear。
"""

from datetime import datetime, timedelta, timezone

import pytest
from app.datasource.models import DataNode, DataEdge
from app.datasource.store import store


# ============================================================
# Fixture:种子数据(扩展 test_recovery_execute 的图,加 owner_team / Secret / Node)
# ============================================================

@pytest.fixture(scope="module", autouse=True)
def _seed_store():
    store.nodes.clear()
    store.edges.clear()
    store.executions.clear()
    store.clear_approvals()

    nodes = [
        DataNode("app:order", "Application", "订单应用",
                 properties={"owner_team": "订单团队"}),
        DataNode("comp:order-api", "ApplicationComponent", "订单API组件",
                 properties={"owner_team": "订单团队"}),
        DataNode("deploy:order-api", "Deployment", "order-api",
                 properties={"desired_replicas": 3, "available_replicas": 3,
                             "current_revision": 2}),
        DataNode("pod:order-api-1", "Pod", "order-api-1",
                 properties={"restart_count": 0, "health_status": "warning"}),
        DataNode("pod:order-api-2", "Pod", "order-api-2",
                 properties={"restart_count": 1}),
        DataNode("pod:no-team", "Pod", "no-team-pod"),  # 无 owner_team,不在 BELONGS_TO 链上
        DataNode("svc:order-api", "Service", "order-api-svc"),
        DataNode("mysql:order-db", "MySQL", "order-db"),
        DataNode("redis:order-cache", "Redis", "order-cache",
                 properties={"flush_count": 0}),
        DataNode("secret:order-jwt", "Secret", "order-jwt",
                 properties={"secret_version": 1}),
        DataNode("node:worker-01", "KubernetesNode", "worker-01"),
    ]
    for n in nodes:
        store.upsert_node(n)

    edges = [
        ("e1", "app:order", "CONTAINS", "comp:order-api"),
        ("e2", "comp:order-api", "DEPLOYED_AS", "deploy:order-api"),
        ("e3", "deploy:order-api", "CONTAINS", "pod:order-api-1"),
        ("e4", "deploy:order-api", "CONTAINS", "pod:order-api-2"),
        # BELONGS_TO 链:Pod → Component → Application(approver_team 上溯用)
        ("e5", "pod:order-api-1", "BELONGS_TO", "comp:order-api"),
        ("e6", "comp:order-api", "BELONGS_TO", "app:order"),
        # USES:Pod → Secret(refresh_secret 反向遍历用)
        ("e7", "pod:order-api-1", "USES", "secret:order-jwt"),
        ("e8", "pod:order-api-2", "USES", "secret:order-jwt"),
        # SCHEDULED_ON:Pod → Node(drain_node 反向遍历用)
        ("e9", "pod:order-api-1", "SCHEDULED_ON", "node:worker-01"),
        ("e10", "pod:order-api-2", "SCHEDULED_ON", "node:worker-01"),
    ]
    for eid, src, rel, tgt in edges:
        store.upsert_edge(DataEdge(eid, src, tgt, rel, rel))

    yield

    store.nodes.clear()
    store.edges.clear()
    store.executions.clear()
    store.clear_approvals()


@pytest.fixture(autouse=True)
def _reset_runtime():
    """每个测试干净 executions/approvals。"""
    store.clear_executions()
    store.clear_approvals()
    # 状态修复(handler 测试会改 properties)
    store.update_node_props("deploy:order-api",
                            desired_replicas=3, available_replicas=3,
                            current_revision=2)
    store.update_node_props("pod:order-api-1", restart_count=0, health_status="warning")
    store.update_node_props("redis:order-cache", flush_count=0)
    store.update_node_props("secret:order-jwt", secret_version=1)


# ============================================================
# 1. approval module
# ============================================================

class TestApprovalModule:
    def test_request_approval_creates_pending(self):
        from app.recovery.execution import execute as run_execution
        from app.recovery import approval as ap

        execution = run_execution(
            action_id="restart_pod",
            target_resource_id="pod:order-api-1",
            initiated_by="alice",
            request_reason="pod 重启降负",
        )
        assert execution.status == "awaiting_approval"
        assert execution.approval_id is not None

        approval = ap.get_approval(execution.approval_id)
        assert approval is not None
        assert approval.approval_status == "pending"
        assert approval.requested_by == "alice"
        assert approval.request_reason == "pod 重启降负"
        # owner_team 沿 BELONGS_TO 上溯到 Component(订单团队)
        assert approval.approver_team == "订单团队"

    def test_approve_triggers_execution_and_succeeds(self):
        from app.recovery.execution import execute as run_execution
        from app.recovery import approval as ap

        execution = run_execution("restart_pod", "pod:order-api-1", initiated_by="alice")
        approval, exec_after = ap.approve(execution.approval_id, "bob", "ok")

        assert approval.approval_status == "approved"
        assert approval.approver_id == "bob"
        assert exec_after.status == "succeeded"
        assert exec_after.result["success"] is True

    def test_reject_marks_execution_rejected(self):
        from app.recovery.execution import execute as run_execution
        from app.recovery import approval as ap

        execution = run_execution("restart_pod", "pod:order-api-1", initiated_by="alice")
        approval, exec_after = ap.reject(execution.approval_id, "bob", "风险太大")

        assert approval.approval_status == "rejected"
        assert exec_after.status == "rejected"
        assert "rejected by bob" in exec_after.result["error"]

    def test_double_approve_409(self):
        from app.recovery.execution import execute as run_execution
        from app.recovery import approval as ap

        execution = run_execution("restart_pod", "pod:order-api-1")
        ap.approve(execution.approval_id, "bob", "ok")
        with pytest.raises(ap.ApprovalError) as exc:
            ap.approve(execution.approval_id, "carol", "again")
        assert exc.value.code == 409

    def test_unknown_approval_404(self):
        from app.recovery import approval as ap
        with pytest.raises(ap.ApprovalError) as exc:
            ap.approve("nonexistent-id", "bob", "")
        assert exc.value.code == 404

    def test_expiry_read_time_check(self):
        """approval.expiry_at < now → 自动标 expired,无需后台 cron。"""
        from app.recovery.execution import execute as run_execution
        from app.recovery import approval as ap

        execution = run_execution("restart_pod", "pod:order-api-1")
        approval = ap.get_approval(execution.approval_id)
        # 把 expiry_at 改成过去
        past = (datetime.now(timezone.utc) - timedelta(hours=1)).isoformat()
        approval.expiry_at = past
        store.update_approval(approval)

        # 再读 → 自动 expired
        result = ap.get_approval(execution.approval_id)
        assert result.approval_status == "expired"

        # approve 已过期的 → 409
        with pytest.raises(ap.ApprovalError) as exc:
            ap.approve(execution.approval_id, "bob", "")
        assert exc.value.code == 409


class TestApproverTeamDerivation:
    def test_direct_owner_team(self):
        from app.recovery.approval import _derive_approver_team
        assert _derive_approver_team("comp:order-api") == "订单团队"

    def test_traverse_belongs_to(self):
        """Pod 没 owner_team,沿 BELONGS_TO → Component(订单团队)。"""
        from app.recovery.approval import _derive_approver_team
        assert _derive_approver_team("pod:order-api-1") == "订单团队"

    def test_default_platform(self):
        """无 owner_team 也无 BELONGS_TO 上溯路径 → 默认 platform。"""
        from app.recovery.approval import _derive_approver_team
        assert _derive_approver_team("pod:no-team") == "platform"

    def test_unknown_node_default(self):
        from app.recovery.approval import _derive_approver_team
        assert _derive_approver_team("nonexistent") == "platform"


# ============================================================
# 2. 5 个新 handler
# ============================================================

class TestRestartPodHandler:
    def test_restart_increments_count(self):
        from app.recovery.handlers.restart_pod import execute
        result = execute("pod:order-api-1", {}, {"execution_id": "exec-1"})
        assert result["success"] is True
        assert result["new_restart_count"] == 1
        node = store.get_node("pod:order-api-1")
        assert node.properties["restart_count"] == 1
        # warning → normal
        assert node.properties["health_status"] == "normal"

    def test_grace_period_out_of_range(self):
        from app.recovery.handlers.restart_pod import execute
        result = execute("pod:order-api-1", {"grace_period_seconds": 999},
                         {"execution_id": "exec-2"})
        assert result["success"] is False
        assert "grace_period" in result["error"]

    def test_target_not_pod(self):
        from app.recovery.handlers.restart_pod import execute
        result = execute("deploy:order-api", {}, {"execution_id": "exec-3"})
        assert result["success"] is False
        assert "not Pod" in result["error"]


class TestRollbackDeploymentHandler:
    def test_rollback_default_decrements(self):
        from app.recovery.handlers.rollback_deployment import execute
        result = execute("deploy:order-api", {}, {"execution_id": "exec-1"})
        assert result["success"] is True
        assert result["old_revision"] == 2
        assert result["new_revision"] == 1

    def test_rollback_specific_revision(self):
        store.update_node_props("deploy:order-api", current_revision=5)
        from app.recovery.handlers.rollback_deployment import execute
        result = execute("deploy:order-api", {"revision": 3}, {"execution_id": "exec-2"})
        assert result["success"] is True
        assert result["new_revision"] == 3

    def test_rollback_below_one(self):
        store.update_node_props("deploy:order-api", current_revision=1)
        from app.recovery.handlers.rollback_deployment import execute
        result = execute("deploy:order-api", {}, {"execution_id": "exec-3"})
        assert result["success"] is False
        assert "below revision 1" in result["error"]

    def test_rollback_to_newer_revision_rejected(self):
        from app.recovery.handlers.rollback_deployment import execute
        result = execute("deploy:order-api", {"revision": 99}, {"execution_id": "exec-4"})
        assert result["success"] is False
        assert "not older" in result["error"]


class TestRefreshSecretHandler:
    def test_refresh_increments_version_and_marks_pods(self):
        from app.recovery.handlers.refresh_secret import execute
        result = execute("secret:order-jwt", {"trigger_pod_restart": True},
                         {"execution_id": "exec-1"})
        assert result["success"] is True
        assert result["new_version"] == 2
        # 两个 USES 反向 Pod 都被标
        assert result["affected_pod_count"] == 2
        for pid in ["pod:order-api-1", "pod:order-api-2"]:
            assert store.get_node(pid).properties["pending_restart"] is True

    def test_refresh_without_pod_restart(self):
        from app.recovery.handlers.refresh_secret import execute
        result = execute("secret:order-jwt", {"trigger_pod_restart": False},
                         {"execution_id": "exec-2"})
        assert result["success"] is True
        assert result["affected_pod_count"] == 0

    def test_target_not_secret(self):
        from app.recovery.handlers.refresh_secret import execute
        result = execute("pod:order-api-1", {}, {"execution_id": "exec-3"})
        assert result["success"] is False


class TestDrainNodeHandler:
    def test_drain_marks_node_and_pods(self):
        from app.recovery.handlers.drain_node import execute
        result = execute("node:worker-01", {}, {"execution_id": "exec-1"})
        assert result["success"] is True
        assert result["drained_pod_count"] == 2
        node = store.get_node("node:worker-01")
        assert node.properties["cordoned"] is True
        # Pod 上有 eviction_pending
        for pid in ["pod:order-api-1", "pod:order-api-2"]:
            assert store.get_node(pid).properties["eviction_pending"] is True

    def test_target_not_node(self):
        from app.recovery.handlers.drain_node import execute
        result = execute("pod:order-api-1", {}, {"execution_id": "exec-2"})
        assert result["success"] is False


class TestClearCacheHandler:
    def test_clear_pattern(self):
        from app.recovery.handlers.clear_cache import execute
        result = execute("redis:order-cache",
                         {"scope": "pattern", "key_pattern": "user:*"},
                         {"execution_id": "exec-1"})
        assert result["success"] is True
        assert result["scope"] == "pattern"
        assert result["flush_count"] == 1

    def test_pattern_requires_key_pattern(self):
        from app.recovery.handlers.clear_cache import execute
        result = execute("redis:order-cache", {"scope": "pattern"},
                         {"execution_id": "exec-2"})
        assert result["success"] is False
        assert "key_pattern" in result["error"]

    def test_clear_all(self):
        from app.recovery.handlers.clear_cache import execute
        result = execute("redis:order-cache", {"scope": "all"},
                         {"execution_id": "exec-3"})
        assert result["success"] is True
        assert "FLUSHALL" in result["note"]

    def test_invalid_scope(self):
        from app.recovery.handlers.clear_cache import execute
        result = execute("redis:order-cache", {"scope": "garbage"},
                         {"execution_id": "exec-4"})
        assert result["success"] is False


# ============================================================
# 3. Rollback flow
# ============================================================

class TestRollback:
    def test_scale_rollback_reverses_delta(self):
        """scale_deployment +2 → rollback → -2。"""
        from app.recovery.execution import execute, rollback

        original = execute(
            action_id="scale_deployment",
            target_resource_id="deploy:order-api",
            input_params={"replicas_delta": 2},
        )
        assert original.status == "succeeded"
        assert store.get_node("deploy:order-api").properties["desired_replicas"] == 5

        rb = rollback(original.execution_id, initiated_by="alice", reason="oversold")
        assert rb.status == "succeeded"
        assert rb.reverses_execution_id == original.execution_id

        # 原 execution 标 rolled_back
        original_after = store.get_execution(original.execution_id)
        assert original_after.status == "rolled_back"
        assert original_after.rollback_execution_id == rb.execution_id
        # 副本数回到 3
        assert store.get_node("deploy:order-api").properties["desired_replicas"] == 3

    def test_rollback_only_succeeded(self):
        from app.recovery.execution import execute, rollback, ExecutionError

        # restart_pod → awaiting_approval(不是 succeeded)
        execution = execute("restart_pod", "pod:order-api-1")
        with pytest.raises(ExecutionError) as exc:
            rollback(execution.execution_id)
        assert exc.value.code == 409

    def test_rollback_idempotent(self):
        """succeeded 的 execution 只能回滚一次。"""
        from app.recovery.execution import execute, rollback, ExecutionError

        original = execute("scale_deployment", "deploy:order-api",
                           {"replicas_delta": 1})
        rollback(original.execution_id)
        with pytest.raises(ExecutionError) as exc:
            rollback(original.execution_id)
        assert exc.value.code == 409

    def test_rollback_no_rollback_action_id(self):
        """restart_service 没有 rollback_action_id。"""
        from app.recovery.execution import execute, rollback, ExecutionError

        original = execute("restart_service", "svc:order-api")
        with pytest.raises(ExecutionError) as exc:
            rollback(original.execution_id)
        assert exc.value.code == 400
        assert "rollback_action_id" in exc.value.message


# ============================================================
# 4. API 端点
# ============================================================

class TestApprovalEndpoints:
    def test_execute_medium_returns_202(self, client):
        cli, _ = client
        resp = cli.post("/api/v1/recovery/execute", json={
            "action_id": "restart_pod",
            "target_resource_id": "pod:order-api-1",
            "initiated_by": "alice",
            "request_reason": "pod 不稳定",
        })
        assert resp.status_code == 202
        data = resp.json()
        assert data["status"] == "awaiting_approval"
        assert data["approval_id"]

    def test_list_approvals(self, client):
        cli, _ = client
        cli.post("/api/v1/recovery/execute", json={
            "action_id": "restart_pod",
            "target_resource_id": "pod:order-api-1",
        })
        resp = cli.get("/api/v1/recovery/approvals?status=pending")
        assert resp.status_code == 200
        data = resp.json()
        assert data["total"] == 1
        assert data["approvals"][0]["approval_status"] == "pending"
        assert data["approvals"][0]["execution_summary"]["action_id"] == "restart_pod"

    def test_approve_via_api_runs_handler(self, client):
        cli, _ = client
        exec_resp = cli.post("/api/v1/recovery/execute", json={
            "action_id": "restart_pod",
            "target_resource_id": "pod:order-api-1",
        })
        approval_id = exec_resp.json()["approval_id"]

        resp = cli.post(f"/api/v1/recovery/approvals/{approval_id}/approve",
                        json={"approver_id": "bob", "comment": "确认"})
        assert resp.status_code == 200
        body = resp.json()
        assert body["approval"]["approval_status"] == "approved"
        assert body["execution"]["status"] == "succeeded"

    def test_reject_via_api(self, client):
        cli, _ = client
        exec_resp = cli.post("/api/v1/recovery/execute", json={
            "action_id": "restart_pod",
            "target_resource_id": "pod:order-api-1",
        })
        approval_id = exec_resp.json()["approval_id"]

        resp = cli.post(f"/api/v1/recovery/approvals/{approval_id}/reject",
                        json={"approver_id": "bob", "comment": "不批"})
        assert resp.status_code == 200
        body = resp.json()
        assert body["approval"]["approval_status"] == "rejected"
        assert body["execution"]["status"] == "rejected"

    def test_approve_double_returns_409(self, client):
        cli, _ = client
        exec_resp = cli.post("/api/v1/recovery/execute", json={
            "action_id": "restart_pod",
            "target_resource_id": "pod:order-api-1",
        })
        approval_id = exec_resp.json()["approval_id"]
        cli.post(f"/api/v1/recovery/approvals/{approval_id}/approve",
                 json={"approver_id": "bob"})
        resp = cli.post(f"/api/v1/recovery/approvals/{approval_id}/approve",
                        json={"approver_id": "carol"})
        assert resp.status_code == 409

    def test_approval_404(self, client):
        cli, _ = client
        resp = cli.get("/api/v1/recovery/approvals/nonexistent")
        assert resp.status_code == 404


class TestRollbackEndpoint:
    def test_rollback_success(self, client):
        cli, _ = client
        exec_resp = cli.post("/api/v1/recovery/execute", json={
            "action_id": "scale_deployment",
            "target_resource_id": "deploy:order-api",
            "input_params": {"replicas_delta": 2},
        })
        eid = exec_resp.json()["execution_id"]

        resp = cli.post(f"/api/v1/recovery/executions/{eid}/rollback",
                        json={"initiated_by": "alice", "reason": "回滚"})
        assert resp.status_code == 200
        data = resp.json()
        assert data["status"] == "succeeded"
        assert data["reverses_execution_id"] == eid

    def test_rollback_unknown_404(self, client):
        cli, _ = client
        resp = cli.post("/api/v1/recovery/executions/nonexistent/rollback",
                        json={"initiated_by": "alice"})
        assert resp.status_code == 404

    def test_rollback_pending_409(self, client):
        cli, _ = client
        # restart_pod → awaiting_approval (not succeeded)
        exec_resp = cli.post("/api/v1/recovery/execute", json={
            "action_id": "restart_pod",
            "target_resource_id": "pod:order-api-1",
        })
        eid = exec_resp.json()["execution_id"]
        resp = cli.post(f"/api/v1/recovery/executions/{eid}/rollback",
                        json={"initiated_by": "alice"})
        assert resp.status_code == 409
