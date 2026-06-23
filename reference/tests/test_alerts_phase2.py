"""PRD-004 Phase 2 测试 — AlertEvent + AlertRule + connector→AlertEvent。

覆盖:
- AlertRule 生成(generate_alert_rules 从 QueryDef)
- record_alert:写入 + 去重 + 序列化 + Neo4j dual-write(best-effort)
- resolve_alert:firing → resolved
- prometheus connector critical breach → record_alert
- alert router 端点
- AlertEvent↔ChangeEvent 贯通(record_alert 后 record_change 的 correlate 能命中)
"""

import pytest
from unittest.mock import MagicMock, patch
from app.datasource.models import DataNode, DataEdge, MetricSnapshot
from app.datasource.store import store


# ============================================================
# 种子
# ============================================================

@pytest.fixture(scope="module", autouse=True)
def _seed_store():
    store.nodes.clear()
    store.edges.clear()
    store.change_events.clear()
    store.alert_events.clear()
    store.alert_rules.clear()
    store.metrics.clear()

    nodes = [
        DataNode("comp:vm-cluster:otel-demo:cart", "ApplicationComponent", "cart"),
        DataNode("comp:vm-cluster:otel-demo:checkout", "ApplicationComponent", "checkout"),
    ]
    for n in nodes:
        store.upsert_node(n)
    yield
    store.nodes.clear()
    store.edges.clear()
    store.change_events.clear()
    store.alert_events.clear()
    store.alert_rules.clear()
    store.metrics.clear()


@pytest.fixture(autouse=True)
def _clear_alerts():
    store.clear_alert_events()
    store.metrics.clear()  # 避免 metric snapshot 跨测试影响 derive_health


# ============================================================
# AlertRule 生成
# ============================================================

class TestAlertRuleGeneration:
    def test_generate_alert_rules_from_queries(self):
        from app.datasource.connectors.health_rules import generate_alert_rules
        rules = generate_alert_rules()
        # 3 QueryDef × 2 severity = 6 rules
        assert len(rules) == 6
        ids = {r.rule_id for r in rules}
        assert "alert_rule:span_p99_ms:critical" in ids
        assert "alert_rule:span_error_rate_pct:warning" in ids

    def test_rule_carries_threshold_and_metric(self):
        from app.datasource.connectors.health_rules import generate_alert_rules
        rules = generate_alert_rules()
        p99_crit = next(r for r in rules if r.rule_id == "alert_rule:span_p99_ms:critical")
        assert p99_crit.metric_name == "span_p99_ms"
        assert p99_crit.severity == "critical"
        assert p99_crit.threshold == 2000.0
        assert p99_crit.unit == "ms"

    def test_sync_to_store_is_idempotent(self):
        from app.datasource.connectors.health_rules import sync_alert_rules_to_store
        n1 = sync_alert_rules_to_store()
        n2 = sync_alert_rules_to_store()
        assert n1 == n2 == 6
        assert len(store.alert_rules) == 6


# ============================================================
# record_alert
# ============================================================

class TestRecordAlert:
    def test_record_alert_writes_to_dss(self):
        from app.alerts.alert_service import record_alert, serialize

        ev = record_alert(
            alert_name="span_p99_ms critical",
            resource_ref="comp:vm-cluster:otel-demo:cart",
            severity="critical",
            rule_id="alert_rule:span_p99_ms:critical",
            metric_name="span_p99_ms",
            metric_value=2500.0,
            cluster_id="vm-cluster",
        )
        assert ev is not None
        assert ev.status == "firing"
        assert ev.metric_value == 2500.0
        assert store.get_alert_event(ev.alert_event_id) is ev
        s = serialize(ev)
        assert s["rule_id"] == "alert_rule:span_p99_ms:critical"

    def test_record_alert_dedupes_firing(self):
        """同一 resource + rule 的 firing 告警不重复产出。"""
        from app.alerts.alert_service import record_alert

        ev1 = record_alert(
            alert_name="x", resource_ref="comp:vm-cluster:otel-demo:cart",
            rule_id="alert_rule:span_p99_ms:critical", metric_name="span_p99_ms",
        )
        ev2 = record_alert(
            alert_name="x", resource_ref="comp:vm-cluster:otel-demo:cart",
            rule_id="alert_rule:span_p99_ms:critical", metric_name="span_p99_ms",
        )
        assert ev1 is not None
        assert ev2 is None  # deduped
        assert len(store.list_alert_events()) == 1

    def test_record_alert_invalid_severity_returns_none(self):
        from app.alerts.alert_service import record_alert
        ev = record_alert(
            alert_name="x", resource_ref="comp:vm-cluster:otel-demo:cart",
            severity="bogus",
        )
        assert ev is None

    def test_record_alert_persists_to_neo4j(self):
        from app.alerts import alert_service

        session = MagicMock()
        driver = MagicMock()
        driver.session.return_value.__enter__.return_value = session
        with patch.object(alert_service.n4j, "get_driver", return_value=driver):
            ev = alert_service.record_alert(
                alert_name="x", resource_ref="comp:vm-cluster:otel-demo:cart",
                severity="critical", rule_id="r1", metric_name="m", metric_value=1.0,
            )
        assert ev is not None
        # 节点 MERGE + FIRED_ON 边(2 次 run;target 存在)
        assert session.run.call_count >= 1
        node_cypher = session.run.call_args_list[0].args[0]
        assert "MERGE (ae:AlertEvent:ResourceInstance" in node_cypher

    def test_record_alert_neo4j_offline_does_not_block(self):
        from app.alerts import alert_service
        with patch.object(alert_service.n4j, "get_driver", return_value=None):
            ev = alert_service.record_alert(
                alert_name="x", resource_ref="comp:vm-cluster:otel-demo:cart",
            )
        assert ev is not None  # DSS 仍写入


# ============================================================
# resolve_alert
# ============================================================

class TestResolveAlert:
    def test_resolve_marks_resolved(self):
        from app.alerts.alert_service import record_alert, resolve_alert
        ev = record_alert(
            alert_name="x", resource_ref="comp:vm-cluster:otel-demo:cart",
            rule_id="r1", metric_name="m",
        )
        resolved = resolve_alert(ev.alert_event_id)
        assert resolved.status == "resolved"
        assert resolved.resolved_at != ""

    def test_resolve_unknown_returns_none(self):
        from app.alerts.alert_service import resolve_alert
        assert resolve_alert("nope") is None


# ============================================================
# Prometheus connector → AlertEvent
# ============================================================

class TestPrometheusEmitsAlerts:
    def test_critical_breach_produces_alert(self):
        """prometheus connector sync_once 检测 critical breach → record_alert。"""
        import asyncio
        from types import SimpleNamespace
        from unittest.mock import AsyncMock, patch
        from app.datasource.connectors.prometheus_connector import PrometheusConnector

        async def fake_get(url, **kwargs):
            promql = kwargs["params"]["query"]
            if "histogram_quantile" in promql:  # p99 → critical 2500
                return SimpleNamespace(
                    raise_for_status=lambda: None,
                    json=lambda: {"status": "success", "data": {"result": [
                        {"metric": {"service_name": "cartservice"}, "value": [0, "2500"]},
                    ]}},
                )
            return SimpleNamespace(
                raise_for_status=lambda: None,
                json=lambda: {"status": "success", "data": {"result": [
                    {"metric": {"service_name": "cartservice"}, "value": [0, "0.5"]},
                ]}},
            )

        conn = PrometheusConnector(
            prometheus_url="http://stub", cluster_id="vm-cluster", namespace="otel-demo",
        )
        with patch("httpx.AsyncClient") as mock_client:
            mock_client.return_value.__aenter__.return_value.get = AsyncMock(side_effect=fake_get)
            result = asyncio.run(conn.sync_once())

        alerts = store.list_alert_events(resource_ref="comp:vm-cluster:otel-demo:cart")
        assert len(alerts) == 1
        assert alerts[0].severity == "critical"
        assert alerts[0].metric_value == 2500.0
        assert result.events_added >= 1

    def test_warning_breach_produces_warning_alert(self):
        import asyncio
        from types import SimpleNamespace
        from unittest.mock import AsyncMock, patch
        from app.datasource.connectors.prometheus_connector import PrometheusConnector

        async def fake_get(url, **kwargs):
            promql = kwargs["params"]["query"]
            if "histogram_quantile" in promql:  # p99 → 800 (warning band)
                return SimpleNamespace(
                    raise_for_status=lambda: None,
                    json=lambda: {"status": "success", "data": {"result": [
                        {"metric": {"service_name": "cartservice"}, "value": [0, "800"]},
                    ]}},
                )
            return SimpleNamespace(
                raise_for_status=lambda: None,
                json=lambda: {"status": "success", "data": {"result": [
                    {"metric": {"service_name": "cartservice"}, "value": [0, "0.5"]},
                ]}},
            )

        conn = PrometheusConnector(
            prometheus_url="http://stub", cluster_id="vm-cluster", namespace="otel-demo",
        )
        with patch("httpx.AsyncClient") as mock_client:
            mock_client.return_value.__aenter__.return_value.get = AsyncMock(side_effect=fake_get)
            asyncio.run(conn.sync_once())

        alerts = store.list_alert_events(resource_ref="comp:vm-cluster:otel-demo:cart")
        assert len(alerts) == 1
        assert alerts[0].severity == "warning"

    def test_no_breach_no_alert(self):
        import asyncio
        from types import SimpleNamespace
        from unittest.mock import AsyncMock, patch
        from app.datasource.connectors.prometheus_connector import PrometheusConnector

        async def fake_get(url, **kwargs):
            promql = kwargs["params"]["query"]
            # 每条 query 都返回安全值:p99<500, error_rate<1, request_rate 任意
            value = "10" if "histogram_quantile" in promql else "0.1"
            return SimpleNamespace(
                raise_for_status=lambda: None,
                json=lambda: {"status": "success", "data": {"result": [
                    {"metric": {"service_name": "cartservice"}, "value": [0, value]},
                ]}},
            )

        conn = PrometheusConnector(
            prometheus_url="http://stub", cluster_id="vm-cluster", namespace="otel-demo",
        )
        with patch("httpx.AsyncClient") as mock_client:
            mock_client.return_value.__aenter__.return_value.get = AsyncMock(side_effect=fake_get)
            asyncio.run(conn.sync_once())
        assert store.list_alert_events() == []


# ============================================================
# Alert router 端点
# ============================================================

class TestAlertEndpoints:
    def test_list_rules(self):
        from app.main import app
        from fastapi.testclient import TestClient
        client = TestClient(app)
        resp = client.get("/api/v1/alerts/rules")
        assert resp.status_code == 200
        body = resp.json()
        assert body["total"] == 6
        ids = [r["rule_id"] for r in body["rules"]]
        assert "alert_rule:span_p99_ms:critical" in ids

    def test_create_and_list_and_resolve(self):
        from app.main import app
        from fastapi.testclient import TestClient
        client = TestClient(app)
        # create
        resp = client.post("/api/v1/alerts", json={
            "alert_name": "test alert",
            "resource_ref": "comp:vm-cluster:otel-demo:cart",
            "severity": "critical",
            "rule_id": "alert_rule:test:critical",
            "metric_name": "m",
            "metric_value": 99.0,
        })
        assert resp.status_code == 201
        aid = resp.json()["alert_event_id"]
        # list
        resp = client.get("/api/v1/alerts?resource_ref=comp:vm-cluster:otel-demo:cart")
        assert resp.json()["total"] == 1
        # resolve
        resp = client.post(f"/api/v1/alerts/{aid}/resolve")
        assert resp.status_code == 200
        assert resp.json()["status"] == "resolved"


# ============================================================
# 贯通:AlertEvent ↔ ChangeEvent(CORRELATED_WITH 链路)
# ============================================================

class TestAlertChangeCorrelation:
    def test_record_change_correlates_existing_alert(self):
        """先 record_alert,再 record_change 同资源 → correlate_alerts 命中。

        PRD-002 Phase 2 的 correlate_alerts 已支持从 DSS 读 AlertEvent(Phase 2 增强),
        形成 connector → AlertEvent ↔ ChangeEvent 贯通链。
        """
        from app.alerts.alert_service import record_alert
        from app.changes.event_service import record_change
        from app.changes.alert_correlation import correlate_alerts

        # 先发告警
        record_alert(
            alert_name="span_p99_ms critical",
            resource_ref="comp:vm-cluster:otel-demo:cart",
            severity="critical",
            rule_id="alert_rule:span_p99_ms:critical",
            metric_name="span_p99_ms",
            metric_value=2500.0,
        )
        # 后发变更(同资源 + 同时间窗)
        ev = record_change(
            change_type="deployment_rolled",
            target_resource_id="comp:vm-cluster:otel-demo:cart",
            source="k8s_api",
        )
        result = correlate_alerts(ev.change_event_id, window_seconds=600)
        assert result["total"] >= 1
        assert result["alerts"][0]["resource_ref"] == "comp:vm-cluster:otel-demo:cart"
