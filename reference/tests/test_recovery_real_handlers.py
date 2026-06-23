"""PRD-001 Phase 2 真实 handler 测试。

模式:`RECOVERY_HANDLER_MODE=real` + mock K8s client。
关键:handler 用 `from ... import get_k8s_apps_api, run_k8s` 直接绑定本地名,
所以 patch 必须打到 **handler 模块的引用**(如 `app.recovery.handlers.scale_deployment.get_k8s_apps_api`),
而不是 `k8s_client.get_k8s_apps_api`。`run_k8s` 让它自然 `asyncio.run`(测试无 running loop)。

断言:① 真实 API 被正确调用(namespace/name/参数)② 成功后 DSS 孪生更新
③ API 异常 → success=False + DSS 不动 ④ mock 模式不调真实 client(回归保护)
"""

from __future__ import annotations

import asyncio
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

from app.config import settings
from app.datasource.models import DataEdge, DataNode
from app.datasource.store import store


# ============================================================
# 种子:节点带 namespace/name properties(mapper 写入的字段)
# ============================================================

@pytest.fixture(autouse=True)
def _seed_store():
    """每个测试重建节点(避免 handler 改 properties 后跨测试污染)。"""
    store.nodes.clear()
    store.edges.clear()

    nodes = [
        DataNode("deploy:vm-cluster:otel-demo:cart", "Deployment", "cart",
                 {"desired_replicas": 3, "available_replicas": 3,
                  "namespace": "otel-demo", "name": "cart", "current_revision": 2}),
        DataNode("pod:vm-cluster:otel-demo:cart-1", "Pod", "cart-1",
                 {"namespace": "otel-demo", "name": "cart-1", "health_status": "warning",
                  "restart_count": 0}),
        DataNode("svc:vm-cluster:otel-demo:cart", "Service", "cart",
                 {"namespace": "otel-demo", "name": "cart", "endpoints_refresh_count": 0}),
        DataNode("secret:vm-cluster:otel-demo:cart-tls", "Secret", "cart-tls",
                 {"namespace": "otel-demo", "name": "cart-tls", "secret_version": 1, "data": {"tls.crt": "abc"}}),
        DataNode("node:vm-cluster:worker-1", "KubernetesNode", "worker-1",
                 {"name": "worker-1", "cordoned": False}),
        DataNode("mysql:vm-cluster:order-db", "MySQL", "order-db",
                 {"host": "mysql.vm-cluster", "port": 3306}),
        DataNode("redis:vm-cluster:order-cache", "Redis", "order-cache",
                 {"host": "redis.vm-cluster", "port": 6379}),
    ]
    for n in nodes:
        store.upsert_node(n)

    edges = [
        ("e1", "pod:vm-cluster:otel-demo:cart-1", "USES", "secret:vm-cluster:otel-demo:cart-tls"),
        ("e2", "pod:vm-cluster:otel-demo:cart-1", "SCHEDULED_ON", "node:vm-cluster:worker-1"),
    ]
    for eid, src, rel, tgt in edges:
        store.upsert_edge(DataEdge(eid, src, tgt, rel, rel))

    yield
    store.nodes.clear()
    store.edges.clear()


@pytest.fixture(autouse=True)
def _reset_runtime():
    store.executions.clear()
    yield
    store.executions.clear()


@pytest.fixture
def real_mode(monkeypatch):
    """切到 real 模式 + reset K8s loaded 标记。"""
    monkeypatch.setattr(settings, "recovery_handler_mode", "real")
    from app.datasource.connectors import k8s_client
    k8s_client.reset_loaded_cluster()
    yield
    k8s_client.reset_loaded_cluster()


def _make_apis():
    """返回 (api_client, apps, core)。每个用到的 API 方法是 AsyncMock。"""
    api_client = MagicMock()
    api_client.close = AsyncMock()
    apps = MagicMock()
    core = MagicMock()
    apps.patch_namespaced_deployment_scale = AsyncMock()
    apps.patch_namespaced_deployment = AsyncMock()
    core.delete_namespaced_pod = AsyncMock()
    core.delete_namespaced_endpoints = AsyncMock()
    core.patch_namespaced_secret = AsyncMock()
    core.patch_node = AsyncMock()
    return api_client, apps, core


def _patch_apps(handler_mod, api_client, apps):
    """patch handler 模块里的 get_k8s_apps_api 引用。run_k8s 保持自然 asyncio.run。"""
    return patch.object(
        handler_mod, "get_k8s_apps_api",
        new=AsyncMock(return_value=(api_client, apps)),
    )


def _patch_core(handler_mod, api_client, core):
    return patch.object(
        handler_mod, "get_k8s_core_api",
        new=AsyncMock(return_value=(api_client, core)),
    )


# ============================================================
# scale_deployment
# ============================================================

class TestScaleDeploymentReal:
    def test_real_calls_patch_scale_and_updates_dss(self, real_mode):
        from app.recovery.handlers import scale_deployment
        api_client, apps, _ = _make_apis()
        with _patch_apps(scale_deployment, api_client, apps):
            result = scale_deployment.execute(
                "deploy:vm-cluster:otel-demo:cart",
                {"replicas_delta": 2}, {"execution_id": "exec-1"},
            )
        assert result["success"] is True
        assert result["new_replicas"] == 5
        apps.patch_namespaced_deployment_scale.assert_awaited_once()
        call = apps.patch_namespaced_deployment_scale.call_args
        assert call.kwargs["name"] == "cart"
        assert call.kwargs["namespace"] == "otel-demo"
        assert store.get_node("deploy:vm-cluster:otel-demo:cart").properties["desired_replicas"] == 5

    def test_real_api_failure_leaves_dss_untouched(self, real_mode):
        from app.recovery.handlers import scale_deployment
        api_client, apps, _ = _make_apis()
        apps.patch_namespaced_deployment_scale = AsyncMock(side_effect=RuntimeError("409 conflict"))
        with _patch_apps(scale_deployment, api_client, apps):
            result = scale_deployment.execute(
                "deploy:vm-cluster:otel-demo:cart",
                {"replicas_delta": 1}, {"execution_id": "exec-2"},
            )
        assert result["success"] is False
        assert "409 conflict" in result["error"]
        assert store.get_node("deploy:vm-cluster:otel-demo:cart").properties["desired_replicas"] == 3


# ============================================================
# restart_pod
# ============================================================

class TestRestartPodReal:
    def test_real_calls_delete_pod_and_increments_count(self, real_mode):
        from app.recovery.handlers import restart_pod
        api_client, _, core = _make_apis()
        with _patch_core(restart_pod, api_client, core):
            result = restart_pod.execute(
                "pod:vm-cluster:otel-demo:cart-1",
                {"grace_period_seconds": 10}, {"execution_id": "exec-3"},
            )
        assert result["success"] is True
        assert result["new_restart_count"] == 1
        core.delete_namespaced_pod.assert_awaited_once()
        call = core.delete_namespaced_pod.call_args
        assert call.kwargs["name"] == "cart-1"
        assert call.kwargs["namespace"] == "otel-demo"
        assert store.get_node("pod:vm-cluster:otel-demo:cart-1").properties["health_status"] == "normal"


# ============================================================
# restart_service
# ============================================================

class TestRestartServiceReal:
    def test_real_calls_delete_endpoints(self, real_mode):
        from app.recovery.handlers import restart_service
        api_client, _, core = _make_apis()
        with _patch_core(restart_service, api_client, core):
            result = restart_service.execute(
                "svc:vm-cluster:otel-demo:cart", {}, {"execution_id": "exec-4"},
            )
        assert result["success"] is True
        core.delete_namespaced_endpoints.assert_awaited_once()
        assert core.delete_namespaced_endpoints.call_args.kwargs["name"] == "cart"
        assert store.get_node("svc:vm-cluster:otel-demo:cart").properties["endpoints_refresh_count"] == 1


# ============================================================
# refresh_secret
# ============================================================

class TestRefreshSecretReal:
    def test_real_patches_secret_and_marks_pods(self, real_mode):
        from app.recovery.handlers import refresh_secret
        api_client, _, core = _make_apis()
        with _patch_core(refresh_secret, api_client, core):
            result = refresh_secret.execute(
                "secret:vm-cluster:otel-demo:cart-tls",
                {}, {"execution_id": "exec-5"},
            )
        assert result["success"] is True
        assert result["new_version"] == 2
        core.patch_namespaced_secret.assert_awaited_once()
        pod = store.get_node("pod:vm-cluster:otel-demo:cart-1")
        assert pod.properties.get("pending_restart") is True


# ============================================================
# rollback_deployment
# ============================================================

class TestRollbackDeploymentReal:
    def test_real_patches_deployment_annotation(self, real_mode):
        from app.recovery.handlers import rollback_deployment
        api_client, apps, _ = _make_apis()
        with _patch_apps(rollback_deployment, api_client, apps):
            result = rollback_deployment.execute(
                "deploy:vm-cluster:otel-demo:cart",
                {}, {"execution_id": "exec-6"},
            )
        assert result["success"] is True
        assert result["new_revision"] == 1
        apps.patch_namespaced_deployment.assert_awaited_once()
        assert store.get_node("deploy:vm-cluster:otel-demo:cart").properties["current_revision"] == 1


# ============================================================
# drain_node
# ============================================================

class TestDrainNodeReal:
    def test_real_cordons_node_marks_pods(self, real_mode):
        from app.recovery.handlers import drain_node
        api_client, _, core = _make_apis()
        with _patch_core(drain_node, api_client, core):
            result = drain_node.execute(
                "node:vm-cluster:worker-1",
                {}, {"execution_id": "exec-7"},
            )
        assert result["success"] is True
        core.patch_node.assert_awaited_once()
        assert core.patch_node.call_args.kwargs["name"] == "worker-1"
        assert store.get_node("node:vm-cluster:worker-1").properties["cordoned"] is True
        assert store.get_node("pod:vm-cluster:otel-demo:cart-1").properties.get("eviction_pending") is True

    def test_real_api_failure_returns_failed(self, real_mode):
        from app.recovery.handlers import drain_node
        api_client, _, core = _make_apis()
        core.patch_node = AsyncMock(side_effect=RuntimeError("boom"))
        with _patch_core(drain_node, api_client, core):
            result = drain_node.execute(
                "node:vm-cluster:worker-1", {}, {"execution_id": "exec-8"},
            )
        assert result["success"] is False
        assert "boom" in result["error"]
        assert store.get_node("node:vm-cluster:worker-1").properties["cordoned"] is False


# ============================================================
# kill_query (MySQL)
# ============================================================

class TestKillQueryReal:
    def test_real_calls_mysql_kill(self, real_mode):
        from app.recovery.handlers import kill_query
        from app.recovery.clients import mysql_client

        mock_client = MagicMock()
        mock_client.connect = MagicMock()
        mock_client.kill = MagicMock()
        mock_client.close = MagicMock()
        mock_client.host = "mysql.vm-cluster"

        with patch.object(mysql_client, "MySQLClient") as MC:
            MC.from_node.return_value = mock_client
            result = kill_query.execute(
                "mysql:vm-cluster:order-db",
                {"query_id": 42}, {"execution_id": "exec-mq"},
            )
        assert result["success"] is True
        mock_client.kill.assert_called_once_with(42)
        # DSS killed_queries 历史追加
        node = store.get_node("mysql:vm-cluster:order-db")
        assert node.properties["killed_queries"][-1]["query_id"] == 42

    def test_real_mysql_failure_returns_failed(self, real_mode):
        from app.recovery.handlers import kill_query
        from app.recovery.clients import mysql_client

        mock_client = MagicMock()
        mock_client.connect = MagicMock()
        mock_client.kill = MagicMock(side_effect=RuntimeError("connection refused"))
        mock_client.close = MagicMock()

        with patch.object(mysql_client, "MySQLClient") as MC:
            MC.from_node.return_value = mock_client
            result = kill_query.execute(
                "mysql:vm-cluster:order-db",
                {"query_id": 99}, {"execution_id": "exec-mq2"},
            )
        assert result["success"] is False
        assert "connection refused" in result["error"]
        # DSS 不动
        assert store.get_node("mysql:vm-cluster:order-db").properties.get("killed_queries", []) == []


# ============================================================
# clear_cache (Redis)
# ============================================================

class TestClearCacheReal:
    def test_real_calls_redis_flush_all(self, real_mode):
        from app.recovery.handlers import clear_cache
        from app.recovery.clients import redis_client

        mock_client = MagicMock()
        mock_client.connect = MagicMock()
        mock_client.flush_all = MagicMock(return_value=1)
        mock_client.flush_db = MagicMock(return_value=1)
        mock_client.delete_pattern = MagicMock(return_value=5)
        mock_client.close = MagicMock()
        mock_client.host = "redis.vm-cluster"

        with patch.object(redis_client, "RedisClient") as RC:
            RC.from_node.return_value = mock_client
            result = clear_cache.execute(
                "redis:vm-cluster:order-cache",
                {"scope": "all"}, {"execution_id": "exec-rc"},
            )
        assert result["success"] is True
        mock_client.flush_all.assert_called_once()
        assert store.get_node("redis:vm-cluster:order-cache").properties["flush_count"] == 1

    def test_real_redis_pattern_delete(self, real_mode):
        from app.recovery.handlers import clear_cache
        from app.recovery.clients import redis_client

        mock_client = MagicMock()
        mock_client.connect = MagicMock()
        mock_client.delete_pattern = MagicMock(return_value=7)
        mock_client.close = MagicMock()

        with patch.object(redis_client, "RedisClient") as RC:
            RC.from_node.return_value = mock_client
            result = clear_cache.execute(
                "redis:vm-cluster:order-cache",
                {"scope": "pattern", "key_pattern": "session:*"}, {"execution_id": "exec-rc2"},
            )
        assert result["success"] is True
        mock_client.delete_pattern.assert_called_once_with("session:*")
        assert result["deleted"] == 7

    def test_real_redis_failure_returns_failed(self, real_mode):
        from app.recovery.handlers import clear_cache
        from app.recovery.clients import redis_client

        mock_client = MagicMock()
        mock_client.connect = MagicMock(side_effect=RuntimeError("auth failed"))
        mock_client.close = MagicMock()

        with patch.object(redis_client, "RedisClient") as RC:
            RC.from_node.return_value = mock_client
            result = clear_cache.execute(
                "redis:vm-cluster:order-cache",
                {"scope": "all"}, {"execution_id": "exec-rc3"},
            )
        assert result["success"] is False
        assert "auth failed" in result["error"]
        assert store.get_node("redis:vm-cluster:order-cache").properties.get("flush_count", 0) == 0


# ============================================================
# mode 切换回归保护
# ============================================================

class TestModeToggle:
    def test_mock_mode_does_not_call_k8s(self, monkeypatch):
        """mock 模式下 handler 不调任何 K8s API。"""
        monkeypatch.setattr(settings, "recovery_handler_mode", "mock")
        from app.recovery.handlers import scale_deployment

        api_client, apps, _ = _make_apis()
        # 即便 patch 了 get_k8s_apps_api 引用,mock 模式分支不会调到
        with _patch_apps(scale_deployment, api_client, apps):
            result = scale_deployment.execute(
                "deploy:vm-cluster:otel-demo:cart",
                {"replicas_delta": 1}, {"execution_id": "exec-mock"},
            )
        assert result["success"] is True
        assert "mock" in result["note"]
        apps.patch_namespaced_deployment_scale.assert_not_called()

    def test_mock_mode_does_not_call_mysql_or_redis(self, monkeypatch):
        """mock 模式下 kill_query / clear_cache 不调真实客户端。"""
        monkeypatch.setattr(settings, "recovery_handler_mode", "mock")
        from app.recovery.handlers import clear_cache, kill_query
        from app.recovery.clients import mysql_client, redis_client

        with patch.object(mysql_client, "MySQLClient") as MC, \
             patch.object(redis_client, "RedisClient") as RC:
            MC.from_node.return_value = MagicMock()
            RC.from_node.return_value = MagicMock()

            r1 = kill_query.execute(
                "mysql:vm-cluster:order-db",
                {"query_id": 1}, {"execution_id": "x"},
            )
            r2 = clear_cache.execute(
                "redis:vm-cluster:order-cache",
                {"scope": "all"}, {"execution_id": "y"},
            )
        assert r1["success"] is True and "mock" in r1["note"]
        assert r2["success"] is True and "mock" in r2["note"]
        MC.from_node.assert_not_called()
        RC.from_node.assert_not_called()
