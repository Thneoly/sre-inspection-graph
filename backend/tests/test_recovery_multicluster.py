"""PRD-001 Phase 2 余项 — 跨集群恢复编排测试。

覆盖:
1. `k8s_ref` 返三元组 `(cluster_id, namespace, name)`(替代旧的 `(namespace, name)`)
2. `resolve_cluster_id` 优先 DSS prop、其次 target_id 兜底、kubeconfigs 校验
3. `ensure_kube_loaded` switch-and-reload(同集群幂等,异集群重新 load)
4. `get_k8s_apps_api(cluster_id)` / `get_k8s_core_api(cluster_id)` 接受参数
5. handler 在 real 模式按 target.cluster_id 路由(vm-cluster vs kind-local)
6. `RecoveryExecution.cluster_id` 由 `execute()` 填充
"""

from __future__ import annotations

from unittest.mock import AsyncMock, MagicMock, patch

import pytest

from app.config import settings
from app.datasource.connectors import k8s_client
from app.datasource.models import DataNode
from app.datasource.store import store


@pytest.fixture(autouse=True)
def _seed_two_cluster_targets():
    """两个集群里各一个 Deployment、Pod、Service、Secret、Node。"""
    store.nodes.clear()
    store.edges.clear()
    store.executions.clear()
    nodes = [
        # vm-cluster
        DataNode("deploy:vm-cluster:otel-demo:cart", "Deployment", "cart",
                 {"namespace": "otel-demo", "name": "cart",
                  "cluster_id": "vm-cluster",
                  "desired_replicas": 3, "available_replicas": 3,
                  "current_revision": 2}),
        DataNode("pod:vm-cluster:otel-demo:cart-1", "Pod", "cart-1",
                 {"namespace": "otel-demo", "name": "cart-1",
                  "cluster_id": "vm-cluster",
                  "restart_count": 0}),
        # kind-local
        DataNode("deploy:kind-local:default:nginx", "Deployment", "nginx",
                 {"namespace": "default", "name": "nginx",
                  "cluster_id": "kind-local",
                  "desired_replicas": 2, "available_replicas": 2,
                  "current_revision": 1}),
        DataNode("pod:kind-local:default:nginx-1", "Pod", "nginx-1",
                 {"namespace": "default", "name": "nginx-1",
                  "cluster_id": "kind-local",
                  "restart_count": 0}),
        # 故意构造一个 properties 里没 cluster_id、只能靠 target_id 兜底的节点
        DataNode("deploy:vm-cluster:otel-demo:checkout", "Deployment", "checkout",
                 {"namespace": "otel-demo", "name": "checkout",
                  "desired_replicas": 1, "available_replicas": 1}),
    ]
    for n in nodes:
        store.upsert_node(n)
    k8s_client.reset_loaded_cluster()
    yield
    store.nodes.clear()
    store.edges.clear()
    store.executions.clear()
    k8s_client.reset_loaded_cluster()


# ============================================================
# resolve_cluster_id / k8s_ref
# ============================================================

class TestResolveClusterId:
    def test_prefers_dss_properties(self):
        cid = k8s_client.resolve_cluster_id("deploy:vm-cluster:otel-demo:cart")
        assert cid == "vm-cluster"

    def test_falls_back_to_target_id_second_segment(self):
        # `checkout` 节点没写 cluster_id property → 走 target_id 兜底
        cid = k8s_client.resolve_cluster_id("deploy:vm-cluster:otel-demo:checkout")
        assert cid == "vm-cluster"

    def test_unknown_target_falls_back_to_target_id(self):
        # 节点不在 DSS 也能从 target_id 兜底解析
        cid = k8s_client.resolve_cluster_id("deploy:kind-local:default:nonexistent")
        assert cid == "kind-local"

    def test_unknown_cluster_raises_when_kubeconfigs_configured(self, monkeypatch):
        monkeypatch.setattr(settings, "kubeconfigs", {"vm-cluster": "/tmp/vm"})
        with pytest.raises(ValueError, match="unknown cluster"):
            k8s_client.resolve_cluster_id("deploy:rogue-cluster:default:x")

    def test_kubeconfigs_empty_skips_validation(self, monkeypatch):
        # 测试态默认 kubeconfigs={} → 不校验,任何 cluster 都放行
        monkeypatch.setattr(settings, "kubeconfigs", {})
        assert k8s_client.resolve_cluster_id("deploy:any-cluster:ns:x") == "any-cluster"


class TestK8sRefTriple:
    def test_returns_triple(self):
        cid, ns, name = k8s_client.k8s_ref("deploy:vm-cluster:otel-demo:cart")
        assert cid == "vm-cluster"
        assert ns == "otel-demo"
        assert name == "cart"

    def test_different_cluster(self):
        cid, ns, name = k8s_client.k8s_ref("deploy:kind-local:default:nginx")
        assert cid == "kind-local"
        assert ns == "default"
        assert name == "nginx"


# ============================================================
# ensure_kube_loaded switch-and-reload
# ============================================================

class TestEnsureKubeLoaded:
    def test_same_cluster_idempotent(self, monkeypatch):
        """同一集群连调两次,load_kube_config 只调一次。"""
        import asyncio

        fake_config = MagicMock()
        fake_config.load_kube_config = AsyncMock()
        fake_config.load_incluster_config = MagicMock(side_effect=Exception("not in cluster"))

        with patch.dict("sys.modules", {"kubernetes_asyncio.config": fake_config}):
            with patch("kubernetes_asyncio.config", fake_config):
                asyncio.run(k8s_client.ensure_kube_loaded("vm-cluster"))
                asyncio.run(k8s_client.ensure_kube_loaded("vm-cluster"))
        # 调用应为 1 次(第二次发现 _active_cluster 已是 vm-cluster,直接返回)
        assert fake_config.load_kube_config.await_count == 1
        assert k8s_client.get_active_cluster() == "vm-cluster"

    def test_switch_cluster_reloads(self, monkeypatch):
        """切换集群,load_kube_config 重新调。"""
        import asyncio

        fake_config = MagicMock()
        fake_config.load_kube_config = AsyncMock()
        fake_config.load_incluster_config = MagicMock(side_effect=Exception("not in cluster"))

        with patch("kubernetes_asyncio.config", fake_config):
            asyncio.run(k8s_client.ensure_kube_loaded("vm-cluster"))
            asyncio.run(k8s_client.ensure_kube_loaded("kind-local"))
        assert fake_config.load_kube_config.await_count == 2
        assert k8s_client.get_active_cluster() == "kind-local"


# ============================================================
# get_k8s_apps_api / get_k8s_core_api 接受 cluster_id 参数
# ============================================================

class TestApiFactoriesAcceptClusterId:
    def test_apps_api_signature(self, monkeypatch):
        """get_k8s_apps_api(cluster_id) 调 ensure_kube_loaded(cluster_id)。"""
        import asyncio

        fake_ensure = AsyncMock(return_value="kind-local")
        # ApiClient + AppsV1Api 也得 mock 掉(避免真的初始化 client)
        fake_module = MagicMock()
        fake_module.AppsV1Api = MagicMock()
        fake_api_client_mod = MagicMock()
        fake_api_client_mod.ApiClient = MagicMock

        with patch.object(k8s_client, "ensure_kube_loaded", fake_ensure), \
             patch.dict("sys.modules", {
                 "kubernetes_asyncio": MagicMock(client=fake_module),
                 "kubernetes_asyncio.client": fake_module,
                 "kubernetes_asyncio.client.api_client": fake_api_client_mod,
             }):
            asyncio.run(k8s_client.get_k8s_apps_api("kind-local"))
        fake_ensure.assert_awaited_with("kind-local")

    def test_core_api_signature(self, monkeypatch):
        import asyncio
        fake_ensure = AsyncMock(return_value="vm-cluster")
        fake_module = MagicMock()
        fake_module.CoreV1Api = MagicMock()
        fake_api_client_mod = MagicMock()
        fake_api_client_mod.ApiClient = MagicMock

        with patch.object(k8s_client, "ensure_kube_loaded", fake_ensure), \
             patch.dict("sys.modules", {
                 "kubernetes_asyncio": MagicMock(client=fake_module),
                 "kubernetes_asyncio.client": fake_module,
                 "kubernetes_asyncio.client.api_client": fake_api_client_mod,
             }):
            asyncio.run(k8s_client.get_k8s_core_api("vm-cluster"))
        fake_ensure.assert_awaited_with("vm-cluster")


# ============================================================
# 6 K8s handler 在 real 模式按 cluster_id 路由
# ============================================================

def _make_apis():
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


@pytest.fixture
def real_mode(monkeypatch):
    monkeypatch.setattr(settings, "recovery_handler_mode", "real")
    yield


class TestHandlerClusterRouting:
    def test_scale_deployment_routes_by_target_cluster(self, real_mode):
        """handler 调 get_k8s_apps_api(cluster_id),cluster_id 来自 target.cluster_id。"""
        from app.recovery.handlers import scale_deployment
        api_client, apps, _ = _make_apis()
        captured_cluster = []

        async def fake_get_apps(cluster_id=None):
            captured_cluster.append(cluster_id)
            return api_client, apps

        with patch.object(scale_deployment, "get_k8s_apps_api", side_effect=fake_get_apps):
            r1 = scale_deployment.execute(
                "deploy:kind-local:default:nginx",
                {"replicas_delta": 1}, {"execution_id": "ex1"},
            )
        assert r1["success"] is True
        assert r1["cluster_id"] == "kind-local"
        assert captured_cluster == ["kind-local"]

        with patch.object(scale_deployment, "get_k8s_apps_api", side_effect=fake_get_apps):
            r2 = scale_deployment.execute(
                "deploy:vm-cluster:otel-demo:cart",
                {"replicas_delta": -1}, {"execution_id": "ex2"},
            )
        assert r2["success"] is True
        assert r2["cluster_id"] == "vm-cluster"
        assert captured_cluster == ["kind-local", "vm-cluster"]

    def test_restart_pod_routes_by_cluster(self, real_mode):
        from app.recovery.handlers import restart_pod
        api_client, _, core = _make_apis()
        captured = []

        async def fake_get_core(cluster_id=None):
            captured.append(cluster_id)
            return api_client, core

        with patch.object(restart_pod, "get_k8s_core_api", side_effect=fake_get_core):
            r = restart_pod.execute(
                "pod:kind-local:default:nginx-1",
                {"grace_period_seconds": 10}, {"execution_id": "ex3"},
            )
        assert r["success"] is True
        assert r["cluster_id"] == "kind-local"
        assert captured == ["kind-local"]


# ============================================================
# RecoveryExecution.cluster_id 填充
# ============================================================

class TestExecutionClusterId:
    def test_execute_populates_cluster_id_from_target(self, monkeypatch):
        """execute() 创建 RecoveryExecution 时填 cluster_id。"""
        monkeypatch.setattr(settings, "recovery_handler_mode", "mock")
        from app.recovery.execution import execute

        ex = execute(
            action_id="scale_deployment",
            target_resource_id="deploy:vm-cluster:otel-demo:cart",
            input_params={"replicas_delta": 1},
            initiated_by="alice",
        )
        assert ex.cluster_id == "vm-cluster"
        assert ex.status == "succeeded"

    def test_execute_populates_cluster_id_for_kind_local(self, monkeypatch):
        monkeypatch.setattr(settings, "recovery_handler_mode", "mock")
        from app.recovery.execution import execute

        ex = execute(
            action_id="scale_deployment",
            target_resource_id="deploy:kind-local:default:nginx",
            input_params={"replicas_delta": 2},
            initiated_by="bob",
        )
        assert ex.cluster_id == "kind-local"

    def test_serialize_includes_cluster_id(self, monkeypatch):
        from app.recovery.execution import execute
        from app.routers.recovery import _serialize_execution
        monkeypatch.setattr(settings, "recovery_handler_mode", "mock")

        ex = execute(
            "scale_deployment",
            "deploy:vm-cluster:otel-demo:cart",
            {"replicas_delta": 1},
            initiated_by="alice",
        )
        d = _serialize_execution(ex)
        assert d["cluster_id"] == "vm-cluster"
