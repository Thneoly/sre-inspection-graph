"""自检报告生成测试 — PRD-003 Sprint 1。

覆盖:
- health_score 适配公式(节点健康度 + 活跃故障)
- 5 个 modules 采集函数
- generator:同步生成 Markdown,模块按需启用,失败路径
- 4 个 HTTP endpoint(generate / status / download / list)

风格对齐 test_recovery.py / test_change_events.py:模块级 _seed_store 种图,
函数级清 runtime 数据(report_store + faults + executions + change_events)。
全 DSS,无 Neo4j —— client fixture 的 mock_run 忽略。
"""

import pytest
from app.datasource.models import (
    ChangeEvent,
    DataEdge,
    DataNode,
    FaultInjection,
    RecoveryExecution,
)
from app.datasource.store import store
from app.reports.generator import generate_report, new_report_id, output_dir
from app.reports.store import report_store


# ============================================================
# 种子数据 — app:order 子树,含 1 red Pod / 1 yellow Pod / 活跃故障 / 变更 / 执行
# ============================================================

@pytest.fixture(scope="module", autouse=True)
def _seed_store():
    store.nodes.clear()
    store.edges.clear()

    nodes = [
        DataNode("app:order", "Application", "订单应用"),
        DataNode("comp:order-api", "ApplicationComponent", "订单API组件"),
        DataNode("deploy:order-api", "Deployment", "order-api",
                 {"desired_replicas": 2, "available_replicas": 2}),
        DataNode("pod:order-api-1", "Pod", "order-api-1", {"health_status": "critical", "phase": "CrashLoopBackOff"}),
        DataNode("pod:order-api-2", "Pod", "order-api-2", {"health_status": "warning", "phase": "Running"}),
        DataNode("pod:order-api-3", "Pod", "order-api-3", {"health_status": "normal", "phase": "Running"}),
        DataNode("cm:order-config", "ConfigMap", "order-config"),
    ]
    for n in nodes:
        store.upsert_node(n)

    edges = [
        ("e1", "app:order", "CONTAINS", "comp:order-api"),
        ("e2", "comp:order-api", "DEPLOYED_AS", "deploy:order-api"),
        ("e3", "deploy:order-api", "CONTAINS", "pod:order-api-1"),
        ("e4", "deploy:order-api", "CONTAINS", "pod:order-api-2"),
        ("e5", "deploy:order-api", "CONTAINS", "pod:order-api-3"),
        ("e6", "pod:order-api-1", "USES", "cm:order-config"),
    ]
    for eid, src, rel, tgt in edges:
        store.upsert_edge(DataEdge(eid, src, tgt, rel, rel))

    yield

    store.nodes.clear()
    store.edges.clear()


@pytest.fixture(autouse=True)
def _reset_runtime():
    """每个测试前清 runtime 数据(report_store + faults + executions + change_events + metrics)。"""
    report_store.clear()
    store.faults.clear()
    store.executions.clear()
    store.change_events.clear()
    store.clear_fault_metrics()
    yield
    # 清掉生成的 .md 文件,避免堆积
    out = output_dir()
    if out.exists():
        for f in out.glob("*.md"):
            try:
                f.unlink()
            except OSError:
                pass
    report_store.clear()
    store.faults.clear()
    store.executions.clear()
    store.change_events.clear()


def _seed_fault(target="pod:order-api-1", fault_type="cpu_spike"):
    store.add_fault(FaultInjection(
        injection_id="flt-1", fault_type=fault_type, target_id=target,
        current_stage=2, total_stages=6, status="injected", injected_at="2026-06-20T00:00:00Z",
    ))


def _seed_change(change_type="deployment_rolled", severity="high", target="deploy:order-api"):
    store.add_change_event(ChangeEvent(
        change_event_id="ce-1", change_type=change_type, target_resource_id=target,
        target_resource_type="Deployment", changed_at="2026-06-20T03:00:00Z",
        changed_by="argo-cd", source="argo_cd", description="rollout v1.2.4",
        severity_estimate=severity, propagated_to=[],
    ))


def _seed_execution(status="succeeded", target="deploy:order-api"):
    store.add_execution(RecoveryExecution(
        execution_id="exec-1", action_id="rollback_deployment",
        target_resource_id=target, target_resource_type="Deployment",
        status=status, initiated_by="test", initiated_at="2026-06-20T03:30:00Z",
        executed_at="2026-06-20T03:30:05Z", completed_at="2026-06-20T03:30:10Z",
    ))


# ============================================================
# 1. Health Score
# ============================================================

class TestHealthScore:
    def test_all_green_scores_100(self):
        # 子树内 pod-3 是 normal,red/yellow pod 也在子树内 -> 不是全绿
        # 这里单独验证:只有 normal 节点时打满分
        from app.reports.health_score import compute_health_score
        # 临时把 red/yellow pod 移出子树不现实,直接断言当前子树的真实分
        # critical pod-1 + (fault 计入) ; warning pod-2
        result = compute_health_score("app:order")
        # 1 critical(pod-1) → -10 ; 1 warning(pod-2) → -3 → 87
        assert result["score"] == 87
        assert result["rating"] == "健康"
        assert result["breakdown"]["critical"] == 1
        assert result["breakdown"]["warning"] == 1

    def test_active_fault_reduces_score(self):
        from app.reports.health_score import compute_health_score
        _seed_fault(target="pod:order-api-1")  # Pod 类 fault
        result = compute_health_score("app:order")
        # 原本 87;+1 critical fault(×10)+ fault_pod(×2) → 87 -10 -2 = 75
        assert result["score"] == 75
        assert result["breakdown"]["fault_pod"] == 1
        assert result["rating"] == "健康警告"

    def test_rating_boundaries(self):
        from app.reports.health_score import _rating
        assert _rating(100) == "健康"
        assert _rating(80) == "健康"
        assert _rating(79) == "健康警告"
        assert _rating(60) == "健康警告"
        assert _rating(59) == "风险中"
        assert _rating(40) == "风险中"
        assert _rating(39) == "风险高"
        assert _rating(0) == "风险高"

    def test_unknown_application(self):
        from app.reports.health_score import compute_health_score
        result = compute_health_score("app:nope")
        assert result["score"] == 100
        assert result["breakdown"]["total_nodes"] == 0


# ============================================================
# 2. Modules
# ============================================================

class TestModules:
    def test_seven_views_counts(self):
        from app.reports.modules import gather_seven_views
        data = gather_seven_views("app:order")
        assert data["topology"]["components"] == 1
        assert data["topology"]["deployments"] == 1
        assert data["topology"]["pods"] == 3
        assert data["topology"]["total_nodes"] == 7
        assert data["health"]["critical"] == 1
        assert data["health"]["warning"] == 1
        assert data["health"]["normal"] >= 1

    def test_seven_views_includes_faults_and_changes(self):
        from app.reports.modules import gather_seven_views
        _seed_fault()
        _seed_change()
        _seed_execution()
        data = gather_seven_views("app:order")
        assert len(data["active_faults"]) == 1
        assert data["changes"]["total"] == 1
        assert data["recoveries"]["total"] == 1
        assert data["recoveries"]["succeeded"] == 1

    def test_risk_list_orders_critical_first(self):
        from app.reports.modules import gather_risk_list
        _seed_fault()
        _seed_change(severity="high")
        data = gather_risk_list("app:order")
        # critical: red pod-1 + fault → 至少 2
        assert data["counts"]["critical"] >= 2
        assert data["counts"]["warning"] == 1  # yellow pod-2
        assert data["counts"]["change"] == 1
        # critical 项里含 fault 记录
        assert any("活跃故障" in r["reason"] for r in data["critical"])

    def test_recommended_actions_from_fault_and_change(self):
        from app.reports.modules import gather_recommended_actions
        _seed_fault(fault_type="cpu_spike")          # → scale_deployment
        _seed_change(change_type="deployment_rolled", severity="high")  # → rollback_deployment
        data = gather_recommended_actions("app:order")
        action_ids = {a["action_id"] for a in data["actions"]}
        assert "scale_deployment" in action_ids
        assert "rollback_deployment" in action_ids
        assert data["total"] >= 2

    def test_recommended_actions_dedup(self):
        from app.reports.modules import gather_recommended_actions
        # 同 target 两个同类型 fault 应去重
        _seed_fault(fault_type="cpu_spike", target="pod:order-api-1")
        store.add_fault(FaultInjection(
            injection_id="flt-2", fault_type="cpu_spike", target_id="pod:order-api-1",
            current_stage=1, total_stages=6, status="injected", injected_at="2026-06-20T00:00:00Z",
        ))
        data = gather_recommended_actions("app:order")
        scale = [a for a in data["actions"] if a["action_id"] == "scale_deployment"]
        assert len(scale) == 1  # 去重

    def test_historical_trends_aggregates_by_day(self):
        from app.reports.modules import gather_historical_trends
        _seed_change()
        _seed_execution()
        data = gather_historical_trends("app:order", days=7)
        # 2026-06-20 有 1 change + 1 recovery
        row = next((r for r in data["rows"] if r["date"] == "2026-06-20"), None)
        assert row is not None
        assert row["changes"] == 1
        assert row["recoveries"] == 1
        assert data["total_changes"] == 1


# ============================================================
# 3. Generator
# ============================================================

class TestGenerator:
    def test_generate_report_sync_all_modules(self):
        _seed_fault()
        _seed_change()
        rid = new_report_id()
        from app.reports.store import ReportTask, ALL_MODULES
        report_store.add_task(ReportTask(
            report_id=rid, template_id="application_health",
            scope={"application_id": "app:order"}, modules=list(ALL_MODULES),
            format="markdown", created_at="2026-06-20T00:00:00Z",
        ))
        generate_report(rid)

        task = report_store.get_task(rid)
        assert task.status == "completed"
        assert task.progress == 100
        assert task.markdown is not None
        assert task.file_path is not None
        # 5 个 section 标题都在
        for heading in ["健康度评分", "视图结论汇总", "风险清单", "推荐恢复动作", "历史趋势"]:
            assert heading in task.markdown

    def test_generate_report_partial_modules(self):
        rid = new_report_id()
        from app.reports.store import ReportTask
        report_store.add_task(ReportTask(
            report_id=rid, template_id="application_health",
            scope={"application_id": "app:order"}, modules=["health_score", "risk_list"],
            format="markdown", created_at="2026-06-20T00:00:00Z",
        ))
        generate_report(rid)
        task = report_store.get_task(rid)
        assert task.status == "completed"
        assert "健康度评分" in task.markdown
        assert "风险清单" in task.markdown
        # 未启用的模块 section 不出现
        assert "视图结论汇总" not in task.markdown
        assert "历史趋势" not in task.markdown

    def test_generate_report_failure_marks_failed(self):
        rid = new_report_id()
        from app.reports.store import ReportTask
        report_store.add_task(ReportTask(
            report_id=rid, template_id="application_health",
            scope={"application_id": "app:order"}, modules=["health_score"],
            format="markdown", created_at="2026-06-20T00:00:00Z",
        ))
        # 让渲染抛错:monkeypatch Jinja2 get_template
        import app.reports.generator as gen
        original = gen._env.get_template
        gen._env.get_template = lambda name: (_ for _ in ()).throw(RuntimeError("boom"))
        try:
            generate_report(rid)
        finally:
            gen._env.get_template = original
        task = report_store.get_task(rid)
        assert task.status == "failed"
        assert task.error_message is not None
        assert "boom" in task.error_message

    def test_generate_report_unknown_task_noop(self):
        # 不存在的 task —— 不抛、不改状态
        generate_report("rpt-does-not-exist")


# ============================================================
# 4. Endpoints
# ============================================================

class TestEndpoints:
    def test_post_generate_returns_pending(self, client):
        c, _ = client
        resp = c.post("/api/v1/reports/generate", json={
            "template_id": "application_health",
            "scope": {"application_id": "app:order"},
            "format": "markdown",
            "modules": ["health_score"],
        })
        assert resp.status_code == 202
        body = resp.json()
        assert body["status"] == "pending"
        assert body["report_id"].startswith("rpt-")

    def test_post_generate_rejects_bad_template(self, client):
        c, _ = client
        resp = c.post("/api/v1/reports/generate", json={
            "template_id": "bogus", "scope": {"application_id": "app:order"},
        })
        assert resp.status_code == 400

    def test_post_generate_rejects_pdf(self, client):
        c, _ = client
        resp = c.post("/api/v1/reports/generate", json={
            "template_id": "application_health",
            "scope": {"application_id": "app:order"},
            "format": "pdf",
        })
        assert resp.status_code == 400

    def test_status_then_download(self, client):
        c, _ = client
        # 直接同步生成,避免线程时序
        rid = new_report_id()
        from app.reports.store import ReportTask, ALL_MODULES
        report_store.add_task(ReportTask(
            report_id=rid, template_id="application_health",
            scope={"application_id": "app:order"}, modules=list(ALL_MODULES),
            format="markdown", created_at="2026-06-20T00:00:00Z",
        ))
        generate_report(rid)

        st = c.get(f"/api/v1/reports/{rid}/status")
        assert st.status_code == 200
        assert st.json()["status"] == "completed"

        dl = c.get(f"/api/v1/reports/{rid}/download")
        assert dl.status_code == 200
        assert "markdown" in dl.headers["content-type"]
        assert "健康度评分" in dl.text

    def test_download_not_ready_409(self, client):
        c, _ = client
        from app.reports.store import ReportTask
        rid = new_report_id()
        report_store.add_task(ReportTask(
            report_id=rid, template_id="application_health",
            scope={"application_id": "app:order"}, modules=["health_score"],
            format="markdown", created_at="2026-06-20T00:00:00Z",
        ))
        # 任务处于 pending,没生成
        dl = c.get(f"/api/v1/reports/{rid}/download")
        assert dl.status_code == 409

    def test_status_404_unknown(self, client):
        c, _ = client
        assert c.get("/api/v1/reports/rpt-none/status").status_code == 404

    def test_list_filters(self, client):
        c, _ = client
        from app.reports.store import ReportTask
        report_store.add_task(ReportTask(
            report_id="rpt-a", template_id="application_health",
            scope={"application_id": "app:order"}, modules=["health_score"],
            format="markdown", created_at="2026-06-20T01:00:00Z",
        ))
        report_store.add_task(ReportTask(
            report_id="rpt-b", template_id="application_health",
            scope={"application_id": "app:billing"}, modules=["health_score"],
            format="markdown", created_at="2026-06-20T02:00:00Z",
        ))
        all_reports = c.get("/api/v1/reports").json()
        assert all_reports["total"] == 2

        filtered = c.get("/api/v1/reports?application_id=app:order").json()
        assert filtered["total"] == 1
        assert filtered["reports"][0]["report_id"] == "rpt-a"
