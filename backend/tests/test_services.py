"""graph_service 与 metrics_service 单元测试"""

from datetime import datetime

from app.services.graph_service import format_graph_response, VALID_REL_TYPES
from app.services.metrics_service import (
    format_metrics_from_snapshots,
    check_threshold,
)
from app.models.graph import GraphResponse
from tests.mocks import MockNeo4jNode, MockNeo4jRel


# ============================================================
# graph_service — format_graph_response
# ============================================================

class TestFormatGraphResponse:
    """测试 format_graph_response 各种输入场景"""

    def test_empty_records_returns_empty_response(self):
        result = format_graph_response([])
        assert isinstance(result, GraphResponse)
        assert result.nodes == []
        assert result.edges == []
        assert result.summary.total_nodes == 0
        assert result.summary.total_edges == 0

    def test_path_record_extraction(self, sample_path):
        """从 Neo4j Path 提取节点和边"""
        result = format_graph_response([{"path": sample_path}])
        assert result.summary.total_nodes == 5
        assert result.summary.total_edges == 4

        node_ids = {n.id for n in result.nodes}
        assert "app:order" in node_ids
        assert "pod:order:order-api-xxx" in node_ids

        edge_ids = {e.id for e in result.edges}
        assert "e001" in edge_ids
        assert "e101" in edge_ids

    def test_nodes_and_edges_record_format(self, sample_nodes, sample_edges):
        """直接传入 nodes + edges 列表的 record"""
        result = format_graph_response([{
            "nodes": sample_nodes[:3],
            "edges": sample_edges[:2],
        }])
        assert result.summary.total_nodes == 3
        assert result.summary.total_edges == 2

    def test_individual_node_records(self):
        """每条 record 是一个节点的场景（如 MATCH (n) RETURN n）"""
        node = MockNeo4jNode("pod:test", ["Pod", "ResourceInstance"], {
            "name": "test-pod", "label": "Pod", "node_id": "pod:test",
            "health_status": "normal", "risk_level": "low",
        })
        result = format_graph_response([{"n": node}])
        assert result.summary.total_nodes == 1
        assert result.nodes[0].id == "pod:test"

    def test_node_deduplication(self):
        """相同 node_id 只出现一次"""
        node = MockNeo4jNode("pod:dup", ["Pod", "ResourceInstance"], {
            "node_id": "pod:dup", "name": "dup-pod", "label": "Pod",
            "health_status": "warning", "risk_level": "medium",
        })
        result = format_graph_response([{"n": node}, {"n": node}])
        assert result.summary.total_nodes == 1

    def test_risk_and_health_counting(self, sample_nodes):
        """验证风险等级和健康状态统计"""
        result = format_graph_response([{"nodes": sample_nodes, "edges": []}])
        s = result.summary
        # 5 nodes: 1 warning+medium, 1 warning+medium, 1 normal+low, 1 critical+high, 1 normal+low
        assert s.risk_counts["high"] == 1
        assert s.risk_counts["medium"] == 2
        assert s.risk_counts["low"] == 2
        assert s.health_counts["normal"] == 2
        assert s.health_counts["warning"] == 2
        assert s.health_counts["critical"] == 1

    def test_node_type_from_labels(self):
        """节点类型从 Neo4j labels 提取，过滤 ResourceInstance"""
        node = MockNeo4jNode("test-1", ["Pod", "ResourceInstance"], {
            "node_id": "test-1", "label": "Pod", "name": "test",
            "health_status": "normal", "risk_level": "low",
        })
        result = format_graph_response([{"n": node}])
        assert result.nodes[0].type == "Pod"

    def test_node_serialize_datetime(self):
        """datetime 被序列化为 ISO 字符串"""
        node = MockNeo4jNode("test-dt", ["Pod"], {
            "node_id": "test-dt", "label": "Pod", "name": "test",
            "health_status": "normal", "risk_level": "low",
            "created_at": datetime(2026, 6, 15, 10, 0, 0),
        })
        result = format_graph_response([{"n": node}])
        props = result.nodes[0].properties
        assert isinstance(props.get("created_at"), str)

    def test_edge_uses_relationship_type(self):
        """边类型从 relationship_type 属性读取"""
        source = MockNeo4jNode("s", ["Pod"], {"node_id": "s", "label": "Pod", "name": "s", "health_status": "normal", "risk_level": "low"})
        target = MockNeo4jNode("t", ["KubernetesNode"], {"node_id": "t", "label": "Node", "name": "t", "health_status": "normal", "risk_level": "low"})
        rel = MockNeo4jRel("r1", "SCHEDULED_ON", source, target, {
            "edge_id": "r1", "relationship_type": "SCHEDULED_ON",
            "relationship_name": "调度在", "dependency_strength": "强",
        })
        result = format_graph_response([{"r": rel}])
        assert result.edges[0].type == "SCHEDULED_ON"
        assert result.edges[0].source == "s"
        assert result.edges[0].target == "t"


class TestValidRelTypes:
    """验证关系类型常量"""

    def test_all_views_use_valid_types(self):
        """确保所有6视图引用的关系类型都在 VALID_REL_TYPES 中"""
        expected = {
            'CONTAINS', 'DEPLOYED_AS', 'DEPLOYED_IN', 'BELONGS_TO',
            'EXPOSES', 'ROUTES_TO', 'USES', 'STORED_IN',
            'MONITORS', 'VISUALIZES', 'RUNS', 'SCHEDULED_ON',
            'GENERATED', 'VIOLATES', 'AFFECTS', 'PROPAGATES_TO',
            'FIRED_ON', 'AGGREGATES_TO', 'MEASURES',
        }
        assert VALID_REL_TYPES == expected


# ============================================================
# metrics_service
# ============================================================

class TestFormatMetricsFromSnapshots:
    """测试指标快照格式化"""

    def test_basic_formatting(self):
        snapshots = [
            {"snapshot_id": "s1", "metric_name": "cpu_usage", "current_value": "45.2",
             "unit": "percent", "fetched_at": "2026-06-15T10:00:00", "is_stale": "false",
             "warning_breached": "false", "critical_breached": "false",
             "warning_threshold": "80", "critical_threshold": "95"},
        ]
        result = format_metrics_from_snapshots(snapshots)
        assert len(result) == 1
        m = result[0]
        assert m["metric_name"] == "cpu_usage"
        assert m["current_value"] == 45.2
        assert m["is_stale"] is False
        assert m["warning_breached"] is False
        assert m["warning_threshold"] == 80.0

    def test_breached_values(self):
        snapshots = [
            {"snapshot_id": "s2", "metric_name": "cpu_usage", "current_value": "92",
             "unit": "percent", "fetched_at": "", "is_stale": "false",
             "warning_breached": "true", "critical_breached": "true",
             "warning_threshold": "80", "critical_threshold": "95"},
        ]
        result = format_metrics_from_snapshots(snapshots)
        assert result[0]["warning_breached"] is True
        assert result[0]["critical_breached"] is True

    def test_empty_list(self):
        assert format_metrics_from_snapshots([]) == []

    def test_missing_fields_default(self):
        result = format_metrics_from_snapshots([{}])
        assert result[0]["metric_name"] == ""
        assert result[0]["current_value"] == 0.0


class TestCheckThreshold:
    """测试阈值检查"""

    def test_normal_below_warning(self):
        assert check_threshold(50, 80, 95) == "normal"

    def test_warning_above_warning_below_critical(self):
        assert check_threshold(85, 80, 95) == "warning"

    def test_critical_above_critical(self):
        assert check_threshold(96, 80, 95) == "critical"

    def test_equal_to_warning_threshold(self):
        assert check_threshold(80, 80, 95) == "warning"

    def test_equal_to_critical_threshold(self):
        assert check_threshold(95, 80, 95) == "critical"

    def test_no_warning_threshold(self):
        """无 warning 阈值时，只检查 critical"""
        assert check_threshold(50, None, 95) == "normal"
        assert check_threshold(96, None, 95) == "critical"

    def test_no_thresholds(self):
        assert check_threshold(50, None, None) == "normal"

    def test_zero_thresholds(self):
        """零值作为有效阈值"""
        assert check_threshold(1, 0, 0) == "critical"
