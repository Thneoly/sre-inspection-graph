"""自检报告 Sprint 2 测试 — PRD-003。

覆盖:
- cluster_overview 4 模块(cluster_health / risk_top_n / changes / recoveries)
- incident_report 3 模块 + anchor 解析(fault_id / change_event_id)
- generator 多模板路由(application_health / cluster_overview / incident_report)
- /generate 端点 modules 按模板校验

订阅 / 调度 / 邮件 / persistence 走 Commit 2 的独立测试文件 test_reports_sprint2_sub.py。
"""

from __future__ import annotations

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
from app.reports.store import ReportTask, report_store


# ============================================================
# 种子:2 个应用(app:order + app:cart),覆盖跨应用聚合
# ============================================================

@pytest.fixture(scope="module", autouse=True)
def _seed_store():
    store.nodes.clear()
    store.edges.clear()

    nodes = [
        # 集群 1: vm-cluster(app:order)
        DataNode("app:vm-cluster:order", "Application", "订单应用",
                 {"health_status": "warning"}),
        DataNode("comp:vm-cluster:order-api", "ApplicationComponent", "订单API组件",
                 {"health_status": "warning"}),
        DataNode("deploy:vm-cluster:order-api", "Deployment", "order-api",
                 {"desired_replicas": 2, "available_replicas": 2}),
        DataNode("pod:vm-cluster:order-api-1", "Pod", "order-api-1",
                 {"health_status": "critical", "phase": "CrashLoopBackOff"}),
        DataNode("pod:vm-cluster:order-api-2", "Pod", "order-api-2",
                 {"health_status": "warning", "phase": "Running"}),

        # 集群 1: vm-cluster(app:cart) — 全绿
        DataNode("app:vm-cluster:cart", "Application", "购物车应用",
                 {"health_status": "normal"}),
        DataNode("comp:vm-cluster:cart-api", "ApplicationComponent", "购物车组件",
                 {"health_status": "normal"}),
        DataNode("deploy:vm-cluster:cart-api", "Deployment", "cart-api"),
        DataNode("pod:vm-cluster:cart-api-1", "Pod", "cart-api-1",
                 {"health_status": "normal", "phase": "Running"}),

        # 集群 2: prod-cluster(app:billing) — 用于 prefix 过滤验证
        DataNode("app:prod-cluster:billing", "Application", "账单应用",
                 {"health_status": "warning"}),
    ]
    for n in nodes:
        store.upsert_node(n)

    edges = [
        ("e1", "app:vm-cluster:order", "CONTAINS", "comp:vm-cluster:order-api"),
        ("e2", "comp:vm-cluster:order-api", "DEPLOYED_AS", "deploy:vm-cluster:order-api"),
        ("e3", "deploy:vm-cluster:order-api", "CONTAINS", "pod:vm-cluster:order-api-1"),
        ("e4", "deploy:vm-cluster:order-api", "CONTAINS", "pod:vm-cluster:order-api-2"),
        ("e5", "app:vm-cluster:cart", "CONTAINS", "comp:vm-cluster:cart-api"),
        ("e6", "comp:vm-cluster:cart-api", "DEPLOYED_AS", "deploy:vm-cluster:cart-api"),
        ("e7", "deploy:vm-cluster:cart-api", "CONTAINS", "pod:vm-cluster:cart-api-1"),
    ]
    for eid, src, rel, tgt in edges:
        store.upsert_edge(DataEdge(eid, src, tgt, rel, rel))

    yield

    store.nodes.clear()
    store.edges.clear()


@pytest.fixture(autouse=True)
def _reset_runtime():
    report_store.clear()
    store.faults.clear()
    store.executions.clear()
    store.change_events.clear()
    store.clear_fault_metrics()
    yield
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


def _seed_fault(injection_id="flt-1", target="pod:vm-cluster:order-api-1",
                fault_type="cpu_spike", at="2026-06-20T03:00:00Z"):
    f = FaultInjection(
        injection_id=injection_id, fault_type=fault_type, target_id=target,
        current_stage=2, total_stages=6, status="injected", injected_at=at,
    )
    store.add_fault(f)
    return f


def _seed_change(ceid="ce-1", change_type="deployment_rolled", severity="high",
                 target="deploy:vm-cluster:order-api", at="2026-06-20T03:00:00Z"):
    e = ChangeEvent(
        change_event_id=ceid, change_type=change_type, target_resource_id=target,
        target_resource_type="Deployment", changed_at=at,
        changed_by="argo-cd", source="argo_cd", description="rollout v1.2.4",
        severity_estimate=severity, propagated_to=[],
    )
    store.add_change_event(e)
    return e


def _seed_execution(eid="exec-1", target="deploy:vm-cluster:order-api",
                    status="succeeded", at="2026-06-20T03:30:00Z"):
    e = RecoveryExecution(
        execution_id=eid, action_id="rollback_deployment",
        target_resource_id=target, target_resource_type="Deployment",
        status=status, initiated_by="test", initiated_at=at,
        executed_at=at, completed_at=at,
    )
    store.add_execution(e)
    return e


# ============================================================
# 1. Cluster modules
# ============================================================

class TestClusterModules:
    def test_health_aggregates_all_apps(self):
        from app.reports.cluster_modules import gather_cluster_health
        data = gather_cluster_health()
        # 3 个应用:app:vm-cluster:order(warning + 1 critical pod), app:vm-cluster:cart(全绿), app:prod-cluster:billing(无子树)
        assert data["total_apps"] == 3
        ids = [a["application_id"] for a in data["apps"]]
        assert "app:vm-cluster:order" in ids
        assert "app:vm-cluster:cart" in ids
        # apps 按 score 升序
        scores = [a["score"] for a in data["apps"]]
        assert scores == sorted(scores)

    def test_health_cluster_prefix_filters(self):
        from app.reports.cluster_modules import gather_cluster_health
        # 仅 vm-cluster
        data = gather_cluster_health(cluster_id="vm-cluster")
        ids = {a["application_id"] for a in data["apps"]}
        assert "app:vm-cluster:order" in ids
        assert "app:vm-cluster:cart" in ids
        assert "app:prod-cluster:billing" not in ids

    def test_risk_top_n_truncates(self):
        from app.reports.cluster_modules import gather_cluster_risk_top_n
        data = gather_cluster_risk_top_n(top_n=2)
        assert data["top_n"] == 2
        assert len(data["top_apps"]) <= 2

    def test_risk_top_n_counts_faults_and_changes(self):
        from app.reports.cluster_modules import gather_cluster_risk_top_n
        _seed_fault()
        _seed_change()
        data = gather_cluster_risk_top_n(cluster_id="vm-cluster")
        assert data["active_faults_total"] == 1
        assert data["high_severity_changes_total"] == 1

    def test_cluster_changes_aggregates_by_type(self):
        from app.reports.cluster_modules import gather_cluster_changes
        _seed_change(ceid="ce-a", change_type="deployment_rolled")
        _seed_change(ceid="ce-b", change_type="configmap_updated", target="deploy:vm-cluster:cart-api")
        _seed_change(ceid="ce-c", change_type="deployment_rolled", target="deploy:vm-cluster:cart-api")
        data = gather_cluster_changes(cluster_id="vm-cluster")
        assert data["total"] == 3
        assert data["by_type"]["deployment_rolled"] == 2
        assert data["by_type"]["configmap_updated"] == 1
        # Top-5 targets
        assert len(data["top_targets"]) >= 2

    def test_cluster_recoveries_success_rate(self):
        from app.reports.cluster_modules import gather_cluster_recoveries
        _seed_execution(eid="e-1", status="succeeded")
        _seed_execution(eid="e-2", status="failed")
        _seed_execution(eid="e-3", status="succeeded")
        data = gather_cluster_recoveries(cluster_id="vm-cluster")
        assert data["total"] == 3
        assert data["status_counts"]["succeeded"] == 2
        assert data["status_counts"]["failed"] == 1
        # 2/3 ≈ 0.667
        assert abs(data["success_rate"] - 0.667) < 0.01


# ============================================================
# 2. Incident modules
# ============================================================

class TestIncidentModules:
    def test_anchor_resolution_fault(self):
        from app.reports.incident_modules import gather_incident_summary
        _seed_fault(injection_id="flt-x", target="pod:vm-cluster:order-api-1")
        data = gather_incident_summary({"fault_id": "flt-x"})
        assert data["kind"] == "fault"
        assert data["target_id"] == "pod:vm-cluster:order-api-1"
        assert data["target_type"] == "Pod"

    def test_anchor_resolution_change(self):
        from app.reports.incident_modules import gather_incident_summary
        _seed_change(ceid="ce-x", target="deploy:vm-cluster:order-api")
        data = gather_incident_summary({"change_event_id": "ce-x"})
        assert data["kind"] == "change"
        assert data["target_id"] == "deploy:vm-cluster:order-api"

    def test_missing_anchor_raises(self):
        from app.reports.incident_modules import gather_incident_summary
        with pytest.raises(ValueError):
            gather_incident_summary({})

    def test_unknown_fault_raises(self):
        from app.reports.incident_modules import gather_incident_summary
        with pytest.raises(ValueError):
            gather_incident_summary({"fault_id": "flt-nope"})

    def test_timeline_within_window(self):
        from app.reports.incident_modules import gather_incident_timeline
        _seed_fault(injection_id="flt-tl", target="pod:vm-cluster:order-api-1",
                    at="2026-06-20T03:00:00Z")
        # Pod 反向 BFS 命中 deployment;那个 deployment 上 5min 后有变更 → 应入时间线
        _seed_change(ceid="ce-near", target="deploy:vm-cluster:order-api",
                     at="2026-06-20T03:05:00Z")
        # 远程变更(2h 外)→ 不入
        _seed_change(ceid="ce-far", target="deploy:vm-cluster:order-api",
                     at="2026-06-20T06:00:00Z")
        data = gather_incident_timeline({"fault_id": "flt-tl"}, window_seconds=3600)
        timestamps = [it["timestamp"] for it in data["events"]]
        assert "2026-06-20T03:05:00Z" in timestamps
        assert "2026-06-20T06:00:00Z" not in timestamps

    def test_recoveries_for_change_anchor_recommend(self):
        from app.reports.incident_modules import gather_incident_recoveries
        _seed_change(ceid="ce-r", change_type="deployment_rolled",
                     target="deploy:vm-cluster:order-api")
        data = gather_incident_recoveries({"change_event_id": "ce-r"})
        # change 锚点 → 应有推荐 rollback_deployment
        ids = {a["action_id"] for a in data["recommended"]}
        assert "rollback_deployment" in ids


# ============================================================
# 3. Generator 多模板路由
# ============================================================

class TestGeneratorMultiTemplate:
    def _new_task(self, template_id, scope=None, modules=None):
        from app.reports.store import modules_for_template
        rid = new_report_id()
        report_store.add_task(ReportTask(
            report_id=rid, template_id=template_id,
            scope=scope or {}, modules=modules or list(modules_for_template(template_id)),
            format="markdown", status="pending", created_at="2026-06-20T03:00:00Z",
        ))
        return rid

    def test_cluster_overview_renders(self):
        rid = self._new_task("cluster_overview", scope={"cluster_id": "vm-cluster"})
        generate_report(rid)
        task = report_store.get_task(rid)
        assert task.status == "completed", task.error_message
        assert task.markdown is not None
        md = task.markdown
        assert "集群健康总览" in md
        assert "健康分布" in md
        assert "Top-" in md

    def test_incident_report_renders_with_fault(self):
        _seed_fault(injection_id="flt-ir", target="pod:vm-cluster:order-api-1")
        rid = self._new_task("incident_report", scope={"fault_id": "flt-ir"})
        generate_report(rid)
        task = report_store.get_task(rid)
        assert task.status == "completed", task.error_message
        assert "事件报告" in task.markdown
        assert "事件摘要" in task.markdown
        assert "时间线" in task.markdown

    def test_incident_report_with_change(self):
        _seed_change(ceid="ce-ir", change_type="deployment_rolled",
                     target="deploy:vm-cluster:order-api")
        rid = self._new_task("incident_report", scope={"change_event_id": "ce-ir"})
        generate_report(rid)
        task = report_store.get_task(rid)
        assert task.status == "completed", task.error_message
        assert "推荐后续" in task.markdown

    def test_incident_unresolvable_marks_failed(self):
        rid = self._new_task("incident_report", scope={"fault_id": "flt-nope"})
        generate_report(rid)
        task = report_store.get_task(rid)
        assert task.status == "failed"
        assert "not found" in (task.error_message or "")

    def test_partial_modules_omit_sections(self):
        rid = self._new_task("cluster_overview",
                             scope={"cluster_id": "vm-cluster"},
                             modules=["cluster_health"])
        generate_report(rid)
        task = report_store.get_task(rid)
        assert task.status == "completed"
        assert "健康分布" in task.markdown
        # 其他 section 跳过
        assert "Top-" not in task.markdown
        assert "变更汇总" not in task.markdown


# ============================================================
# 4. Endpoints 校验:按模板的 modules
# ============================================================

class TestGenerateEndpointMultiTemplate:
    def test_application_health_requires_application_id(self, client):
        tc, _ = client
        r = tc.post("/api/v1/reports/generate", json={
            "template_id": "application_health",
            "scope": {},  # 无 application_id
            "format": "markdown",
        })
        assert r.status_code == 400
        assert "application_id" in r.json()["detail"]

    def test_incident_requires_anchor(self, client):
        tc, _ = client
        r = tc.post("/api/v1/reports/generate", json={
            "template_id": "incident_report",
            "scope": {},
            "format": "markdown",
        })
        assert r.status_code == 400
        assert "fault_id" in r.json()["detail"] or "change_event_id" in r.json()["detail"]

    def test_invalid_module_for_template(self, client):
        tc, _ = client
        # cluster_overview 模板传 application_health 的模块
        r = tc.post("/api/v1/reports/generate", json={
            "template_id": "cluster_overview",
            "scope": {"cluster_id": "vm-cluster"},
            "format": "markdown",
            "modules": ["health_score"],
        })
        assert r.status_code == 400
        assert "unknown modules" in r.json()["detail"]

    def test_cluster_overview_default_modules(self, client):
        tc, _ = client
        r = tc.post("/api/v1/reports/generate", json={
            "template_id": "cluster_overview",
            "scope": {"cluster_id": "vm-cluster"},
            "format": "markdown",
        })
        assert r.status_code == 202
        assert r.json()["status"] == "pending"

    def test_download_rejects_pdf_format(self, client):
        tc, _ = client
        # 先建一个 completed
        rid = new_report_id()
        report_store.add_task(ReportTask(
            report_id=rid, template_id="application_health",
            scope={"application_id": "app:vm-cluster:order"},
            modules=["health_score"], format="markdown",
            status="completed", file_path="/tmp/nope.md",
            created_at="2026-06-20T03:00:00Z",
        ))
        r = tc.get(f"/api/v1/reports/{rid}/download?format=pdf")
        assert r.status_code == 400
