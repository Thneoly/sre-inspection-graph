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
