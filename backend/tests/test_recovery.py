"""Recovery Action 测试 — PRD-001 Sprint 1。

覆盖:
- ACTION_DEFS 8 种动作的元数据完整性
- list_actions 过滤
- suggest_for_rule 推荐查询
- cascade.dry_run 正常路径 + 错误路径
- API 端点 4 个

风格对齐 test_routers.py(class-based + client fixture + mock Neo4j)。
"""

import pytest
from app.datasource.models import DataNode, DataEdge
from app.datasource.store import store


# ============================================================
# Fixture:用一组小图填充 DSS store(整个测试模块共享)
# ============================================================

@pytest.fixture(scope="module", autouse=True)
def _seed_store():
    """在 store 里塞一组 mock 图,用于 cascade 测试。

    拓扑:
        app:order
          └ comp:order-api      (BELONGS_TO 反向 = comp 在 app 下)
              └ deploy:order-api  (DEPLOYED_AS)
                  └ pod:order-api-1  (CONTAINS)
                  └ pod:order-api-2  (CONTAINS)
                      └ node:worker-1  (SCHEDULED_ON)
        svc:order-api  (ROUTES_TO pod:order-api-1, pod:order-api-2)
        secret:order-tls  (USES → pod:order-api-1)
        mysql:order-db  (USES → pod:order-api-1)
    """
    # 清场(避免被前面 startup load_baseline 污染)
    store.nodes.clear()
    store.edges.clear()

    nodes = [
        DataNode("app:order", "Application", "订单应用"),
        DataNode("comp:order-api", "ApplicationComponent", "订单API组件"),
        DataNode("deploy:order-api", "Deployment", "order-api"),
        DataNode("pod:order-api-1", "Pod", "order-api-1"),
        DataNode("pod:order-api-2", "Pod", "order-api-2"),
        DataNode("node:worker-1", "KubernetesNode", "worker-1"),
        DataNode("svc:order-api", "Service", "order-api-svc"),
        DataNode("secret:order-tls", "Secret", "order-tls"),
        DataNode("mysql:order-db", "MySQL", "order-db"),
    ]
    for n in nodes:
        store.upsert_node(n)

    # source-relationship-target(对齐 cascade 算法的 forward/reverse 语义)
    edges = [
        ("e1", "app:order", "CONTAINS", "comp:order-api"),
        ("e2", "comp:order-api", "DEPLOYED_AS", "deploy:order-api"),
        ("e3", "deploy:order-api", "CONTAINS", "pod:order-api-1"),
        ("e4", "deploy:order-api", "CONTAINS", "pod:order-api-2"),
        ("e5", "pod:order-api-2", "SCHEDULED_ON", "node:worker-1"),
        ("e6", "svc:order-api", "ROUTES_TO", "pod:order-api-1"),
        ("e7", "svc:order-api", "ROUTES_TO", "pod:order-api-2"),
        ("e8", "pod:order-api-1", "USES", "secret:order-tls"),
        ("e9", "pod:order-api-1", "USES", "mysql:order-db"),
        # BELONGS_TO 反向链:Pod → Component → Application
        ("e10", "deploy:order-api", "BELONGS_TO", "comp:order-api"),
        ("e11", "comp:order-api", "BELONGS_TO", "app:order"),
    ]
    for eid, src, rel, tgt in edges:
        store.upsert_edge(DataEdge(eid, src, tgt, rel, rel))

    yield

    # 清场不影响其他测试
    store.nodes.clear()
    store.edges.clear()


# ============================================================
# 1. action_defs 元数据完整性
# ============================================================

class TestActionDefs:
    def test_eight_actions_defined(self):
        from app.recovery.action_defs import ACTION_DEFS
        expected = {
            "restart_pod", "scale_deployment", "rollback_deployment",
            "refresh_secret", "drain_node", "kill_query",
            "restart_service", "clear_cache",
        }
        assert set(ACTION_DEFS.keys()) == expected

    @pytest.mark.parametrize("action_id", [
        "restart_pod", "scale_deployment", "rollback_deployment",
        "refresh_secret", "drain_node", "kill_query",
        "restart_service", "clear_cache",
    ])
    def test_required_fields(self, action_id):
        from app.recovery.action_defs import ACTION_DEFS
        a = ACTION_DEFS[action_id]
        # 元数据必填
        for field in ("name", "category", "target_type", "risk_level",
                      "requires_approval", "estimated_duration_seconds",
                      "description", "input_schema",
                      "propagation", "sla_impact_estimate", "warnings"):
            assert field in a, f"{action_id} missing field: {field}"
        # 风险级别合法
        assert a["risk_level"] in ("low", "medium", "high")
        # propagation 是非空 list
        assert isinstance(a["propagation"], list) and len(a["propagation"]) > 0
        # 每条 propagation 规则有必备字段
        for rule in a["propagation"]:
            assert "edge" in rule and "direction" in rule
            assert rule["direction"] in ("forward", "reverse")
            assert rule.get("impact", "low") in ("minimal", "low", "medium", "high")

    def test_high_risk_requires_approval(self):
        """约束:high_risk 动作必须 requires_approval=True。"""
        from app.recovery.action_defs import ACTION_DEFS
        for action_id, a in ACTION_DEFS.items():
            if a["risk_level"] == "high":
                assert a["requires_approval"] is True, \
                    f"{action_id} is high_risk but requires_approval is False"


class TestListActions:
    def test_list_all(self):
        from app.recovery.action_defs import list_actions
        actions = list_actions()
        assert len(actions) == 8

    def test_filter_by_target_type(self):
        from app.recovery.action_defs import list_actions
        deployments = list_actions(target_type="Deployment")
        ids = {a["action_id"] for a in deployments}
        assert ids == {"scale_deployment", "rollback_deployment"}

    def test_filter_by_risk_level(self):
        from app.recovery.action_defs import list_actions
        high = list_actions(risk_level="high")
        ids = {a["action_id"] for a in high}
        assert ids == {"rollback_deployment", "drain_node"}


class TestSuggestForRule:
    def test_known_rule(self):
        from app.recovery.action_defs import suggest_for_rule
        sugs = suggest_for_rule("rule-001")
        # rule-001 推荐 scale_deployment + restart_pod
        ids = [s["action_id"] for s in sugs]
        assert "scale_deployment" in ids
        assert "restart_pod" in ids
        # 含 confidence 和 rationale
        for s in sugs:
            assert "confidence" in s and 0 < s["confidence"] <= 1.0
            assert "rationale" in s and len(s["rationale"]) > 0

    def test_unknown_rule(self):
        from app.recovery.action_defs import suggest_for_rule
        assert suggest_for_rule("rule-9999") == []


# ============================================================
# 2. cascade.dry_run 算法
# ============================================================

class TestDryRun:
    def test_unknown_action(self):
        from app.recovery.cascade import dry_run
        result = dry_run("nonexistent_action", "pod:x")
        assert result["target_valid"] is False
        assert "unknown action_id" in result["validation_error"]

    def test_target_not_in_store(self):
        from app.recovery.cascade import dry_run
        result = dry_run("restart_pod", "pod:does-not-exist")
        assert result["target_valid"] is False
        assert "not found" in result["validation_error"]

    def test_target_type_mismatch(self):
        """对 Deployment 用 restart_pod(目标类型不匹配)→ target_valid=False。"""
        from app.recovery.cascade import dry_run
        result = dry_run("restart_pod", "deploy:order-api")
        assert result["target_valid"] is False
        assert "Pod" in result["validation_error"]
        assert "Deployment" in result["validation_error"]

    def test_scale_deployment_propagation(self):
        """scale_deployment 应该影响 Pod(forward CONTAINS)+ Component/App(BELONGS_TO)。"""
        from app.recovery.cascade import dry_run
        result = dry_run("scale_deployment", "deploy:order-api",
                         input_params={"replicas_delta": 2})
        assert result["target_valid"] is True
        ids = {r["resource_id"] for r in result["affected_resources"]}
        # 应该至少看到 2 个 Pod
        assert "pod:order-api-1" in ids
        assert "pod:order-api-2" in ids
        # BELONGS_TO 链向上传到 component / application
        assert "comp:order-api" in ids or "app:order" in ids
        # 自身不应被列(deploy:order-api)
        assert "deploy:order-api" not in ids
        # 回滚参数
        assert result["rollback_input_params"] == {"replicas_delta": -2}

    def test_restart_pod_propagation(self):
        """restart_pod 应该波及 Service(reverse ROUTES_TO)。"""
        from app.recovery.cascade import dry_run
        result = dry_run("restart_pod", "pod:order-api-1")
        assert result["target_valid"] is True
        ids = {r["resource_id"] for r in result["affected_resources"]}
        # Service 通过 reverse ROUTES_TO 命中
        assert "svc:order-api" in ids

    def test_drain_node_propagation(self):
        """drain_node 应该列出所有 SCHEDULED_ON 此节点的 Pod。"""
        from app.recovery.cascade import dry_run
        result = dry_run("drain_node", "node:worker-1")
        assert result["target_valid"] is True
        ids = {r["resource_id"] for r in result["affected_resources"]}
        # pod:order-api-2 调度在 worker-1
        assert "pod:order-api-2" in ids

    def test_refresh_secret_propagation(self):
        """refresh_secret 应该波及所有 USES 它的 Pod。"""
        from app.recovery.cascade import dry_run
        result = dry_run("refresh_secret", "secret:order-tls")
        assert result["target_valid"] is True
        ids = {r["resource_id"] for r in result["affected_resources"]}
        assert "pod:order-api-1" in ids

    def test_severity_aggregation(self):
        """同一节点被多条 propagation 规则命中时,严重度取较大值。"""
        from app.recovery.cascade import dry_run
        result = dry_run("rollback_deployment", "deploy:order-api")
        assert result["target_valid"] is True
        # rollback_deployment 的 propagation 列表中 Pod 是 medium
        pods = [r for r in result["affected_resources"] if r["type"] == "Pod"]
        assert all(p["impact_severity"] in ("medium", "high") for p in pods)


# ============================================================
# 3. API 端点
# ============================================================

class TestRecoveryActionsEndpoint:
    def test_list_all(self, client):
        cli, _ = client
        resp = cli.get("/api/v1/recovery/actions")
        assert resp.status_code == 200
        data = resp.json()
        assert data["total"] == 8
        assert len(data["actions"]) == 8

    def test_filter_by_target_type(self, client):
        cli, _ = client
        resp = cli.get("/api/v1/recovery/actions?target_type=Pod")
        assert resp.status_code == 200
        data = resp.json()
        ids = {a["action_id"] for a in data["actions"]}
        assert ids == {"restart_pod"}

    def test_filter_invalid_risk_level(self, client):
        cli, _ = client
        resp = cli.get("/api/v1/recovery/actions?risk_level=banana")
        assert resp.status_code == 422


class TestActionDetailEndpoint:
    def test_known_action(self, client):
        cli, _ = client
        resp = cli.get("/api/v1/recovery/actions/scale_deployment")
        assert resp.status_code == 200
        data = resp.json()
        assert data["action_id"] == "scale_deployment"
        assert data["target_resource_type"] == "Deployment"
        assert "input_schema" in data

    def test_unknown_action(self, client):
        cli, _ = client
        resp = cli.get("/api/v1/recovery/actions/nonexistent")
        assert resp.status_code == 404


class TestSuggestionsEndpoint:
    def test_by_rule_id(self, client):
        cli, _ = client
        resp = cli.get("/api/v1/recovery/suggestions?rule_id=rule-001")
        assert resp.status_code == 200
        data = resp.json()
        assert data["rule_id"] == "rule-001"
        assert data["total"] >= 1
        ids = {s["action_id"] for s in data["suggestions"]}
        assert "scale_deployment" in ids

    def test_unknown_rule(self, client):
        cli, _ = client
        resp = cli.get("/api/v1/recovery/suggestions?rule_id=rule-9999")
        assert resp.status_code == 200
        data = resp.json()
        assert data["total"] == 0

    def test_missing_params(self, client):
        cli, _ = client
        resp = cli.get("/api/v1/recovery/suggestions")
        assert resp.status_code == 400

    def test_finding_id_not_implemented(self, client):
        cli, _ = client
        resp = cli.get("/api/v1/recovery/suggestions?finding_id=finding-x")
        assert resp.status_code == 501


class TestDryRunEndpoint:
    def test_valid_request(self, client):
        cli, _ = client
        resp = cli.post("/api/v1/recovery/dry-run", json={
            "action_id": "scale_deployment",
            "target_resource_id": "deploy:order-api",
            "input_params": {"replicas_delta": 2},
        })
        assert resp.status_code == 200
        data = resp.json()
        assert data["target_valid"] is True
        assert data["affected_count"] >= 1
        assert data["risk_level"] == "low"

    def test_target_type_mismatch_returns_200_with_error(self, client):
        """目标类型不匹配返 200 + target_valid=False(非 4xx,前端友好提示)。"""
        cli, _ = client
        resp = cli.post("/api/v1/recovery/dry-run", json={
            "action_id": "restart_pod",
            "target_resource_id": "deploy:order-api",
        })
        assert resp.status_code == 200
        data = resp.json()
        assert data["target_valid"] is False
        assert data["validation_error"]

    def test_finding_id_passthrough(self, client):
        cli, _ = client
        resp = cli.post("/api/v1/recovery/dry-run", json={
            "action_id": "scale_deployment",
            "target_resource_id": "deploy:order-api",
            "input_params": {"replicas_delta": 1},
            "finding_id": "finding-run2-001",
        })
        assert resp.status_code == 200
        assert resp.json()["finding_id"] == "finding-run2-001"

    def test_missing_required(self, client):
        cli, _ = client
        resp = cli.post("/api/v1/recovery/dry-run", json={
            "action_id": "scale_deployment",
            # missing target_resource_id
        })
        assert resp.status_code == 422
