"""Sprint 2/3 connector 单元测试。

测试 PrometheusConnector / JaegerConnector / TraceAggregator /
FlagdConnector / K8sEventConnector / OTel scenarios mapping。

K8s/HTTP API 用 mock 模拟 — 不依赖真集群。
"""

from __future__ import annotations

import asyncio
from types import SimpleNamespace
from unittest.mock import AsyncMock, patch

import pytest

from app.datasource.connectors.flagd_connector import (
    FlagdConnector, _state_differs, _extract_value,
)
from app.datasource.connectors.health_rules import derive_health, evaluate_breach
from app.datasource.connectors.jaeger_connector import JaegerConnector
from app.datasource.connectors.k8s_event_connector import K8sEventConnector
from app.datasource.connectors.prometheus_connector import PrometheusConnector
from app.datasource.connectors.prometheus_queries import QUERIES
from app.datasource.connectors.trace_aggregator import (
    aggregate_calls_from_traces, _service_to_component_id,
)
from app.datasource.models import DataNode, MetricSnapshot
from app.datasource.store import store
from app.recovery.scenarios.otel_demo_scenarios import (
    SCENARIOS, scenario_for_flag, scenario_for_name,
)


# ============================================================
# Common fixtures
# ============================================================

@pytest.fixture(autouse=True)
def _clear_dss():
    """每个测试前清掉 connector 创建过的状态。"""
    for nid in [n for n, node in store.nodes.items()
                if (node.properties or {}).get("discovery_method", "").endswith("_connector")]:
        del store.nodes[nid]
    for eid in [e for e, edge in store.edges.items()
                if (edge.properties or {}).get("discovery_method", "").endswith("_connector")]:
        del store.edges[eid]
    store.metrics.clear()
    store.change_events.clear()
    yield


# ============================================================
# Health rules
# ============================================================

class TestHealthRules:
    def test_no_metrics_returns_none(self):
        """没数据时返回 None,告诉调用方"不要刷新原 health"。"""
        assert derive_health([]) is None

    def test_all_normal_green(self):
        snaps = [
            MetricSnapshot("s1", "comp:cart", "span_p99_ms", 50.0),
            MetricSnapshot("s2", "comp:cart", "span_error_rate_pct", 0.1),
        ]
        assert derive_health(snaps) == "green"

    def test_warning_breach_yellow(self):
        snaps = [
            MetricSnapshot("s1", "comp:cart", "span_p99_ms", 600.0),  # > 500 warning
        ]
        assert derive_health(snaps) == "yellow"

    def test_critical_beats_warning(self):
        snaps = [
            MetricSnapshot("s1", "comp:cart", "span_error_rate_pct", 10.0),  # > 5 critical
            MetricSnapshot("s2", "comp:cart", "span_p99_ms", 600.0),         # warning only
        ]
        assert derive_health(snaps) == "red"

    def test_unknown_metric_ignored(self):
        snaps = [
            MetricSnapshot("s1", "comp:cart", "ghost_metric", 999999.0),
        ]
        assert derive_health(snaps) == "green"  # 没有匹配 query → 不算 breach

    def test_evaluate_breach_returns_pair(self):
        warn, crit = evaluate_breach("span_p99_ms", 1500.0)
        assert warn is True
        assert crit is False
        warn, crit = evaluate_breach("span_p99_ms", 3000.0)
        assert warn is True
        assert crit is True
        warn, crit = evaluate_breach("unknown", 100.0)
        assert warn is False
        assert crit is False


# ============================================================
# Prometheus connector
# ============================================================

class TestPrometheusConnector:
    def test_resolve_target_id_service(self):
        c = PrometheusConnector(
            prometheus_url="http://stub", cluster_id="vm-cluster", namespace="otel-demo")
        assert c._resolve_target_id("service", {"service_name": "cartservice"}) == \
               "comp:vm-cluster:otel-demo:cart"
        assert c._resolve_target_id("service", {"service_name": "frontend"}) == \
               "comp:vm-cluster:otel-demo:frontend"
        assert c._resolve_target_id("service", {}) == ""

    def test_resolve_target_id_pod_path_returns_empty_without_label(self):
        c = PrometheusConnector(prometheus_url="http://stub")
        assert c._resolve_target_id("pod", {"namespace": "otel-demo"}) == ""

    def test_make_snapshot_breach_flags(self):
        c = PrometheusConnector(prometheus_url="http://stub")
        snap = c._make_snapshot("comp:cart", "span_p99_ms", 600.0, "ms")
        assert snap.warning_breached is True
        assert snap.critical_breached is False
        assert snap.metric_name == "span_p99_ms"
        assert snap.unit == "ms"
        assert "prom-" in snap.snapshot_id

    def test_sync_once_writes_metrics_and_updates_health(self):
        # 准备 component 节点
        node = DataNode(
            id="comp:vm-cluster:otel-demo:cart", type="ApplicationComponent", name="cart",
            properties={"discovery_method": "k8s_connector", "health": "green"},
        )
        store.upsert_node(node)

        # mock httpx 返回数据
        async def fake_get(url, **kwargs):
            if "query" not in kwargs.get("params", {}):
                raise AssertionError("expected query param")
            promql = kwargs["params"]["query"]
            # 简单分发:p99 → critical(2500ms),其他正常
            if "histogram_quantile" in promql:
                return SimpleNamespace(
                    raise_for_status=lambda: None,
                    json=lambda: {"status": "success", "data": {"result": [
                        {"metric": {"service_name": "cartservice"},
                         "value": [0, "2500"]},
                    ]}},
                )
            return SimpleNamespace(
                raise_for_status=lambda: None,
                json=lambda: {"status": "success", "data": {"result": [
                    {"metric": {"service_name": "cartservice"}, "value": [0, "0.5"]},
                ]}},
            )

        c = PrometheusConnector(prometheus_url="http://stub",
                                cluster_id="vm-cluster", namespace="otel-demo")

        with patch("httpx.AsyncClient") as mock_client:
            mock_client.return_value.__aenter__.return_value.get = AsyncMock(side_effect=fake_get)
            result = asyncio.run(c.sync_once())

        assert result.metrics_added > 0
        # health 应被推导成 red(p99 = 2500 > 2000 critical)
        assert store.get_node("comp:vm-cluster:otel-demo:cart").properties["health"] == "red"

    def test_sync_once_swallows_query_error(self):
        async def boom(*a, **kw):
            raise RuntimeError("prom down")

        c = PrometheusConnector(prometheus_url="http://stub")
        with patch("httpx.AsyncClient") as mock_client:
            mock_client.return_value.__aenter__.return_value.get = AsyncMock(side_effect=boom)
            result = asyncio.run(c.sync_once())
        # 不抛异常,而是把错误记到 notes
        assert any("failed" in n for n in result.notes)


# ============================================================
# Jaeger / trace aggregator
# ============================================================

class TestTraceAggregator:
    def _trace(self, edges_per_pair: dict[tuple[str, str], int]) -> dict:
        """构造一个 fake Jaeger trace。

        edges_per_pair = {(parent_svc, child_svc): count}
        生成对应数量的 child span,each 通过 references 指向 parent。
        """
        spans = []
        processes = {}
        seq = 0

        def add_span(svc):
            nonlocal seq
            span_id = f"sp-{seq}"
            pid = f"p-{svc}"
            processes[pid] = {"serviceName": svc}
            seq += 1
            return span_id, pid

        for (parent, child), count in edges_per_pair.items():
            parent_id, parent_pid = add_span(parent)
            spans.append({"spanID": parent_id, "processID": parent_pid, "references": []})
            for _ in range(count):
                child_id, child_pid = add_span(child)
                spans.append({
                    "spanID": child_id, "processID": child_pid,
                    "references": [{"refType": "CHILD_OF", "spanID": parent_id}],
                })

        return {"spans": spans, "processes": processes}

    def test_above_threshold_creates_edge(self):
        trace = self._trace({("frontend", "cartservice"): 6})
        counter, edges = aggregate_calls_from_traces(
            [trace], cluster_id="vm-cluster", namespace="otel-demo", threshold=5)
        assert counter[("frontend", "cartservice")] == 6
        assert len(edges) == 1
        assert edges[0]["source_id"] == "comp:vm-cluster:otel-demo:frontend"
        assert edges[0]["target_id"] == "comp:vm-cluster:otel-demo:cart"
        assert edges[0]["relationship_type"] == "CALLS"
        assert edges[0]["properties"]["call_count_5m"] == 6

    def test_below_threshold_filtered(self):
        trace = self._trace({("frontend", "cartservice"): 3})
        _, edges = aggregate_calls_from_traces([trace], "vm-cluster", "otel-demo", threshold=5)
        assert edges == []

    def test_self_call_excluded(self):
        trace = self._trace({("cartservice", "cartservice"): 10})
        _, edges = aggregate_calls_from_traces([trace], "vm-cluster", "otel-demo", threshold=5)
        assert edges == []

    def test_multiple_pairs_aggregated_across_traces(self):
        traces = [
            self._trace({("frontend", "cartservice"): 3}),
            self._trace({("frontend", "cartservice"): 4, ("checkoutservice", "paymentservice"): 6}),
        ]
        _, edges = aggregate_calls_from_traces(traces, "vm-cluster", "otel-demo", threshold=5)
        # frontend→cart = 7 (above), checkout→payment = 6 (above)
        rel_map = {(e["source_id"], e["target_id"]): e["properties"]["call_count_5m"] for e in edges}
        assert rel_map[("comp:vm-cluster:otel-demo:frontend", "comp:vm-cluster:otel-demo:cart")] == 7
        assert rel_map[("comp:vm-cluster:otel-demo:checkout", "comp:vm-cluster:otel-demo:payment")] == 6

    def test_service_to_component_id_strips_service_suffix(self):
        assert _service_to_component_id("cartservice", "vm-cluster", "otel-demo") == \
               "comp:vm-cluster:otel-demo:cart"
        assert _service_to_component_id("frontend", "vm-cluster", "otel-demo") == \
               "comp:vm-cluster:otel-demo:frontend"
        assert _service_to_component_id("", "vm-cluster", "otel-demo") == ""

    def test_empty_traces(self):
        counter, edges = aggregate_calls_from_traces(
            [], "vm-cluster", "otel-demo", threshold=5)
        assert len(counter) == 0
        assert edges == []

    def test_non_child_of_reference_ignored(self):
        """FOLLOWS_FROM(异步) 不算 caller→callee。"""
        trace = {
            "spans": [
                {"spanID": "s1", "processID": "p1", "references": []},
                {"spanID": "s2", "processID": "p2",
                 "references": [{"refType": "FOLLOWS_FROM", "spanID": "s1"}]},
            ],
            "processes": {"p1": {"serviceName": "a"}, "p2": {"serviceName": "b"}},
        }
        counter, _ = aggregate_calls_from_traces([trace] * 10, "vm-cluster", "otel-demo", threshold=1)
        assert counter == {}


class TestJaegerConnector:
    def test_skip_jaeger_internal_services(self):
        c = JaegerConnector(jaeger_url="http://stub")
        assert c._is_otel_demo_service("jaeger-query") is False
        assert c._is_otel_demo_service("loadgenerator") is False
        assert c._is_otel_demo_service("cartservice") is True

    def test_sync_with_no_traces_clears_old_edges(self):
        from app.datasource.models import DataEdge
        # 模拟上一轮留下的 jaeger_connector 边
        old = DataEdge(
            id="comp:a|CALLS|comp:b", source_id="comp:a", target_id="comp:b",
            relationship_type="CALLS", relationship_name="调用",
            properties={"discovery_method": "jaeger_connector", "call_count_5m": 8},
        )
        store.upsert_edge(old)

        c = JaegerConnector(jaeger_url="http://stub")

        async def fake_get(url, **kwargs):
            if url.endswith("/api/services"):
                return SimpleNamespace(
                    raise_for_status=lambda: None,
                    json=lambda: {"data": []},
                )
            return SimpleNamespace(
                raise_for_status=lambda: None,
                json=lambda: {"data": []},
            )

        with patch("httpx.AsyncClient") as mock_client:
            mock_client.return_value.__aenter__.return_value.get = AsyncMock(side_effect=fake_get)
            result = asyncio.run(c.sync_once())

        # 没新边 → 删旧边
        assert result.edges_removed == 1
        assert store.get_edge("comp:a|CALLS|comp:b") is None


# ============================================================
# Flagd connector
# ============================================================

class TestFlagdConnector:
    def test_extract_value_bool(self):
        assert _extract_value({"variant": "off", "boolValue": False}) is False
        assert _extract_value({"variant": "on", "boolValue": True}) is True

    def test_extract_value_double(self):
        assert _extract_value({"variant": "level1", "doubleValue": 0.5}) == 0.5

    def test_state_differs_by_variant(self):
        assert _state_differs(
            {"variant": "off", "boolValue": False},
            {"variant": "on", "boolValue": True},
        ) is True

    def test_state_differs_same_variant_same_value(self):
        old = {"variant": "off", "boolValue": False}
        assert _state_differs(old, old) is False

    def test_first_sync_baseline_no_events(self):
        c = FlagdConnector(flagd_url="http://stub")

        async def fake_post(url, **kw):
            return SimpleNamespace(
                raise_for_status=lambda: None,
                json=lambda: {"flags": {
                    "f1": {"variant": "off", "boolValue": False},
                    "f2": {"variant": "off", "boolValue": False},
                }},
            )

        with patch("httpx.AsyncClient") as mock_client:
            mock_client.return_value.__aenter__.return_value.post = AsyncMock(side_effect=fake_post)
            result = asyncio.run(c.sync_once())
        assert result.events_added == 0
        assert any("baseline" in n for n in result.notes)

    def test_second_sync_emits_change_events_on_flip(self):
        c = FlagdConnector(flagd_url="http://stub")
        c._last_snapshot = {
            "f1": {"variant": "off", "boolValue": False},
            "f2": {"variant": "off", "boolValue": False},
        }

        async def fake_post(url, **kw):
            return SimpleNamespace(
                raise_for_status=lambda: None,
                json=lambda: {"flags": {
                    "f1": {"variant": "on", "boolValue": True},  # flipped
                    "f2": {"variant": "off", "boolValue": False},  # unchanged
                }},
            )

        with patch("httpx.AsyncClient") as mock_client:
            mock_client.return_value.__aenter__.return_value.post = AsyncMock(side_effect=fake_post)
            result = asyncio.run(c.sync_once())

        assert result.events_added == 1
        # 检查 ChangeEvent 是否真的写入 DSS
        events = list(store.change_events.values())
        assert len(events) == 1
        assert events[0].source == "flagd"
        assert "f1" in events[0].diff_summary

    def test_added_flag_emits_event(self):
        c = FlagdConnector(flagd_url="http://stub")
        c._last_snapshot = {"f1": {"variant": "off", "boolValue": False}}

        async def fake_post(url, **kw):
            return SimpleNamespace(
                raise_for_status=lambda: None,
                json=lambda: {"flags": {
                    "f1": {"variant": "off", "boolValue": False},
                    "newFlag": {"variant": "on", "boolValue": True},
                }},
            )

        with patch("httpx.AsyncClient") as mock_client:
            mock_client.return_value.__aenter__.return_value.post = AsyncMock(side_effect=fake_post)
            result = asyncio.run(c.sync_once())
        assert result.events_added == 1
        assert any("flag added" in str(e.description) for e in store.change_events.values())

    def test_removed_flag_emits_event(self):
        c = FlagdConnector(flagd_url="http://stub")
        c._last_snapshot = {
            "f1": {"variant": "off", "boolValue": False},
            "stale": {"variant": "off", "boolValue": False},
        }

        async def fake_post(url, **kw):
            return SimpleNamespace(
                raise_for_status=lambda: None,
                json=lambda: {"flags": {"f1": {"variant": "off", "boolValue": False}}},
            )

        with patch("httpx.AsyncClient") as mock_client:
            mock_client.return_value.__aenter__.return_value.post = AsyncMock(side_effect=fake_post)
            result = asyncio.run(c.sync_once())
        assert result.events_added == 1


# ============================================================
# K8s event connector
# ============================================================

class TestK8sEventConnector:
    def _make_event(self, uid, reason, kind, name, msg=""):
        return SimpleNamespace(
            metadata=SimpleNamespace(uid=uid),
            reason=reason,
            message=msg,
            involved_object=SimpleNamespace(kind=kind, name=name),
        )

    def test_event_to_change_deployment_scaled(self):
        c = K8sEventConnector(cluster_id="vm-cluster", namespace="otel-demo")
        ev = self._make_event(
            "uid-1", "ScalingReplicaSet", "Deployment", "frontend",
            msg="Scaled up replica set",
        )
        ce = c._event_to_change(ev)
        assert ce is not None
        assert ce["change_type"] == "deployment_rolled"
        assert ce["target_resource_id"] == "deploy:vm-cluster:otel-demo:frontend"
        assert ce["source"] == "k8s_api"

    def test_event_to_change_replicaset_strips_hash(self):
        c = K8sEventConnector(cluster_id="vm-cluster", namespace="otel-demo")
        ev = self._make_event(
            "uid-2", "ScalingReplicaSet", "ReplicaSet", "frontend-87bbfc4c9",
        )
        ce = c._event_to_change(ev)
        assert ce is not None
        # ReplicaSet → Deployment 反推
        assert ce["target_resource_id"] == "deploy:vm-cluster:otel-demo:frontend"

    def test_uninteresting_reason_skipped(self):
        c = K8sEventConnector()
        ev = self._make_event("uid-3", "FailedScheduling", "Pod", "x")
        assert c._event_to_change(ev) is None

    def test_unknown_kind_skipped(self):
        c = K8sEventConnector()
        ev = self._make_event("uid-4", "ScalingReplicaSet", "DaemonSet", "x")
        assert c._event_to_change(ev) is None


# ============================================================
# Fault scenarios
# ============================================================

class TestScenarios:
    def test_seven_or_more_scenarios_defined(self):
        """PRD-004 要求至少 7 个 scenario。"""
        assert len(SCENARIOS) >= 7

    def test_scenario_lookup_by_flag(self):
        s = scenario_for_flag("productCatalogFailure")
        assert s is not None
        assert s.target_component == "product-catalog"
        assert s.recommended_action == "restart_pod"

    def test_scenario_lookup_unknown_returns_none(self):
        assert scenario_for_flag("ghost") is None

    def test_scenario_lookup_by_name(self):
        s = scenario_for_name("cart_failure")
        assert s is not None
        assert s.flag_name == "cartServiceFailure"
        assert s.recommended_action == "clear_cache"

    def test_all_scenarios_have_required_fields(self):
        for s in SCENARIOS:
            assert s.flag_name
            assert s.target_component
            assert s.recommended_action in {
                "restart_pod", "restart_service", "scale_deployment",
                "rollback_deployment", "clear_cache", "kill_query",
                "refresh_secret", "drain_node",
            }, f"{s.name} → unknown action {s.recommended_action}"
            assert s.finding_severity in {"warning", "critical"}

    def test_scenario_name_uniqueness(self):
        names = [s.name for s in SCENARIOS]
        assert len(names) == len(set(names)), "duplicate scenario names"

    def test_scenario_flag_uniqueness(self):
        flags = [s.flag_name for s in SCENARIOS]
        assert len(flags) == len(set(flags)), "duplicate flag mappings"
