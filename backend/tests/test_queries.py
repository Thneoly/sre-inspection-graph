"""Cypher 查询模块单元测试 — 验证 6 个视图查询生成"""

from app.db.queries.view1_topology import get_app_topology
from app.db.queries.view2_access_link import get_access_link
from app.db.queries.view3_node_impact import get_node_impact
from app.db.queries.view4_config_impact import get_config_impact
from app.db.queries.view5_image_risk import get_image_risk
from app.db.queries.view6_alert_aggr import get_alert_aggregation


class TestView1Topology:
    def test_basic_params(self):
        cypher, params = get_app_topology("order")
        assert params["app_node_id"] == "app:order"
        assert params["limit"] == 200
        assert "*1..5" in cypher
        assert "MATCH path" in cypher
        assert "Application" in cypher

    def test_custom_depth(self):
        cypher, _ = get_app_topology("order", depth=3)
        assert "*1..3" in cypher

    def test_query_contains_required_rel_types(self):
        cypher, _ = get_app_topology("order")
        for rel_type in ["CONTAINS", "DEPLOYED_AS", "RUNS", "SCHEDULED_ON"]:
            assert rel_type in cypher


class TestView2AccessLink:
    def test_basic_params(self):
        cypher, params = get_access_link("order")
        assert params["app_node_id"] == "app:order"
        assert "Ingress" in cypher
        assert "ROUTES_TO" in cypher


class TestView3NodeImpact:
    def test_basic_params(self):
        node_id = "node:cce-prod-01:worker-01"
        cypher, params = get_node_impact(node_id)
        assert params["node_id"] == node_id
        assert "KubernetesNode" in cypher
        assert "SCHEDULED_ON" in cypher

    def test_custom_depth(self):
        cypher, _ = get_node_impact("node:x", depth=6)
        assert "*1..6" in cypher


class TestView4ConfigImpact:
    def test_secret_impact(self):
        cypher, params = get_config_impact("secret:cce-prod-01:order:order-api-secret")
        assert params["resource_id"] == "secret:cce-prod-01:order:order-api-secret"
        assert "Secret" in cypher
        assert "ConfigMap" in cypher

    def test_configmap_impact(self):
        _, params = get_config_impact("cm:cce-prod-01:order:order-api-config")
        assert params["resource_id"] == "cm:cce-prod-01:order:order-api-config"


class TestView5ImageRisk:
    def test_basic_params(self):
        cypher, params = get_image_risk("image:order-api:1.2.3")
        assert params["image_id"] == "image:order-api:1.2.3"
        assert "ContainerImage" in cypher
        assert "USES" in cypher


class TestView6AlertAggregation:
    def test_no_filter(self):
        cypher, _ = get_alert_aggregation()
        assert "AlertEvent" in cypher
        assert "FIRED_ON" in cypher

    def test_severity_filter_critical(self):
        _, params = get_alert_aggregation(severity="critical")
        assert params.get("severity_health") == "critical"

    def test_severity_filter_warning(self):
        _, params = get_alert_aggregation(severity="warning")
        assert params.get("severity_health") == "warning"

    def test_custom_limit(self):
        _, params = get_alert_aggregation(limit=50)
        assert params["limit"] == 50


class TestParameterization:
    """验证参数不会被 SQL/注入攻击"""

    def test_app_code_special_chars(self):
        """特殊字符应安全处理 — 通过参数化查询，值是安全的"""
        _, params = get_app_topology("order'; DETACH DELETE n--")
        # 参数化的值不改变查询结构
        assert params["app_node_id"] == "app:order'; DETACH DELETE n--"

    def test_node_id_special_chars(self):
        _, params = get_node_impact("../../etc/passwd")
        assert params["node_id"] == "../../etc/passwd"
