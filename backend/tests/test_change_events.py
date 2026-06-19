"""ChangeEvent 测试 — PRD-002 Sprint 1。

覆盖:
- ChangeEvent dataclass 默认值
- DSS Store 增/查/过滤
- 影响范围 BFS(propagation.py)
- record_change() 业务编排(propagated_to + severity)
- correlated_changes() 时间窗口 + direct/propagated 双匹配
- 6 个 HTTP endpoint

测试种子:复用 test_recovery_execute.py 的 _seed_store 风格。
"""

import pytest
from app.datasource.models import ChangeEvent, DataNode, DataEdge
from app.datasource.store import store


# ============================================================
# 种子数据 — 一个最小图,覆盖 ConfigMap → Pod 链路
# ============================================================

@pytest.fixture(scope="module", autouse=True)
def _seed_store():
    store.nodes.clear()
    store.edges.clear()
    store.change_events.clear()

    nodes = [
        DataNode("app:order", "Application", "订单应用"),
        DataNode("comp:order-api", "ApplicationComponent", "订单API组件"),
        DataNode("deploy:order-api", "Deployment", "order-api"),
        DataNode("pod:order-api-1", "Pod", "order-api-1"),
        DataNode("pod:order-api-2", "Pod", "order-api-2"),
        DataNode("cm:order-config", "ConfigMap", "order-config"),
        DataNode("secret:order-db", "Secret", "order-db-secret"),
        DataNode("svc:order-api", "Service", "order-api-svc"),
        DataNode("img:order:1.2.3", "ContainerImage", "order:1.2.3"),
        DataNode("orphan:lonely", "ConfigMap", "lonely-cm"),  # 无依赖
    ]
    for n in nodes:
        store.upsert_node(n)

    edges = [
        # 应用结构链 — 项目约定:parent -CONTAINS-> child / comp -DEPLOYED_AS-> deploy
        ("e1", "app:order", "CONTAINS", "comp:order-api"),
        ("e2", "comp:order-api", "DEPLOYED_AS", "deploy:order-api"),
        ("e3", "deploy:order-api", "CONTAINS", "pod:order-api-1"),
        ("e4", "deploy:order-api", "CONTAINS", "pod:order-api-2"),
        # Pod 直接 USES ConfigMap / Secret(对应 K8s pod-level volumeMount/envFrom)
        ("e5", "pod:order-api-1", "USES", "cm:order-config"),
        ("e6", "pod:order-api-2", "USES", "cm:order-config"),
        ("e7", "pod:order-api-1", "USES", "secret:order-db"),
        ("e8", "pod:order-api-2", "USES", "secret:order-db"),
        # Service 暴露 Pod
        ("e9", "svc:order-api", "ROUTES_TO", "pod:order-api-1"),
        # USES_IMAGE 不在 PROPAGATION_EDGES 白名单,用于测试不跨非白名单边
        ("e10", "deploy:order-api", "USES_IMAGE", "img:order:1.2.3"),
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
# 1. dataclass
# ============================================================

class TestChangeEventModel:
    def test_defaults(self):
        e = ChangeEvent(
            change_event_id="ce-1",
            change_type="configmap_updated",
            target_resource_id="cm:foo",
            target_resource_type="ConfigMap",
            changed_at="2026-06-19T03:00:00Z",
        )
        assert e.changed_by == ""
        assert e.source == "manual"
        assert e.severity_estimate == "low"
        assert e.propagated_to == []
        assert e.diff_summary == {}

    def test_diff_summary_preserved(self):
        diff = {"max_pool_size": {"old": 20, "new": 50}, "host": {"old": "a", "new": "b"}}
        e = ChangeEvent(
            change_event_id="ce-2",
            change_type="configmap_updated",
            target_resource_id="cm:foo",
            target_resource_type="ConfigMap",
            changed_at="2026-06-19T03:00:00Z",
            diff_summary=diff,
        )
        assert e.diff_summary["max_pool_size"]["old"] == 20
        assert e.diff_summary["host"]["new"] == "b"


# ============================================================
# 2. DSS store
# ============================================================

class TestStore:
    def test_add_and_get(self):
        e = ChangeEvent(
            change_event_id="ce-store-1",
            change_type="configmap_updated",
            target_resource_id="cm:order-config",
            target_resource_type="ConfigMap",
            changed_at="2026-06-19T03:00:00Z",
        )
        store.add_change_event(e)
        got = store.get_change_event("ce-store-1")
        assert got is e

    def test_filter_by_type(self):
        store.add_change_event(ChangeEvent("ce-a", "configmap_updated", "cm:order-config",
                                           "ConfigMap", "2026-06-19T03:00:00Z"))
        store.add_change_event(ChangeEvent("ce-b", "secret_rotated", "secret:order-db",
                                           "Secret", "2026-06-19T03:01:00Z"))
        cm_events = store.list_change_events(change_type="configmap_updated")
        assert len(cm_events) == 1
        assert cm_events[0].change_event_id == "ce-a"

    def test_filter_by_target(self):
        store.add_change_event(ChangeEvent("ce-c", "configmap_updated", "cm:order-config",
                                           "ConfigMap", "2026-06-19T03:00:00Z"))
        store.add_change_event(ChangeEvent("ce-d", "configmap_updated", "cm:other",
                                           "ConfigMap", "2026-06-19T03:00:00Z"))
        hits = store.list_change_events(target_resource_id="cm:order-config")
        assert len(hits) == 1
        assert hits[0].change_event_id == "ce-c"

    def test_filter_by_time_window(self):
        store.add_change_event(ChangeEvent("ce-old", "configmap_updated", "cm:x", "ConfigMap",
                                           "2026-06-18T00:00:00Z"))
        store.add_change_event(ChangeEvent("ce-mid", "configmap_updated", "cm:x", "ConfigMap",
                                           "2026-06-19T03:00:00Z"))
        store.add_change_event(ChangeEvent("ce-new", "configmap_updated", "cm:x", "ConfigMap",
                                           "2026-06-19T05:00:00Z"))
        hits = store.list_change_events(
            since="2026-06-19T00:00:00Z",
            until="2026-06-19T04:00:00Z",
        )
        ids = {e.change_event_id for e in hits}
        assert ids == {"ce-mid"}


# ============================================================
# 3. propagation BFS
# ============================================================

class TestPropagation:
    def test_configmap_to_pods_one_hop(self):
        from app.changes.propagation import derive_propagation
        # cm:order-config 反向走 USES 命中 2 个 Pod
        propagated = derive_propagation("cm:order-config")
        assert "pod:order-api-1" in propagated
        assert "pod:order-api-2" in propagated

    def test_secret_to_application_multi_hop(self):
        from app.changes.propagation import derive_propagation
        # secret:order-db 反向走:
        #   secret <-USES- pods (depth 1)
        #   pods <-CONTAINS- deploy (depth 2)
        #   deploy <-DEPLOYED_AS- comp (depth 3)
        #   comp <-CONTAINS- app (depth 4)
        propagated = derive_propagation("secret:order-db")
        assert "pod:order-api-1" in propagated
        assert "pod:order-api-2" in propagated
        assert "deploy:order-api" in propagated
        assert "comp:order-api" in propagated
        assert "app:order" in propagated

    def test_orphan_no_propagation(self):
        from app.changes.propagation import derive_propagation
        propagated = derive_propagation("orphan:lonely")
        assert propagated == []

    def test_max_depth_cap(self):
        from app.changes.propagation import derive_propagation
        # depth=1 from secret 只能命中 pod-1/pod-2(1 跳)不能继续往 deploy/comp/app
        propagated = derive_propagation("secret:order-db", max_depth=1)
        assert sorted(propagated) == ["pod:order-api-1", "pod:order-api-2"]

    def test_non_propagation_edge_skipped(self):
        from app.changes.propagation import derive_propagation
        # img:order:1.2.3 只有 USES_IMAGE 这条边过来,不在 PROPAGATION_EDGES 里
        propagated = derive_propagation("img:order:1.2.3")
        assert propagated == []

    def test_unknown_target(self):
        from app.changes.propagation import derive_propagation
        assert derive_propagation("cm:does-not-exist") == []

    def test_propagation_path(self):
        from app.changes.propagation import find_propagation_path
        path = find_propagation_path("secret:order-db", "app:order")
        # secret <- pod-1 <- deploy <- comp <- app 的反向 BFS 最短路径
        # 起点是 secret,终点是 app
        assert path[0] == "secret:order-db"
        assert path[-1] == "app:order"
        assert "deploy:order-api" in path
        assert "comp:order-api" in path


# ============================================================
# 4. record_change
# ============================================================

class TestRecordChange:
    def test_basic_record(self):
        from app.changes.event_service import record_change
        ev = record_change(
            change_type="configmap_updated",
            target_resource_id="cm:order-config",
            changed_by="alice@x",
            source="manual",
            description="池大小 20 → 50",
            diff_summary={"max_pool_size": {"old": 20, "new": 50}},
        )
        assert ev.change_event_id.startswith("ce-")
        assert ev.target_resource_type == "ConfigMap"
        # propagated_to 应包含 2 个 pod
        assert "pod:order-api-1" in ev.propagated_to
        assert "pod:order-api-2" in ev.propagated_to

    def test_severity_low_medium_high(self):
        from app.changes.event_service import _estimate_severity
        # 边界值:0-4=low, 5-9=medium, 10+=high
        assert _estimate_severity(0) == "low"
        assert _estimate_severity(4) == "low"
        assert _estimate_severity(5) == "medium"
        assert _estimate_severity(9) == "medium"
        assert _estimate_severity(10) == "high"
        assert _estimate_severity(50) == "high"

    def test_severity_from_real_propagation(self):
        from app.changes.event_service import record_change
        # cm:order-config 反向命中 pods + deploy + comp + app = 5 → medium
        ev = record_change("configmap_updated", "cm:order-config")
        assert len(ev.propagated_to) >= 5
        assert ev.severity_estimate == "medium"
        # orphan:lonely 没下游 → low
        ev2 = record_change("configmap_updated", "orphan:lonely")
        assert ev2.propagated_to == []
        assert ev2.severity_estimate == "low"

    def test_target_not_in_dss(self):
        from app.changes.event_service import record_change
        # PRD: target 不存在仍记录,propagated_to 为空
        ev = record_change(
            change_type="configmap_updated",
            target_resource_id="cm:does-not-exist",
        )
        assert ev.propagated_to == []
        assert ev.target_resource_type == ""
        assert ev.severity_estimate == "low"

    def test_invalid_change_type(self):
        from app.changes.event_service import record_change, ChangeEventError
        with pytest.raises(ChangeEventError):
            record_change(change_type="bogus_type", target_resource_id="cm:order-config")

    def test_invalid_source(self):
        from app.changes.event_service import record_change, ChangeEventError
        with pytest.raises(ChangeEventError):
            record_change(
                change_type="configmap_updated",
                target_resource_id="cm:order-config",
                source="weird",
            )


# ============================================================
# 5. correlated_changes
# ============================================================

class TestCorrelatedQuery:
    def test_direct_match(self):
        from app.changes.event_service import record_change, correlated_changes
        record_change("configmap_updated", "cm:order-config",
                      changed_at="2026-06-19T03:00:00Z")
        result = correlated_changes(
            target_resource_id="cm:order-config",
            since="2026-06-19T02:55:00Z",
            until="2026-06-19T03:05:00Z",
        )
        assert result["total"] == 1
        assert result["changes"][0]["match_type"] == "direct"

    def test_propagated_match(self):
        from app.changes.event_service import record_change, correlated_changes
        # 录入 ConfigMap 变更 → 用 Pod 反查
        record_change("configmap_updated", "cm:order-config",
                      changed_at="2026-06-19T03:00:00Z")
        result = correlated_changes(
            target_resource_id="pod:order-api-1",
            since="2026-06-19T02:55:00Z",
            until="2026-06-19T03:05:00Z",
        )
        assert result["total"] == 1
        assert result["changes"][0]["match_type"] == "propagated"
        assert result["changes"][0]["propagation_distance"] >= 1

    def test_window_excludes_old(self):
        from app.changes.event_service import record_change, correlated_changes
        record_change("configmap_updated", "cm:order-config",
                      changed_at="2026-06-19T01:00:00Z")
        result = correlated_changes(
            target_resource_id="cm:order-config",
            since="2026-06-19T02:55:00Z",
            until="2026-06-19T03:05:00Z",
        )
        assert result["total"] == 0

    def test_default_window(self):
        from app.changes.event_service import record_change, correlated_changes
        # 默认 window=300,since/until 不给 → [now-300, now]。一个很久的事件不该命中
        record_change("configmap_updated", "cm:order-config",
                      changed_at="2020-01-01T00:00:00Z")
        result = correlated_changes(target_resource_id="cm:order-config")
        # 默认窗口里没有事件
        assert result["total"] == 0
        assert "window_start" in result
        assert "window_end" in result

    def test_include_propagated_false(self):
        from app.changes.event_service import record_change, correlated_changes
        record_change("configmap_updated", "cm:order-config",
                      changed_at="2026-06-19T03:00:00Z")
        result = correlated_changes(
            target_resource_id="pod:order-api-1",
            since="2026-06-19T02:55:00Z",
            until="2026-06-19T03:05:00Z",
            include_propagated=False,
        )
        # Pod 不是变更的直接对象,关闭传播匹配后应空
        assert result["total"] == 0

    def test_sorted_desc(self):
        from app.changes.event_service import record_change, correlated_changes
        record_change("configmap_updated", "cm:order-config",
                      changed_at="2026-06-19T03:00:00Z")
        record_change("configmap_updated", "cm:order-config",
                      changed_at="2026-06-19T03:02:00Z")
        record_change("configmap_updated", "cm:order-config",
                      changed_at="2026-06-19T03:01:00Z")
        result = correlated_changes(
            target_resource_id="cm:order-config",
            since="2026-06-19T02:55:00Z",
            until="2026-06-19T03:05:00Z",
        )
        ts = [c["changed_at"] for c in result["changes"]]
        assert ts == sorted(ts, reverse=True)


# ============================================================
# 6. impact + timeline
# ============================================================

class TestImpactAndTimeline:
    def test_impact_returns_paths(self):
        from app.changes.event_service import record_change, get_impact
        ev = record_change("secret_rotated", "secret:order-db")
        impact = get_impact(ev.change_event_id)
        assert impact["affected_count"] == len(ev.propagated_to)
        # 每个 affected 应有 path 且第一节点是 secret
        for entry in impact["affected"]:
            assert entry["path"][0] == "secret:order-db"
            assert entry["path"][-1] == entry["resource_id"]

    def test_impact_unknown_event(self):
        from app.changes.event_service import get_impact, ChangeEventError
        with pytest.raises(ChangeEventError) as exc:
            get_impact("ce-does-not-exist")
        assert exc.value.code == 404

    def test_application_timeline(self):
        from app.changes.event_service import record_change, application_timeline
        record_change("configmap_updated", "cm:order-config",
                      changed_at="2026-06-19T03:00:00Z")
        record_change("deployment_rolled", "deploy:order-api",
                      changed_at="2026-06-19T03:05:00Z")
        record_change("configmap_updated", "orphan:lonely",
                      changed_at="2026-06-19T03:10:00Z")  # 不属于 app:order
        timeline = application_timeline("app:order")
        ids = {e["target_resource_id"] for e in timeline["events"]}
        assert "cm:order-config" in ids
        assert "deploy:order-api" in ids
        assert "orphan:lonely" not in ids
        assert timeline["by_type"]["configmap_updated"] == 1
        assert timeline["by_type"]["deployment_rolled"] == 1

    def test_application_timeline_unknown(self):
        from app.changes.event_service import application_timeline, ChangeEventError
        with pytest.raises(ChangeEventError) as exc:
            application_timeline("app:does-not-exist")
        assert exc.value.code == 404


# ============================================================
# 7. HTTP endpoints
# ============================================================

class TestEndpoints:
    def test_post_create(self, client):
        c, _ = client
        resp = c.post("/api/v1/change-events", json={
            "change_type": "configmap_updated",
            "target_resource_id": "cm:order-config",
            "changed_by": "alice@e2e",
            "description": "raise pool size",
            "diff_summary": {"max_pool_size": {"old": 20, "new": 50}},
        })
        assert resp.status_code == 201
        body = resp.json()
        assert body["target_resource_type"] == "ConfigMap"
        assert "pod:order-api-1" in body["propagated_to"]

    def test_post_invalid_type(self, client):
        c, _ = client
        resp = c.post("/api/v1/change-events", json={
            "change_type": "weird_type",
            "target_resource_id": "cm:order-config",
        })
        assert resp.status_code == 400

    def test_get_list_filter(self, client):
        c, _ = client
        c.post("/api/v1/change-events", json={
            "change_type": "configmap_updated",
            "target_resource_id": "cm:order-config",
            "changed_at": "2026-06-19T03:00:00Z",
        })
        c.post("/api/v1/change-events", json={
            "change_type": "secret_rotated",
            "target_resource_id": "secret:order-db",
            "changed_at": "2026-06-19T03:01:00Z",
        })
        resp = c.get("/api/v1/change-events?change_type=configmap_updated")
        assert resp.status_code == 200
        body = resp.json()
        assert body["total"] == 1
        assert body["events"][0]["change_type"] == "configmap_updated"

    def test_get_one_404(self, client):
        c, _ = client
        resp = c.get("/api/v1/change-events/ce-nope")
        assert resp.status_code == 404

    def test_get_correlated(self, client):
        c, _ = client
        c.post("/api/v1/change-events", json={
            "change_type": "configmap_updated",
            "target_resource_id": "cm:order-config",
            "changed_at": "2026-06-19T03:00:00Z",
        })
        resp = c.get(
            "/api/v1/change-events/correlated"
            "?target_resource_id=pod:order-api-1"
            "&since=2026-06-19T02:55:00Z"
            "&until=2026-06-19T03:05:00Z"
        )
        assert resp.status_code == 200
        body = resp.json()
        assert body["total"] == 1
        assert body["changes"][0]["match_type"] == "propagated"

    def test_get_impact(self, client):
        c, _ = client
        post_resp = c.post("/api/v1/change-events", json={
            "change_type": "secret_rotated",
            "target_resource_id": "secret:order-db",
        })
        eid = post_resp.json()["change_event_id"]
        resp = c.get(f"/api/v1/change-events/{eid}/impact")
        assert resp.status_code == 200
        body = resp.json()
        assert body["affected_count"] >= 5
        # path 起点是 secret 自身
        assert body["affected"][0]["path"][0] == "secret:order-db"

    def test_get_timeline(self, client):
        c, _ = client
        c.post("/api/v1/change-events", json={
            "change_type": "configmap_updated",
            "target_resource_id": "cm:order-config",
            "changed_at": "2026-06-19T03:00:00Z",
        })
        resp = c.get("/api/v1/change-events/timeline?application_id=app:order")
        assert resp.status_code == 200
        body = resp.json()
        assert body["total"] == 1
        assert body["by_type"]["configmap_updated"] == 1

    def test_get_timeline_unknown_app(self, client):
        c, _ = client
        resp = c.get("/api/v1/change-events/timeline?application_id=app:nope")
        assert resp.status_code == 404
