"""API 路由集成测试 — FastAPI TestClient + Mock Neo4j"""


class TestHealthEndpoint:
    def test_health_ok(self, client):
        cli, _ = client
        resp = cli.get("/api/v1/health")
        assert resp.status_code == 200
        data = resp.json()
        assert data["status"] == "ok"
        assert data["neo4j"] == "connected"


class TestTopologyEndpoint:
    def test_valid_app_code(self, client, sample_path):
        cli, mock_run = client
        mock_run.return_value = [{"path": sample_path}]

        resp = cli.get("/api/v1/topology/app/order")
        assert resp.status_code == 200
        data = resp.json()
        assert "nodes" in data
        assert "edges" in data
        assert "summary" in data

    def test_depth_validation(self, client):
        cli, _ = client
        resp = cli.get("/api/v1/topology/app/order?depth=0")
        assert resp.status_code == 422

        resp = cli.get("/api/v1/topology/app/order?depth=11")
        assert resp.status_code == 422


class TestAccessLinkEndpoint:
    def test_valid_request(self, client, sample_path):
        cli, mock_run = client
        mock_run.return_value = [{"path": sample_path}]

        resp = cli.get("/api/v1/access-link/order")
        assert resp.status_code == 200
        assert "summary" in resp.json()


class TestNodeImpactEndpoint:
    def test_valid_node_id(self, client, sample_path):
        cli, mock_run = client
        mock_run.return_value = [{"path": sample_path}]

        resp = cli.get("/api/v1/node-impact/node:cce-prod-01:worker-01")
        assert resp.status_code == 200

    def test_depth_validation(self, client):
        cli, _ = client
        resp = cli.get("/api/v1/node-impact/node:x?depth=0")
        assert resp.status_code == 422


class TestConfigImpactEndpoint:
    def test_secret_impact(self, client, sample_path):
        cli, mock_run = client
        mock_run.return_value = [{"path": sample_path}]

        resp = cli.get("/api/v1/config-impact/secret:x:y:z")
        assert resp.status_code == 200


class TestImageRiskEndpoint:
    def test_image_risk(self, client, sample_path):
        cli, mock_run = client
        mock_run.return_value = [{"path": sample_path}]

        resp = cli.get("/api/v1/image-risk/image:order-api:1.2.3")
        assert resp.status_code == 200


class TestAlertAggregationEndpoint:
    def test_no_filter(self, client, sample_path):
        cli, mock_run = client
        mock_run.return_value = [{"path": sample_path}]

        resp = cli.get("/api/v1/alert-aggregation")
        assert resp.status_code == 200

    def test_severity_filter(self, client):
        cli, _ = client
        resp = cli.get("/api/v1/alert-aggregation?severity=critical")
        assert resp.status_code == 200

    def test_invalid_severity_ignored(self, client):
        cli, _ = client
        resp = cli.get("/api/v1/alert-aggregation?severity=INVALID")
        assert resp.status_code == 200


class TestResponseShape:
    """验证 GraphResponse 结构完整性"""

    def test_summary_has_required_fields(self, client, sample_path):
        cli, mock_run = client
        mock_run.return_value = [{"path": sample_path}]

        resp = cli.get("/api/v1/topology/app/order")
        data = resp.json()
        summary = data["summary"]
        assert "total_nodes" in summary
        assert "total_edges" in summary
        assert "risk_counts" in summary
        assert "health_counts" in summary
        assert "high" in summary["risk_counts"]
        assert "medium" in summary["risk_counts"]
        assert "low" in summary["risk_counts"]

    def test_node_has_required_fields(self, client, sample_path):
        cli, mock_run = client
        mock_run.return_value = [{"path": sample_path}]

        resp = cli.get("/api/v1/topology/app/order")
        nodes = resp.json()["nodes"]
        for node in nodes:
            assert "id" in node
            assert "label" in node
            assert "type" in node
            assert "properties" in node

    def test_edge_has_required_fields(self, client, sample_path):
        cli, mock_run = client
        mock_run.return_value = [{"path": sample_path}]

        resp = cli.get("/api/v1/topology/app/order")
        edges = resp.json()["edges"]
        for edge in edges:
            assert "id" in edge
            assert "source" in edge
            assert "target" in edge
            assert "type" in edge


class TestErrorHandling:
    def test_empty_response_handled(self, client):
        """空查询结果不应导致错误"""
        cli, _ = client
        resp = cli.get("/api/v1/topology/app/nonexistent")
        assert resp.status_code == 200
        data = resp.json()
        assert data["summary"]["total_nodes"] == 0

    def test_health_returns_expected_structure(self, client):
        """health 端点返回预期结构"""
        cli, _ = client
        resp = cli.get("/api/v1/health")
        assert resp.status_code == 200
        data = resp.json()
        assert "status" in data
        assert "neo4j" in data
        assert "version" in data
