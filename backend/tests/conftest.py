"""pytest 配置与共享 fixtures"""

import sys
import pytest
from unittest.mock import MagicMock, patch
from fastapi.testclient import TestClient

from tests.mocks import MockNeo4jNode, MockNeo4jRel, MockNeo4jPath


# ============================================================
# Pre-mock neo4j to avoid import errors in test environment
# ============================================================
_mock_neo4j = MagicMock()
_mock_neo4j.GraphDatabase = MagicMock()
_mock_neo4j.Driver = MagicMock()
sys.modules["neo4j"] = _mock_neo4j


# ============================================================
# 共享 Fixtures
# ============================================================

@pytest.fixture
def sample_nodes():
    return [
        MockNeo4jNode("app:order", ["Application", "ResourceInstance"], {
            "node_id": "app:order", "name": "订单应用", "label": "Application",
            "health_status": "warning", "risk_level": "medium", "owner_team": "订单团队",
            "app_code": "order", "sla_level": "P1",
        }),
        MockNeo4jNode("comp:order-api", ["ApplicationComponent", "ResourceInstance"], {
            "node_id": "comp:order-api", "name": "订单API组件", "label": "ApplicationComponent",
            "health_status": "warning", "risk_level": "medium", "owner_team": "订单团队",
        }),
        MockNeo4jNode("deploy:order:order-api", ["Deployment", "ResourceInstance"], {
            "node_id": "deploy:order:order-api", "name": "order-api", "label": "Deployment",
            "health_status": "normal", "risk_level": "low",
            "desired_replicas": 3, "available_replicas": 2,
        }),
        MockNeo4jNode("pod:order:order-api-xxx", ["Pod", "ResourceInstance"], {
            "node_id": "pod:order:order-api-xxx", "name": "order-api-xxx", "label": "Pod",
            "health_status": "critical", "risk_level": "high",
            "pod_ip": "10.244.1.23", "phase": "Running", "restart_count": 5,
        }),
        MockNeo4jNode("node:worker-01", ["KubernetesNode", "ResourceInstance"], {
            "node_id": "node:worker-01", "name": "worker-01", "label": "KubernetesNode",
            "health_status": "normal", "risk_level": "low",
        }),
    ]


@pytest.fixture
def sample_edges(sample_nodes):
    apps = {n._id: n for n in sample_nodes}
    return [
        MockNeo4jRel("e001", "CONTAINS", apps["app:order"], apps["comp:order-api"], {
            "edge_id": "e001", "relationship_type": "CONTAINS",
            "relationship_name": "包含", "dependency_strength": "强",
        }),
        MockNeo4jRel("e003", "DEPLOYED_AS", apps["comp:order-api"], apps["deploy:order:order-api"], {
            "edge_id": "e003", "relationship_type": "DEPLOYED_AS",
            "relationship_name": "部署为", "dependency_strength": "强",
        }),
        MockNeo4jRel("e100", "CONTAINS", apps["deploy:order:order-api"], apps["pod:order:order-api-xxx"], {
            "edge_id": "e100", "relationship_type": "CONTAINS",
            "relationship_name": "包含", "dependency_strength": "强",
        }),
        MockNeo4jRel("e101", "SCHEDULED_ON", apps["pod:order:order-api-xxx"], apps["node:worker-01"], {
            "edge_id": "e101", "relationship_type": "SCHEDULED_ON",
            "relationship_name": "调度在", "dependency_strength": "强",
        }),
    ]


@pytest.fixture
def sample_path(sample_nodes, sample_edges):
    apps = {n._id: n for n in sample_nodes}
    path_nodes = [
        apps["app:order"], apps["comp:order-api"], apps["deploy:order:order-api"],
        apps["pod:order:order-api-xxx"], apps["node:worker-01"],
    ]
    return MockNeo4jPath(path_nodes, sample_edges[:4])


@pytest.fixture
def client():
    """带 mock Neo4j 的 FastAPI TestClient"""
    # 先 import，让 neo4j_client 模块挂到 sys.modules 上
    import app.db.neo4j_client as n4j

    with patch.object(n4j, "get_driver", return_value=MagicMock()), \
         patch.object(n4j, "run_query", return_value=[]) as mock_run_query, \
         patch.object(n4j, "check_connection", return_value=True):
        from app.main import app
        yield TestClient(app), mock_run_query
