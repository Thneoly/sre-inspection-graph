"""PRD-004 Phase 2 测试 — flagd 接入 scenario_for_flag。

FlagdConnector 写 ChangeEvent 时查 OTel demo scenario 映射,把 recommended_action
塞进 diff_summary + description,贯通 flag→finding→recovery 链。
"""

import pytest
from app.datasource.store import store


@pytest.fixture(autouse=True)
def _clear():
    store.clear_change_events()
    yield
    store.clear_change_events()


class TestFlagdScenarioEnrichment:
    def test_fault_flag_event_carries_recommended_action(self):
        """productCatalogFailure 翻转 → ChangeEvent.diff_summary 含 scenario.recommended_action。"""
        from app.datasource.connectors.flagd_connector import _try_record

        _try_record(
            target_id="configmap:vm-cluster:otel-demo:otel-demo-flagd-config",
            flag_name="productCatalogFailure",
            old=False, new=True,
            description="flag productCatalogFailure: variant=off → on",
        )
        events = store.list_change_events()
        assert len(events) == 1
        ev = events[0]
        summary = ev.diff_summary
        assert "scenario" in summary
        assert summary["scenario"]["recommended_action"] == "restart_pod"
        assert summary["scenario"]["target_component"] == "product-catalog"
        # description 也带 scenario 标注
        assert "scenario=" in ev.description
        assert "restart_pod" in ev.description

    def test_non_fault_flag_event_has_no_scenario(self):
        """非 OTel demo 故障 flag(如 feature flag business 开关)→ 无 scenario 字段。"""
        from app.datasource.connectors.flagd_connector import _try_record

        _try_record(
            target_id="configmap:vm-cluster:otel-demo:otel-demo-flagd-config",
            flag_name="someBusinessToggle",
            old=False, new=True,
            description="flag someBusinessToggle flipped",
        )
        ev = store.list_change_events()[0]
        assert "scenario" not in ev.diff_summary
        assert "scenario=" not in ev.description

    def test_lookup_scenario_returns_none_for_unknown(self):
        from app.datasource.connectors.flagd_connector import _lookup_scenario
        assert _lookup_scenario("ghost-flag") is None

    def test_lookup_scenario_returns_known(self):
        from app.datasource.connectors.flagd_connector import _lookup_scenario
        s = _lookup_scenario("cartServiceFailure")
        assert s is not None
        assert s.recommended_action in {"restart_pod", "clear_cache", "scale_deployment",
                                        "rollback_deployment", "restart_service"}
