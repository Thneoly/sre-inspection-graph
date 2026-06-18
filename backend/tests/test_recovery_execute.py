"""Recovery Action Execute 测试 — PRD-001 Sprint 2。

覆盖:
- handlers 直接调用(scale / kill / restart)
- execution.execute 编排(low_risk OK / medium_risk 501 / high_risk 501 / 类型不匹配)
- API: POST /execute / GET /executions / GET /executions/{id}
- list_executions 过滤

风格对齐 test_recovery.py(class-based + client fixture + DSS store seeding)。
"""

import pytest
from app.datasource.models import DataNode, DataEdge
from app.datasource.store import store


# ============================================================
# Fixture:种子数据
# ============================================================

@pytest.fixture(scope="module", autouse=True)
def _seed_store():
    """与 test_recovery.py 共享相似的小图,但加 MySQL/Redis/Service 节点。"""
    store.nodes.clear()
    store.edges.clear()
    store.executions.clear()

    nodes = [
        DataNode("app:order", "Application", "订单应用"),
        DataNode("comp:order-api", "ApplicationComponent", "订单API组件"),
        DataNode("deploy:order-api", "Deployment", "order-api",
                 properties={"desired_replicas": 3, "available_replicas": 3}),
        DataNode("pod:order-api-1", "Pod", "order-api-1"),
        DataNode("pod:order-api-2", "Pod", "order-api-2"),
        DataNode("svc:order-api", "Service", "order-api-svc"),
        DataNode("mysql:order-db", "MySQL", "order-db"),
        DataNode("redis:order-cache", "Redis", "order-cache"),
    ]
    for n in nodes:
        store.upsert_node(n)

    edges = [
        ("e1", "app:order", "CONTAINS", "comp:order-api"),
        ("e2", "comp:order-api", "DEPLOYED_AS", "deploy:order-api"),
        ("e3", "deploy:order-api", "CONTAINS", "pod:order-api-1"),
        ("e4", "deploy:order-api", "CONTAINS", "pod:order-api-2"),
        ("e5", "svc:order-api", "ROUTES_TO", "pod:order-api-1"),
    ]
    for eid, src, rel, tgt in edges:
        store.upsert_edge(DataEdge(eid, src, tgt, rel, rel))

    yield

    store.nodes.clear()
    store.edges.clear()
    store.executions.clear()


@pytest.fixture(autouse=True)
def _clear_executions():
    """每个测试都从干净 executions 开始。"""
    store.clear_executions()


# ============================================================
# 1. handlers 直接调用
# ============================================================

class TestScaleDeploymentHandler:
    def test_scale_up(self):
        from app.recovery.handlers.scale_deployment import execute
        result = execute("deploy:order-api", {"replicas_delta": 2}, {"execution_id": "exec-1"})
        assert result["success"] is True
        assert result["old_replicas"] == 3
        assert result["new_replicas"] == 5
        # DSS 状态被更新
        node = store.get_node("deploy:order-api")
        assert node.properties["desired_replicas"] == 5

    def test_scale_down(self):
        # 重置回 5(上一个测试改的)
        store.update_node_props("deploy:order-api", desired_replicas=5)
        from app.recovery.handlers.scale_deployment import execute
        result = execute("deploy:order-api", {"replicas_delta": -2}, {"execution_id": "exec-2"})
        assert result["success"] is True
        assert result["new_replicas"] == 3

    def test_zero_delta_rejected(self):
        from app.recovery.handlers.scale_deployment import execute
        result = execute("deploy:order-api", {"replicas_delta": 0}, {"execution_id": "exec-3"})
        assert result["success"] is False
        assert "non-zero" in result["error"]

    def test_negative_replicas_rejected(self):
        store.update_node_props("deploy:order-api", desired_replicas=1)
        from app.recovery.handlers.scale_deployment import execute
        result = execute("deploy:order-api", {"replicas_delta": -5}, {"execution_id": "exec-4"})
        assert result["success"] is False
        assert "negative" in result["error"]

    def test_target_not_deployment(self):
        from app.recovery.handlers.scale_deployment import execute
        result = execute("pod:order-api-1", {"replicas_delta": 1}, {"execution_id": "exec-5"})
        assert result["success"] is False
        assert "not Deployment" in result["error"]


class TestKillQueryHandler:
    def test_kill(self):
        from app.recovery.handlers.kill_query import execute
        result = execute("mysql:order-db",
                         {"query_id": "qid-12345", "min_duration_seconds": 30},
                         {"execution_id": "exec-1"})
        assert result["success"] is True
        assert result["query_id"] == "qid-12345"
        # MySQL 节点维护了 killed_queries 历史
        node = store.get_node("mysql:order-db")
        killed = node.properties["killed_queries"]
        assert any(k["query_id"] == "qid-12345" for k in killed)

    def test_missing_query_id(self):
        from app.recovery.handlers.kill_query import execute
        result = execute("mysql:order-db", {}, {"execution_id": "exec-2"})
        assert result["success"] is False
        assert "query_id" in result["error"]


class TestRestartServiceHandler:
    def test_restart(self):
        from app.recovery.handlers.restart_service import execute
        result = execute("svc:order-api", {}, {"execution_id": "exec-1"})
        assert result["success"] is True
        node = store.get_node("svc:order-api")
        assert node.properties["endpoints_refresh_count"] == 1

    def test_count_increments(self):
        from app.recovery.handlers.restart_service import execute
        execute("svc:order-api", {}, {"execution_id": "exec-1"})
        execute("svc:order-api", {}, {"execution_id": "exec-2"})
        node = store.get_node("svc:order-api")
        # 第一个测试已经 +1,这里又 +2 = 3
        assert node.properties["endpoints_refresh_count"] >= 2

    def test_target_not_service(self):
        from app.recovery.handlers.restart_service import execute
        result = execute("deploy:order-api", {}, {"execution_id": "exec-3"})
        assert result["success"] is False


# ============================================================
# 2. execution.execute 编排
# ============================================================

class TestExecutionFlow:
    def test_low_risk_succeeds(self):
        from app.recovery.execution import execute
        execution = execute(
            action_id="scale_deployment",
            target_resource_id="deploy:order-api",
            input_params={"replicas_delta": 1},
            initiated_by="alice",
            request_reason="business low peak",
        )
        assert execution.status == "succeeded"
        assert execution.result["success"] is True
        assert execution.executed_at
        assert execution.completed_at
        # 写入 store
        assert store.get_execution(execution.execution_id) is execution

    def test_medium_risk_blocked_in_sprint2(self):
        from app.recovery.execution import execute, ExecutionError
        with pytest.raises(ExecutionError) as exc:
            execute("restart_pod", "pod:order-api-1")
        assert exc.value.code == 501
        assert "Sprint 3" in exc.value.message

    def test_high_risk_blocked_in_sprint2(self):
        from app.recovery.execution import execute, ExecutionError
        with pytest.raises(ExecutionError) as exc:
            execute("rollback_deployment", "deploy:order-api")
        assert exc.value.code == 501

    def test_unknown_action(self):
        from app.recovery.execution import execute, ExecutionError
        with pytest.raises(ExecutionError) as exc:
            execute("nonexistent", "deploy:order-api")
        assert exc.value.code == 404

    def test_target_type_mismatch_fails_validation(self):
        from app.recovery.execution import execute, ExecutionError
        with pytest.raises(ExecutionError) as exc:
            execute("scale_deployment", "pod:order-api-1")    # Pod 不是 Deployment
        assert exc.value.code == 400
        assert "validation failed" in exc.value.message

    def test_handler_failure_recorded(self):
        """handler 内部失败:execution.status='failed',不抛异常。"""
        from app.recovery.execution import execute
        # 先把 deploy 副本调到 1
        store.update_node_props("deploy:order-api", desired_replicas=1)
        # 然后 scale -5,会被 handler 拒绝(变负数)
        execution = execute(
            action_id="scale_deployment",
            target_resource_id="deploy:order-api",
            input_params={"replicas_delta": -5},
        )
        assert execution.status == "failed"
        assert execution.result["success"] is False
        assert "negative" in execution.result["error"]


class TestListExecutions:
    def test_filter_by_status(self):
        from app.recovery.execution import execute, list_executions
        # 跑 2 个成功
        execute("scale_deployment", "deploy:order-api", {"replicas_delta": 1})
        execute("restart_service", "svc:order-api")
        # 跑 1 个失败
        store.update_node_props("deploy:order-api", desired_replicas=1)
        execute("scale_deployment", "deploy:order-api", {"replicas_delta": -10})

        succeeded = list_executions(status="succeeded")
        failed = list_executions(status="failed")
        assert len(succeeded) == 2
        assert len(failed) == 1

    def test_filter_by_action_id(self):
        from app.recovery.execution import execute, list_executions
        execute("scale_deployment", "deploy:order-api", {"replicas_delta": 1})
        execute("restart_service", "svc:order-api")
        scales = list_executions(action_id="scale_deployment")
        assert len(scales) == 1
        assert scales[0].action_id == "scale_deployment"

    def test_limit(self):
        from app.recovery.execution import execute, list_executions
        for _ in range(5):
            execute("restart_service", "svc:order-api")
        assert len(list_executions(limit=3)) == 3
        assert len(list_executions(limit=100)) == 5


# ============================================================
# 3. API 端点
# ============================================================

class TestExecuteEndpoint:
    def test_execute_low_risk(self, client):
        cli, _ = client
        resp = cli.post("/api/v1/recovery/execute", json={
            "action_id": "scale_deployment",
            "target_resource_id": "deploy:order-api",
            "input_params": {"replicas_delta": 1},
            "initiated_by": "alice",
        })
        assert resp.status_code == 200
        data = resp.json()
        assert data["status"] == "succeeded"
        assert data["initiated_by"] == "alice"
        assert "execution_id" in data
        assert data["result"]["success"] is True

    def test_execute_medium_risk_returns_501(self, client):
        cli, _ = client
        resp = cli.post("/api/v1/recovery/execute", json={
            "action_id": "restart_pod",
            "target_resource_id": "pod:order-api-1",
        })
        assert resp.status_code == 501

    def test_execute_high_risk_returns_501(self, client):
        cli, _ = client
        resp = cli.post("/api/v1/recovery/execute", json={
            "action_id": "rollback_deployment",
            "target_resource_id": "deploy:order-api",
        })
        assert resp.status_code == 501

    def test_execute_unknown_action_404(self, client):
        cli, _ = client
        resp = cli.post("/api/v1/recovery/execute", json={
            "action_id": "nonexistent",
            "target_resource_id": "deploy:order-api",
        })
        assert resp.status_code == 404

    def test_execute_type_mismatch_400(self, client):
        cli, _ = client
        resp = cli.post("/api/v1/recovery/execute", json={
            "action_id": "scale_deployment",
            "target_resource_id": "pod:order-api-1",
        })
        assert resp.status_code == 400


class TestListExecutionsEndpoint:
    def test_list_after_execute(self, client):
        cli, _ = client
        # 执行一次
        cli.post("/api/v1/recovery/execute", json={
            "action_id": "scale_deployment",
            "target_resource_id": "deploy:order-api",
            "input_params": {"replicas_delta": 1},
        })
        resp = cli.get("/api/v1/recovery/executions")
        assert resp.status_code == 200
        data = resp.json()
        assert data["total"] >= 1

    def test_filter_by_status(self, client):
        cli, _ = client
        cli.post("/api/v1/recovery/execute", json={
            "action_id": "restart_service",
            "target_resource_id": "svc:order-api",
        })
        resp = cli.get("/api/v1/recovery/executions?status=succeeded")
        assert resp.status_code == 200
        for e in resp.json()["executions"]:
            assert e["status"] == "succeeded"

    def test_invalid_status_422(self, client):
        cli, _ = client
        resp = cli.get("/api/v1/recovery/executions?status=banana")
        assert resp.status_code == 422


class TestExecutionDetailEndpoint:
    def test_get_after_execute(self, client):
        cli, _ = client
        resp = cli.post("/api/v1/recovery/execute", json={
            "action_id": "restart_service",
            "target_resource_id": "svc:order-api",
        })
        eid = resp.json()["execution_id"]

        detail_resp = cli.get(f"/api/v1/recovery/executions/{eid}")
        assert detail_resp.status_code == 200
        assert detail_resp.json()["execution_id"] == eid

    def test_unknown_execution_404(self, client):
        cli, _ = client
        resp = cli.get("/api/v1/recovery/executions/nonexistent-uuid")
        assert resp.status_code == 404
