"""自检报告订阅 / 调度 / 邮件 / persistence 测试 — PRD-003 Sprint 2 Commit 2。

覆盖:
- SubscriptionStore CRUD
- InMemoryEmailSender 累加
- ReportScheduler register/unregister/trigger_now
- 订阅端点(POST/GET/PATCH/DELETE/trigger)
- Neo4j dual-write 调用次数 + load_subscriptions_from_neo4j hydrate

调度器测试不真起后台线程,直接调 trigger_now 同步跑 → 写入 InMemoryEmailSender。
"""

from __future__ import annotations

from unittest.mock import MagicMock, patch

import pytest

from app.datasource.models import DataNode, DataEdge
from app.datasource.store import store
from app.reports.email_sender import InMemoryEmailSender, get_email_sender, reset_email_sender
from app.reports.scheduler import (
    ReportScheduler,
    _run_subscription_safely,
    report_scheduler as global_scheduler,
)
from app.reports.store import report_store
from app.reports.subscription_store import (
    ReportSubscription,
    new_subscription_id,
    subscription_store,
)


# ============================================================
# 种子:与 sprint2 模板测试同源,但精简(只为订阅跑通)
# ============================================================

@pytest.fixture(scope="module", autouse=True)
def _seed_min_store():
    store.nodes.clear()
    store.edges.clear()

    nodes = [
        DataNode("app:vm-cluster:order", "Application", "订单应用",
                 {"health_status": "normal"}),
        DataNode("comp:vm-cluster:order-api", "ApplicationComponent", "订单组件",
                 {"health_status": "normal"}),
        DataNode("pod:vm-cluster:order-api-1", "Pod", "order-api-1",
                 {"health_status": "normal", "phase": "Running"}),
    ]
    for n in nodes:
        store.upsert_node(n)
    store.upsert_edge(DataEdge("e1", "app:vm-cluster:order", "comp:vm-cluster:order-api",
                                "CONTAINS", "CONTAINS"))
    store.upsert_edge(DataEdge("e2", "comp:vm-cluster:order-api", "pod:vm-cluster:order-api-1",
                                "CONTAINS", "CONTAINS"))
    yield
    store.nodes.clear()
    store.edges.clear()


@pytest.fixture(autouse=True)
def _reset_runtime():
    subscription_store.clear()
    report_store.clear()
    store.faults.clear()
    store.executions.clear()
    store.change_events.clear()
    # 邮件 sender 单例重置 → InMemoryEmailSender(无 SMTP_HOST)
    reset_email_sender()
    # 全局 scheduler 清 job
    try:
        global_scheduler.scheduler.remove_all_jobs()
    except Exception:
        pass
    yield
    subscription_store.clear()
    report_store.clear()
    reset_email_sender()
    try:
        global_scheduler.scheduler.remove_all_jobs()
    except Exception:
        pass


def _make_sub(**overrides) -> ReportSubscription:
    sid = overrides.pop("subscription_id", new_subscription_id())
    base = dict(
        subscription_id=sid,
        template_id="application_health",
        scope={"application_id": "app:vm-cluster:order"},
        modules=["health_score"],
        cron="0 9 * * 1",
        recipients=["sre@example.com"],
        enabled=True,
        created_at="2026-06-20T03:00:00Z",
    )
    base.update(overrides)
    return ReportSubscription(**base)


# ============================================================
# 1. SubscriptionStore
# ============================================================

class TestSubscriptionStore:
    def test_add_and_get(self):
        s = _make_sub(subscription_id="sub-1")
        subscription_store.add(s)
        got = subscription_store.get("sub-1")
        assert got is not None and got.template_id == "application_health"

    def test_list_filters(self):
        subscription_store.add(_make_sub(subscription_id="sub-a", template_id="application_health"))
        subscription_store.add(_make_sub(
            subscription_id="sub-b", template_id="cluster_overview",
            scope={"cluster_id": "vm-cluster"}, modules=["cluster_health"],
        ))
        all_ = subscription_store.list()
        assert len(all_) == 2
        filtered = subscription_store.list(template_id="cluster_overview")
        assert len(filtered) == 1 and filtered[0].subscription_id == "sub-b"

    def test_update_fields(self):
        subscription_store.add(_make_sub(subscription_id="sub-u"))
        subscription_store.update("sub-u", enabled=False, last_status="ok")
        s = subscription_store.get("sub-u")
        assert s.enabled is False
        assert s.last_status == "ok"

    def test_delete(self):
        subscription_store.add(_make_sub(subscription_id="sub-d"))
        assert subscription_store.delete("sub-d") is True
        assert subscription_store.get("sub-d") is None
        assert subscription_store.delete("sub-d") is False


# ============================================================
# 2. EmailSender
# ============================================================

class TestEmailSender:
    def test_inmemory_accumulates(self):
        sender = get_email_sender()
        assert isinstance(sender, InMemoryEmailSender)
        sender.send(["a@x.com"], "subj1", "body1")
        sender.send(["b@x.com", "c@x.com"], "subj2", "body2",
                    attachments=[{"filename": "r.md", "content": "x", "mimetype": "text/markdown"}])
        assert len(sender.sent) == 2
        assert sender.sent[0]["recipients"] == ["a@x.com"]
        assert sender.sent[1]["attachments"][0]["filename"] == "r.md"

    def test_factory_singleton(self):
        s1 = get_email_sender()
        s2 = get_email_sender()
        assert s1 is s2

    def test_reset_creates_new(self):
        s1 = get_email_sender()
        reset_email_sender()
        s2 = get_email_sender()
        assert s1 is not s2


# ============================================================
# 3. Scheduler
# ============================================================

class TestScheduler:
    def test_register_adds_job(self):
        sched = ReportScheduler()
        sched.start()
        try:
            sub = _make_sub(subscription_id="sub-r")
            sched.register_subscription(sub)
            assert sched.scheduler.get_job("sub-r") is not None
            sched.unregister("sub-r")
            assert sched.scheduler.get_job("sub-r") is None
        finally:
            sched.stop()

    def test_invalid_cron_raises(self):
        sched = ReportScheduler()
        try:
            sub = _make_sub(subscription_id="sub-bad", cron="bad cron string")
            with pytest.raises(ValueError):
                sched.register_subscription(sub)
        finally:
            sched.stop()

    def test_trigger_now_generates_and_sends(self):
        # 全局 scheduler(避免重复 start/stop)
        sub = _make_sub(subscription_id="sub-tg")
        subscription_store.add(sub)

        result = _run_subscription_safely("sub-tg")
        assert result.last_status == "ok", result.last_error
        assert result.last_report_id != ""

        # 邮件已发到 InMemory
        sender = get_email_sender()
        assert isinstance(sender, InMemoryEmailSender)
        assert len(sender.sent) == 1
        assert sender.sent[0]["recipients"] == ["sre@example.com"]

    def test_trigger_disabled_skipped(self):
        sub = _make_sub(subscription_id="sub-off", enabled=False)
        subscription_store.add(sub)
        _run_subscription_safely("sub-off")
        # 未触发邮件
        sender = get_email_sender()
        assert isinstance(sender, InMemoryEmailSender)
        assert len(sender.sent) == 0

    def test_trigger_unknown_sub(self):
        result = _run_subscription_safely("sub-nope")
        assert result.last_status == "failed"


# ============================================================
# 4. 订阅端点
# ============================================================

class TestSubscriptionEndpoints:
    def test_post_creates_sub(self, client):
        tc, _ = client
        r = tc.post("/api/v1/reports/subscriptions", json={
            "template_id": "application_health",
            "scope": {"application_id": "app:vm-cluster:order"},
            "cron": "0 9 * * 1",
            "recipients": ["sre@example.com"],
        })
        assert r.status_code == 201, r.text
        data = r.json()
        assert data["template_id"] == "application_health"
        assert data["subscription_id"].startswith("sub-")
        # store 已写入
        assert subscription_store.get(data["subscription_id"]) is not None

    def test_post_invalid_cron(self, client):
        tc, _ = client
        r = tc.post("/api/v1/reports/subscriptions", json={
            "template_id": "application_health",
            "scope": {"application_id": "app:vm-cluster:order"},
            "cron": "not a cron",
            "recipients": ["a@x.com"],
        })
        assert r.status_code == 400
        assert "cron" in r.json()["detail"]

    def test_post_requires_recipients(self, client):
        tc, _ = client
        r = tc.post("/api/v1/reports/subscriptions", json={
            "template_id": "application_health",
            "scope": {"application_id": "app:vm-cluster:order"},
            "cron": "0 9 * * 1",
            "recipients": [],
        })
        assert r.status_code == 400

    def test_get_list(self, client):
        tc, _ = client
        for i in range(2):
            tc.post("/api/v1/reports/subscriptions", json={
                "template_id": "application_health",
                "scope": {"application_id": f"app:test-{i}"},
                "cron": "0 9 * * 1",
                "recipients": ["a@x.com"],
            })
        r = tc.get("/api/v1/reports/subscriptions")
        assert r.status_code == 200
        assert r.json()["total"] == 2

    def test_patch_disable(self, client):
        tc, _ = client
        r = tc.post("/api/v1/reports/subscriptions", json={
            "template_id": "application_health",
            "scope": {"application_id": "app:vm-cluster:order"},
            "cron": "0 9 * * 1",
            "recipients": ["a@x.com"],
        })
        sid = r.json()["subscription_id"]
        r2 = tc.patch(f"/api/v1/reports/subscriptions/{sid}", json={"enabled": False})
        assert r2.status_code == 200
        assert r2.json()["enabled"] is False

    def test_delete(self, client):
        tc, _ = client
        r = tc.post("/api/v1/reports/subscriptions", json={
            "template_id": "application_health",
            "scope": {"application_id": "app:vm-cluster:order"},
            "cron": "0 9 * * 1",
            "recipients": ["a@x.com"],
        })
        sid = r.json()["subscription_id"]
        r2 = tc.delete(f"/api/v1/reports/subscriptions/{sid}")
        assert r2.status_code == 204
        assert subscription_store.get(sid) is None

    def test_trigger_now(self, client):
        tc, _ = client
        r = tc.post("/api/v1/reports/subscriptions", json={
            "template_id": "application_health",
            "scope": {"application_id": "app:vm-cluster:order"},
            "cron": "0 9 * * 1",
            "recipients": ["a@x.com"],
            "modules": ["health_score"],
        })
        sid = r.json()["subscription_id"]
        r2 = tc.post(f"/api/v1/reports/subscriptions/{sid}/trigger")
        assert r2.status_code == 200
        assert r2.json()["last_status"] == "ok"

    def test_sent_emails_endpoint(self, client):
        tc, _ = client
        # 先发一封
        r = tc.post("/api/v1/reports/subscriptions", json={
            "template_id": "application_health",
            "scope": {"application_id": "app:vm-cluster:order"},
            "cron": "0 9 * * 1",
            "recipients": ["debug@x.com"],
            "modules": ["health_score"],
        })
        sid = r.json()["subscription_id"]
        tc.post(f"/api/v1/reports/subscriptions/{sid}/trigger")
        # 查
        r2 = tc.get("/api/v1/reports/sent-emails")
        assert r2.status_code == 200
        assert r2.json()["total"] >= 1
        assert any("debug@x.com" in e["recipients"] for e in r2.json()["sent"])


# ============================================================
# 5. Persistence(Neo4j dual-write)
# ============================================================

class TestSubscriptionPersistence:
    def test_persist_calls_neo4j_session_run(self):
        from app.reports.persistence import _persist_subscription
        sub = _make_sub(subscription_id="sub-pers")

        mock_driver = MagicMock()
        mock_session = MagicMock()
        mock_driver.session.return_value.__enter__.return_value = mock_session

        with patch("app.reports.persistence.n4j.get_driver", return_value=mock_driver):
            _persist_subscription(sub)

        # session.run 至少被调用一次(MERGE 节点)
        assert mock_session.run.call_count == 1

    def test_load_subscriptions_hydrates(self):
        from app.reports.persistence import load_subscriptions_from_neo4j

        # 模拟 Neo4j 返回 2 条订阅
        mock_rows = [
            {
                "sid": "sub-loaded-1", "tid": "application_health",
                "scope": '{"application_id": "app:loaded"}',
                "modules": ["health_score"],
                "cron": "0 9 * * 1", "recipients": ["a@x.com"],
                "enabled": True, "created": "2026-06-20T00:00:00Z",
                "last_run": "", "last_status": "never", "last_error": "",
                "last_report_id": "",
            },
            {
                "sid": "sub-loaded-2", "tid": "cluster_overview",
                "scope": '{"cluster_id": "vm-cluster"}',
                "modules": ["cluster_health"],
                "cron": "0 9 1 * *", "recipients": ["b@x.com"],
                "enabled": False, "created": "2026-06-20T00:00:00Z",
                "last_run": "", "last_status": "never", "last_error": "",
                "last_report_id": "",
            },
        ]
        with patch("app.reports.persistence.n4j.run_query", return_value=mock_rows):
            count = load_subscriptions_from_neo4j()

        assert count == 2
        assert subscription_store.get("sub-loaded-1") is not None
        assert subscription_store.get("sub-loaded-2").enabled is False

    def test_load_subscriptions_handles_empty(self):
        from app.reports.persistence import load_subscriptions_from_neo4j
        with patch("app.reports.persistence.n4j.run_query", return_value=[]):
            count = load_subscriptions_from_neo4j()
        assert count == 0

    def test_delete_calls_session_run(self):
        from app.reports.persistence import _delete_subscription_node
        mock_driver = MagicMock()
        mock_session = MagicMock()
        mock_driver.session.return_value.__enter__.return_value = mock_session

        with patch("app.reports.persistence.n4j.get_driver", return_value=mock_driver):
            _delete_subscription_node("sub-del")

        assert mock_session.run.call_count == 1
