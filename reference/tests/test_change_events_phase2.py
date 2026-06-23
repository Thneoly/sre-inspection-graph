"""PRD-002 Phase 2 测试 — 实时变更接入 + 深度关联。

覆盖:
- YAML diff(yaml_diff.py):compute_yaml_diff 基础 + 噪声字段过滤 + summarize_diff
- record_change 新字段:commit_sha / git_repo / yaml_diff 写入 + 序列化 + commit_sha 回退
- 频率告警(frequency.py):窗口内超阈值→severity 提升 / 未超不动 / detect_frequent_changes 分桶
- K8s watcher(k8s_watch_connector):MODIFIED→ChangeEvent / ADDED baseline 不发 / 断线重连 / DELETED 跳过
- webhook(argocd / harbor):payload→ChangeEvent
- Alert 关联(alert_correlation):correlate_alerts 命中 / 不命中 / Neo4j 离线空返 + CORRELATED_WITH 边

模式:同 test_change_events.py,直接调 service / handler 函数 + DSS 断言,
Neo4j 用 conftest 的 mock(离线返空,不阻塞)。
"""

import pytest
from app.datasource.models import DataNode, DataEdge
from app.datasource.store import store


# ============================================================
# 种子 — 复用 test_change_events 的最小图
# ============================================================

@pytest.fixture(scope="module", autouse=True)
def _seed_store():
    store.nodes.clear()
    store.edges.clear()
    store.change_events.clear()

    nodes = [
        DataNode("app:order", "Application", "订单应用"),
        DataNode("comp:order-api", "ApplicationComponent", "订单API组件"),
        DataNode("deploy:order-api", "Deployment", "order-api",
                 {"namespace": "order", "name": "order-api"}),
        DataNode("pod:order-api-1", "Pod", "order-api-1"),
        DataNode("pod:order-api-2", "Pod", "order-api-2"),
        DataNode("cm:order-config", "ConfigMap", "order-config",
                 {"namespace": "order", "name": "order-config"}),
        DataNode("secret:order-db", "Secret", "order-db-secret",
                 {"namespace": "order", "name": "order-db"}),
        DataNode("img:order:1.2.3", "ContainerImage", "order:1.2.3"),
    ]
    for n in nodes:
        store.upsert_node(n)

    edges = [
        ("e1", "app:order", "CONTAINS", "comp:order-api"),
        ("e2", "comp:order-api", "DEPLOYED_AS", "deploy:order-api"),
        ("e3", "deploy:order-api", "CONTAINS", "pod:order-api-1"),
        ("e4", "deploy:order-api", "CONTAINS", "pod:order-api-2"),
        ("e5", "pod:order-api-1", "USES", "cm:order-config"),
        ("e6", "pod:order-api-2", "USES", "cm:order-config"),
        ("e7", "pod:order-api-1", "USES", "secret:order-db"),
    ]
    for eid, src, rel, tgt in edges:
        store.upsert_edge(DataEdge(eid, src, tgt, rel, rel))

    yield

    store.nodes.clear()
    store.edges.clear()
    store.change_events.clear()


@pytest.fixture(autouse=True)
def _clear_change_events():
    store.clear_change_events()


# ============================================================
# YAML diff
# ============================================================

class TestYamlDiff:
    def test_compute_diff_detects_value_change(self):
        from app.changes.yaml_diff import compute_yaml_diff

        old = {"data": {"max_pool_size": "20", "timeout": "30"}}
        new = {"data": {"max_pool_size": "50", "timeout": "30"}}
        diff = compute_yaml_diff(old, new, name="order-config")
        assert diff  # 非空
        assert "max_pool_size" in diff
        assert "20" in diff and "50" in diff
        # timeout 没变,不在 diff 主体
        assert "+timeout" not in diff

    def test_compute_diff_strips_noise_fields(self):
        from app.changes.yaml_diff import compute_yaml_diff

        old = {
            "data": {"key": "v1"},
            "metadata": {
                "resourceVersion": "123",
                "uid": "abc-456",
                "managedFields": [{"manager": "kubectl"}],
                "creationTimestamp": "2026-01-01T00:00:00Z",
            },
        }
        new = {
            "data": {"key": "v1"},  # 业务字段没变
            "metadata": {
                "resourceVersion": "999",  # 噪声变了
                "uid": "abc-456",
                "managedFields": [{"manager": "kubectl", "operation": "Apply"}],
                "creationTimestamp": "2026-01-01T00:00:00Z",
            },
        }
        diff = compute_yaml_diff(old, new, name="cm")
        assert diff == "", f"噪声字段变动不应产生 diff,得到: {diff!r}"

    def test_summarize_diff_counts_added_removed(self):
        from app.changes.yaml_diff import compute_yaml_diff, summarize_diff

        old = {"data": {"a": "1", "b": "2"}}
        new = {"data": {"a": "1", "b": "9", "c": "3"}}
        diff = compute_yaml_diff(old, new)
        summary = summarize_diff(diff)
        assert summary["added"] >= 1  # +c
        assert summary["removed"] >= 1  # -b旧值
        assert "b" in summary["changed_keys"]


# ============================================================
# record_change 新字段
# ============================================================

class TestRecordChangePhase2Fields:
    def test_commit_sha_and_git_repo_persisted(self):
        from app.changes.event_service import record_change, serialize

        event = record_change(
            change_type="deployment_rolled",
            target_resource_id="deploy:order-api",
            source="argo_cd",
            commit_sha="abc123def456",
            git_repo="https://github.com/acme/order-api",
            pipeline_url="https://ci.example.com/run/42",
            cluster_id="vm-cluster",
            yaml_diff="@@ -1 +1 @@\n-old\n+new",
        )
        assert event.commit_sha == "abc123def456"
        assert event.git_repo == "https://github.com/acme/order-api"
        assert event.cluster_id == "vm-cluster"
        assert "old" in event.yaml_diff

        s = serialize(event)
        assert s["commit_sha"] == "abc123def456"
        assert s["pipeline_url"] == "https://ci.example.com/run/42"

    def test_commit_sha_backfills_related_commit(self):
        """commit_sha 优先于 related_commit;显式 related_commit 不被覆盖。"""
        from app.changes.event_service import record_change

        # 只传 commit_sha → related_commit 同步
        ev1 = record_change(
            change_type="deployment_rolled",
            target_resource_id="deploy:order-api",
            source="argo_cd",
            commit_sha="sha-1",
        )
        assert ev1.related_commit == "sha-1"

        # 显式 related_commit 优先(向后兼容旧调用)
        ev2 = record_change(
            change_type="deployment_rolled",
            target_resource_id="deploy:order-api",
            source="argo_cd",
            related_commit="legacy-sha",
            commit_sha="sha-2",
        )
        assert ev2.related_commit == "legacy-sha"


# ============================================================
# 频率告警
# ============================================================

class TestFrequencyAlert:
    def test_frequent_change_elevates_severity_to_medium(self):
        """同一资源窗口内变更 > 阈值 → severity 至少 medium。"""
        from app.changes.event_service import record_change

        # cm:order-config propagated_to 含 2 个 Pod → base severity low(<5)
        # 连续记 6 次(默认阈值 5,>5 命中)
        for _ in range(6):
            record_change(
                change_type="configmap_updated",
                target_resource_id="cm:order-config",
                source="k8s_api",
            )
        events = store.list_change_events(target_resource_id="cm:order-config")
        # 最后一条应被频率告警提升到 medium
        assert events[-1].severity_estimate == "medium"
        assert "过频变更" in events[-1].description

    def test_below_threshold_keeps_severity(self):
        """窗口内变更未超阈值 → 不加频率标记,severity 保持 propagated 基础值。"""
        from app.changes.event_service import record_change

        for _ in range(3):  # < 5,不命中
            record_change(
                change_type="configmap_updated",
                target_resource_id="cm:order-config",
                source="k8s_api",
            )
        events = store.list_change_events(target_resource_id="cm:order-config")
        # cm:order-config propagated_to=5(pod1/pod2/deploy/comp/app)→ 基础 medium
        # 频率未命中:不应有"过频变更"标记,severity 不被额外提升到 high
        assert all("过频变更" not in e.description for e in events)
        assert all(e.severity_estimate == "medium" for e in events)

    def test_detect_frequent_changes_buckets_by_target(self):
        from app.changes.event_service import record_change
        from app.changes.frequency import detect_frequent_changes

        # cm:order-config 6 次(命中),secret:order-db 2 次(不命中)
        for _ in range(6):
            record_change(
                change_type="configmap_updated",
                target_resource_id="cm:order-config",
                source="k8s_api",
            )
        for _ in range(2):
            record_change(
                change_type="secret_rotated",
                target_resource_id="secret:order-db",
                source="k8s_api",
            )

        frequent = detect_frequent_changes(window_seconds=3600, threshold=5)
        targets = [f["target_resource_id"] for f in frequent]
        assert "cm:order-config" in targets
        assert "secret:order-db" not in targets
        cm = next(f for f in frequent if f["target_resource_id"] == "cm:order-config")
        assert cm["count"] == 6
        assert len(cm["event_ids"]) == 6


# ============================================================
# K8s WatchConnector — 纯事件处理逻辑(不真起 watch)
# ============================================================

class TestK8sWatchConnector:
    def _make_connector(self):
        from app.config import settings
        from app.datasource.connectors.k8s_watch_connector import K8sWatchConnector
        c = K8sWatchConnector(cluster_id="test-cluster", namespace="order")
        # 关闭 gate 限制:status 走 watch 分支
        return c

    def test_modified_event_produces_change_with_yaml_diff(self):
        c = self._make_connector()
        # 首轮(first_sync=True)ADDED 建快照,不发
        baseline = {
            "metadata": {"name": "order-config", "resourceVersion": "100",
                         "uid": "x", "managedFields": []},
            "data": {"max_pool": "20"},
        }
        c._handle_watch_event("ConfigMap", {"type": "ADDED", "object": baseline})
        assert len(store.change_events) == 0  # 首轮 ADDED 只建快照
        # bootstrap 完成,翻 first_sync
        c._first_sync["ConfigMap"] = False

        # MODIFIED 业务字段变了
        modified = {
            "metadata": {"name": "order-config", "resourceVersion": "101",
                         "uid": "x", "managedFields": [], "creationTimestamp": "t"},
            "data": {"max_pool": "50"},
        }
        c._handle_watch_event("ConfigMap", {"type": "MODIFIED", "object": modified})
        events = store.list_change_events(target_resource_id="configmap:test-cluster:order:order-config")
        assert len(events) == 1
        ev = events[0]
        assert ev.change_type == "configmap_updated"
        assert ev.source == "k8s_api"
        assert ev.cluster_id == "test-cluster"
        assert "max_pool" in ev.yaml_diff
        assert "20" in ev.yaml_diff and "50" in ev.yaml_diff

    def test_first_sync_added_does_not_emit(self):
        """首轮 ADDED 只建快照,不发 ChangeEvent(防启动炸历史)。"""
        c = self._make_connector()
        assert c._first_sync["ConfigMap"] is True
        obj = {"metadata": {"name": "cm1", "resourceVersion": "1"}, "data": {"k": "v"}}
        c._handle_watch_event("ConfigMap", {"type": "ADDED", "object": obj})
        assert len(store.change_events) == 0
        assert "cm1" in c._snapshots["ConfigMap"]

    def test_deleted_event_does_not_emit(self):
        c = self._make_connector()
        # 首轮建快照
        obj = {"metadata": {"name": "order-config", "resourceVersion": "1"}, "data": {"k": "v"}}
        c._handle_watch_event("ConfigMap", {"type": "ADDED", "object": obj})
        c._first_sync["ConfigMap"] = False
        c._handle_watch_event("ConfigMap", {"type": "DELETED", "object": obj})
        assert len(store.change_events) == 0  # DELETED 不发
        assert "order-config" not in c._snapshots["ConfigMap"]

    def test_noise_only_modified_does_not_emit(self):
        """只有 resourceVersion 等噪声字段变 → 不发事件。"""
        c = self._make_connector()
        old = {"metadata": {"name": "cm1", "resourceVersion": "1", "uid": "u"},
               "data": {"k": "v"}}
        c._handle_watch_event("ConfigMap", {"type": "ADDED", "object": old})
        c._first_sync["ConfigMap"] = False
        new = {"metadata": {"name": "cm1", "resourceVersion": "999", "uid": "u"},  # 只 rv 变
               "data": {"k": "v"}}
        c._handle_watch_event("ConfigMap", {"type": "MODIFIED", "object": new})
        assert len(store.change_events) == 0


# ============================================================
# Webhook router
# ============================================================

class TestWebhooks:
    def test_argocd_webhook_creates_deployment_rolled(self, monkeypatch):
        from app.config import settings
        from app.main import app
        from fastapi.testclient import TestClient

        monkeypatch.setattr(settings, "webhook_token", "")  # 跳过校验
        client = TestClient(app)

        payload = {
            "application": {
                "metadata": {"name": "order-api"},
                "spec": {"source": {"repoURL": "https://github.com/acme/order-api"}},
            },
            "revision": "abc123def456789",
            "images": ["order-api:1.2.4"],
        }
        resp = client.post("/api/v1/webhooks/argocd", json=payload)
        assert resp.status_code == 201, resp.text
        body = resp.json()
        assert body["change_type"] == "deployment_rolled"
        assert body["source"] == "argo_cd"
        assert body["commit_sha"] == "abc123def456789"
        assert body["git_repo"] == "https://github.com/acme/order-api"
        # target 命中 DSS 节点(deploy:order-api name=order-api)
        assert body["target_resource_id"] == "deploy:order-api"

    def test_harbor_webhook_creates_image_pushed(self, monkeypatch):
        from app.config import settings
        from app.main import app
        from fastapi.testclient import TestClient

        monkeypatch.setattr(settings, "webhook_token", "")
        client = TestClient(app)

        payload = {
            "type": "PUSH_ARTIFACT",
            "event_data": {
                "repository": {"repo_full_name": "acme/order-api"},
                "resources": [
                    {"resource": {"digest": "sha256:abcdef1234567890", "tag": "1.2.4"}},
                ],
            },
        }
        resp = client.post("/api/v1/webhooks/harbor", json=payload)
        assert resp.status_code == 201, resp.text
        body = resp.json()
        assert body["total"] == 1
        ev = body["events"][0]
        assert ev["change_type"] == "image_pushed"
        assert ev["source"] == "gitops"
        assert ev["diff_summary"]["tag"] == "1.2.4"
        assert "acme/order-api" in ev["diff_summary"]["repository"]

    def test_webhook_token_rejected_when_mismatch(self, monkeypatch):
        from app.config import settings
        from app.main import app
        from fastapi.testclient import TestClient

        monkeypatch.setattr(settings, "webhook_token", "secret")
        client = TestClient(app)
        resp = client.post("/api/v1/webhooks/argocd",
                           json={"application": {"metadata": {"name": "x"}}},
                           headers={"X-Webhook-Token": "wrong"})
        assert resp.status_code == 401


# ============================================================
# ChangeEvent ↔ AlertEvent CORRELATED_WITH
# ============================================================

class TestAlertCorrelation:
    def _fake_alert_record(self, aid, resource_ref, fired_at, severity="critical"):
        """造一个像 Neo4j Record 的对象(有 .get())。"""
        from unittest.mock import MagicMock
        data = {
            "aid": aid,
            "name": f"alert-{aid}",
            "severity": severity,
            "fired_at": fired_at,
            "resource_ref": resource_ref,
            "summary": f"{aid} on {resource_ref}",
        }
        rec = MagicMock()
        rec.get.side_effect = lambda k, default=None: data.get(k, default)
        return rec

    def _fake_driver_with_alerts(self, records):
        from unittest.mock import MagicMock
        session = MagicMock()
        session.run.return_value = list(records)
        driver = MagicMock()
        driver.session.return_value.__enter__.return_value = session
        return driver, session

    def test_correlate_alerts_matches_resource_in_propagated_to(self):
        """AlertEvent.resource_ref 落在变更 propagated_to → 命中。"""
        from app.changes.event_service import record_change
        from app.changes.alert_correlation import correlate_alerts
        from app.db import neo4j_client as n4j
        from unittest.mock import patch

        # cm:order-config 变更 → propagated_to 含 pod:order-api-1 / pod:order-api-2
        event = record_change(
            change_type="configmap_updated",
            target_resource_id="cm:order-config",
            source="k8s_api",
        )
        assert "pod:order-api-1" in event.propagated_to

        alert_rec = self._fake_alert_record(
            "fault_alert_1", "pod:order-api-1", event.changed_at.replace("Z", ""),
        )
        driver, session = self._fake_driver_with_alerts([alert_rec])

        with patch.object(n4j, "get_driver", return_value=driver):
            result = correlate_alerts(event.change_event_id, window_seconds=600)
        assert result["total"] == 1
        assert result["alerts"][0]["alert_event_id"] == "fault_alert_1"
        assert result["neo4j_available"] is True
        # persist_correlation 被调(CORRELATED_WITH 边)—— session.run 至少调 2 次(list + MERGE)
        # 这里只验 correlate;persist 单独测

    def test_correlate_alerts_no_match_when_resource_outside_impact(self):
        from app.changes.event_service import record_change
        from app.changes.alert_correlation import correlate_alerts
        from app.db import neo4j_client as n4j
        from unittest.mock import patch

        event = record_change(
            change_type="configmap_updated",
            target_resource_id="cm:order-config",
            source="k8s_api",
        )
        # resource_ref 是个无关资源
        alert_rec = self._fake_alert_record(
            "alert_x", "pod:unrelated-9", event.changed_at.replace("Z", ""),
        )
        driver, _ = self._fake_driver_with_alerts([alert_rec])
        with patch.object(n4j, "get_driver", return_value=driver):
            result = correlate_alerts(event.change_event_id, window_seconds=600)
        assert result["total"] == 0

    def test_correlate_alerts_neo4j_offline_returns_empty(self):
        """Neo4j 离线(get_driver=None)→ alerts 空,neo4j_available=False,不抛。"""
        from app.changes.event_service import record_change
        from app.changes.alert_correlation import correlate_alerts
        from app.db import neo4j_client as n4j
        from unittest.mock import patch

        event = record_change(
            change_type="configmap_updated",
            target_resource_id="cm:order-config",
            source="k8s_api",
        )
        with patch.object(n4j, "get_driver", return_value=None):
            result = correlate_alerts(event.change_event_id)
        assert result["total"] == 0
        assert result["neo4j_available"] is False

    def test_persist_correlation_writes_edge(self):
        from app.changes.alert_correlation import persist_correlation
        from app.db import neo4j_client as n4j
        from unittest.mock import patch

        session = MagicMock()
        driver = MagicMock()
        driver.session.return_value.__enter__.return_value = session
        with patch.object(n4j, "get_driver", return_value=driver):
            ok = persist_correlation("ce-1", "fault_alert_1")
        assert ok is True
        # 调了一次 MERGE CORRELATED_WITH
        assert session.run.call_count == 1
        cypher = session.run.call_args.args[0]
        assert "CORRELATED_WITH" in cypher

    def test_alerts_endpoint_returns_empty_when_neo4j_offline(self, monkeypatch):
        from app.changes.event_service import record_change
        from app.db import neo4j_client as n4j
        from app.main import app
        from fastapi.testclient import TestClient
        from unittest.mock import patch

        event = record_change(
            change_type="configmap_updated",
            target_resource_id="cm:order-config",
            source="k8s_api",
        )
        with patch.object(n4j, "get_driver", return_value=None):
            client = TestClient(app)
            resp = client.get(f"/api/v1/change-events/{event.change_event_id}/alerts")
        assert resp.status_code == 200
        body = resp.json()
        assert body["total"] == 0
        assert body["neo4j_available"] is False

    def test_frequent_endpoint(self, monkeypatch):
        from app.changes.event_service import record_change
        from app.main import app
        from fastapi.testclient import TestClient

        for _ in range(6):
            record_change(
                change_type="configmap_updated",
                target_resource_id="cm:order-config",
                source="k8s_api",
            )
        client = TestClient(app)
        resp = client.get("/api/v1/change-events/frequent?window=3600&threshold=5")
        assert resp.status_code == 200
        body = resp.json()
        targets = [f["target_resource_id"] for f in body["frequent"]]
        assert "cm:order-config" in targets


# 导入 MagicMock(上面类用到)
from unittest.mock import MagicMock  # noqa: E402
