"""PRD-001 Phase 2 余项 — Recovery Chain 编排器测试。

覆盖:
1. chain template 列表 / 获取
2. execute_chain 全 low_risk happy path → succeeded(顺序跑完所有 step)
3. on_failure=stop → 第 N 步失败 → status=partial,停在 N
4. on_failure=rollback_all → 第 N 步失败 → 反向回滚 1..N-1,status=rolled_back
5. on_failure=continue → 第 N 步失败 → 继续 N+1,最终 partial(因为有失败)
6. 链级审批:任一步 medium/high → 整链 awaiting_approval(只一次审批)
7. approve → continue_chain_after_approval → 跑完;reject → chain.status=failed
8. abort 端点:执行中可中止
9. step execution.chain_id / chain_step_index 反向关联正确
10. 链跑完返回的 RecoveryChain 序列化包含 step 详情
"""
from __future__ import annotations

import pytest

from app.config import settings
from app.datasource.models import DataNode
from app.datasource.store import store
from app.recovery import chains as chains_mod
from app.recovery import verifiers as ver_mod


@pytest.fixture(autouse=True)
def _seed(monkeypatch):
    monkeypatch.setattr(settings, "recovery_handler_mode", "mock")
    store.nodes.clear()
    store.edges.clear()
    store.executions.clear()
    store.approvals.clear()
    store.chains.clear()
    nodes = [
        DataNode("deploy:vm-cluster:otel-demo:cart", "Deployment", "cart",
                 {"namespace": "otel-demo", "name": "cart", "cluster_id": "vm-cluster",
                  "desired_replicas": 3, "available_replicas": 3, "current_revision": 2,
                  "owner_team": "platform"}),
        DataNode("secret:vm-cluster:otel-demo:cart-tls", "Secret", "cart-tls",
                 {"namespace": "otel-demo", "name": "cart-tls", "cluster_id": "vm-cluster",
                  "secret_version": 1, "owner_team": "security"}),
        DataNode("node:vm-cluster:worker-1", "KubernetesNode", "worker-1",
                 {"name": "worker-1", "cluster_id": "vm-cluster", "cordoned": False}),
    ]
    for n in nodes:
        store.upsert_node(n)
    yield
    store.nodes.clear()
    store.edges.clear()
    store.executions.clear()
    store.approvals.clear()
    store.chains.clear()


# ============================================================
# Template 元数据
# ============================================================

class TestChainTemplates:
    def test_list_templates(self):
        from app.recovery.action_defs import list_chain_templates
        tpls = list_chain_templates()
        ids = [t["template_id"] for t in tpls]
        assert "safe_rollback_deployment" in ids
        assert "graceful_refresh_secret" in ids
        assert "drain_node_safely" in ids

    def test_get_unknown_template(self):
        from app.recovery.action_defs import get_chain_template
        assert get_chain_template("nonexistent") is None


# ============================================================
# happy path / 失败策略
# ============================================================

class TestExecuteChainHappyPath:
    def test_safe_rollback_deployment_succeeds(self, monkeypatch):
        """safe_rollback_deployment: scale+2 → rollback → scale-2,全步 verify passed → succeeded。"""
        # 链 3 步全要审批(scale_deployment low / rollback_deployment high / scale_deployment low)
        # 我们临时把 rollback 也降到 low,跳过审批走 happy path
        from app.recovery.action_defs import ACTION_DEFS
        monkeypatch.setitem(ACTION_DEFS["rollback_deployment"], "risk_level", "low")
        monkeypatch.setitem(ACTION_DEFS["rollback_deployment"], "requires_approval", False)

        chain = chains_mod.execute_chain(
            template_id="safe_rollback_deployment",
            target_resource_id="deploy:vm-cluster:otel-demo:cart",
            initiated_by="alice",
        )
        assert chain.status == "succeeded"
        assert chain.total_steps == 3
        assert chain.current_step_index == 3
        assert len(chain.step_executions) == 3
        # 每个 step execution 都有 chain_id + chain_step_index
        for idx, eid in enumerate(chain.step_executions):
            ex = store.get_execution(eid)
            assert ex.chain_id == chain.chain_id
            assert ex.chain_step_index == idx
            assert ex.status == "succeeded"

    def test_unknown_template_raises(self):
        from app.recovery.execution import ExecutionError
        with pytest.raises(ExecutionError, match="unknown chain template"):
            chains_mod.execute_chain(
                template_id="nonexistent",
                target_resource_id="deploy:vm-cluster:otel-demo:cart",
                initiated_by="alice",
            )

    def test_target_type_mismatch_raises(self, monkeypatch):
        """链 template 第一步是 scale_deployment 但 target 是 Secret → 报错。"""
        from app.recovery.execution import ExecutionError
        from app.recovery.action_defs import ACTION_DEFS
        monkeypatch.setitem(ACTION_DEFS["rollback_deployment"], "risk_level", "low")
        monkeypatch.setitem(ACTION_DEFS["rollback_deployment"], "requires_approval", False)
        with pytest.raises(ExecutionError, match="mismatches"):
            chains_mod.execute_chain(
                template_id="safe_rollback_deployment",
                target_resource_id="secret:vm-cluster:otel-demo:cart-tls",
                initiated_by="alice",
            )


# ============================================================
# on_failure 策略
# ============================================================

class TestOnFailureStrategies:
    def _fake_verifier_failing_at_step(self, fail_action_id):
        def fake(target_id, params, exec_result, context):
            return {"passed": False, "predicate": fail_action_id, "message": "fake fail"}
        return fake

    def test_stop_strategy(self, monkeypatch):
        """on_failure=stop:第 2 步 rollback_deployment verify_failed → partial,停在 1。"""
        from app.recovery.action_defs import ACTION_DEFS
        monkeypatch.setitem(ACTION_DEFS["rollback_deployment"], "risk_level", "low")
        monkeypatch.setitem(ACTION_DEFS["rollback_deployment"], "requires_approval", False)
        # 让 rollback_deployment 的 verifier 失败
        monkeypatch.setitem(
            ver_mod.VERIFIERS, "rollback_deployment",
            self._fake_verifier_failing_at_step("rollback_deployment"),
        )
        chain = chains_mod.execute_chain(
            template_id="safe_rollback_deployment",
            target_resource_id="deploy:vm-cluster:otel-demo:cart",
            initiated_by="alice",
            on_failure_override="stop",
        )
        # 第 1 步成功,第 2 步 verify_failed → stop
        assert chain.status == "partial"
        assert chain.current_step_index == 2  # 推进到 idx=2,停在那
        assert len(chain.step_executions) == 2

    def test_rollback_all_strategy(self, monkeypatch):
        """on_failure=rollback_all:第 2 步失败 → 反向回滚第 1 步。"""
        from app.recovery.action_defs import ACTION_DEFS
        monkeypatch.setitem(ACTION_DEFS["rollback_deployment"], "risk_level", "low")
        monkeypatch.setitem(ACTION_DEFS["rollback_deployment"], "requires_approval", False)
        monkeypatch.setitem(
            ver_mod.VERIFIERS, "rollback_deployment",
            self._fake_verifier_failing_at_step("rollback_deployment"),
        )
        # 用 safe_rollback_deployment 的默认 rollback_all
        chain = chains_mod.execute_chain(
            template_id="safe_rollback_deployment",
            target_resource_id="deploy:vm-cluster:otel-demo:cart",
            initiated_by="alice",
        )
        assert chain.status == "rolled_back"
        assert chain.on_failure == "rollback_all"
        # 第 1 步 execution 应被标 rolled_back
        first_ex = store.get_execution(chain.step_executions[0])
        assert first_ex.status == "rolled_back"
        assert first_ex.rollback_execution_id is not None

    def test_continue_strategy(self, monkeypatch):
        """on_failure=continue:第 2 步失败 → 继续第 3 步,最终 partial。"""
        from app.recovery.action_defs import ACTION_DEFS
        monkeypatch.setitem(ACTION_DEFS["rollback_deployment"], "risk_level", "low")
        monkeypatch.setitem(ACTION_DEFS["rollback_deployment"], "requires_approval", False)
        monkeypatch.setitem(
            ver_mod.VERIFIERS, "rollback_deployment",
            self._fake_verifier_failing_at_step("rollback_deployment"),
        )
        chain = chains_mod.execute_chain(
            template_id="safe_rollback_deployment",
            target_resource_id="deploy:vm-cluster:otel-demo:cart",
            initiated_by="alice",
            on_failure_override="continue",
        )
        # 跑完全部 3 步,但有失败 → partial
        assert chain.status == "partial"
        assert len(chain.step_executions) == 3


# ============================================================
# 链级审批
# ============================================================

class TestChainApproval:
    def test_chain_with_high_risk_step_awaits_approval(self):
        """safe_rollback_deployment 包含 rollback_deployment (high_risk) → 整链 awaiting_approval。"""
        chain = chains_mod.execute_chain(
            template_id="safe_rollback_deployment",
            target_resource_id="deploy:vm-cluster:otel-demo:cart",
            initiated_by="alice",
        )
        assert chain.status == "awaiting_approval"
        assert chain.approval_id != ""
        # 此时还没跑任何 step
        assert len(chain.step_executions) == 0
        # ApprovalRequest 在 store 里
        ap = store.get_approval(chain.approval_id)
        assert ap is not None
        assert ap.execution_id == chain.chain_id  # 链级审批占用 execution_id 字段

    def test_approve_chain_runs_all_steps(self, monkeypatch):
        """approve 链审批 → continue_chain_after_approval → 跑完。"""
        from app.recovery.approval import approve
        chain = chains_mod.execute_chain(
            template_id="safe_rollback_deployment",
            target_resource_id="deploy:vm-cluster:otel-demo:cart",
            initiated_by="alice",
        )
        approval, _ = approve(chain.approval_id, "bob", comment="approved")
        # 重新读 chain
        chain = store.get_chain(chain.chain_id)
        assert chain.status == "succeeded"
        assert len(chain.step_executions) == 3

    def test_reject_chain_marks_failed(self):
        from app.recovery.approval import reject
        chain = chains_mod.execute_chain(
            template_id="safe_rollback_deployment",
            target_resource_id="deploy:vm-cluster:otel-demo:cart",
            initiated_by="alice",
        )
        reject(chain.approval_id, "bob", comment="too risky")
        chain = store.get_chain(chain.chain_id)
        assert chain.status == "failed"
        assert "too risky" in chain.failure_reason
        # 无 step 执行
        assert len(chain.step_executions) == 0


# ============================================================
# abort
# ============================================================

class TestChainAbort:
    def test_abort_awaiting_approval(self):
        chain = chains_mod.execute_chain(
            template_id="safe_rollback_deployment",
            target_resource_id="deploy:vm-cluster:otel-demo:cart",
            initiated_by="alice",
        )
        assert chain.status == "awaiting_approval"
        chain = chains_mod.abort_chain(chain.chain_id, reason="testing")
        assert chain.status == "aborted"
        assert chain.failure_reason == "testing"

    def test_abort_succeeded_chain_rejected(self, monkeypatch):
        from app.recovery.action_defs import ACTION_DEFS
        from app.recovery.execution import ExecutionError
        monkeypatch.setitem(ACTION_DEFS["rollback_deployment"], "risk_level", "low")
        monkeypatch.setitem(ACTION_DEFS["rollback_deployment"], "requires_approval", False)
        chain = chains_mod.execute_chain(
            template_id="safe_rollback_deployment",
            target_resource_id="deploy:vm-cluster:otel-demo:cart",
            initiated_by="alice",
        )
        assert chain.status == "succeeded"
        with pytest.raises(ExecutionError, match="cannot abort"):
            chains_mod.abort_chain(chain.chain_id)


# ============================================================
# 端点 + 序列化
# ============================================================

class TestChainEndpoints:
    def test_serialize_chain_includes_steps(self, monkeypatch):
        from app.routers.recovery import _serialize_chain
        from app.recovery.action_defs import ACTION_DEFS
        monkeypatch.setitem(ACTION_DEFS["rollback_deployment"], "risk_level", "low")
        monkeypatch.setitem(ACTION_DEFS["rollback_deployment"], "requires_approval", False)
        chain = chains_mod.execute_chain(
            template_id="safe_rollback_deployment",
            target_resource_id="deploy:vm-cluster:otel-demo:cart",
            initiated_by="alice",
        )
        d = _serialize_chain(chain, expand=True)
        assert d["status"] == "succeeded"
        assert len(d["steps"]) == 3
        # 每个 step 应包含 action_id / chain_step_index
        assert d["steps"][0]["action_id"] == "scale_deployment"
        assert d["steps"][1]["action_id"] == "rollback_deployment"
        assert d["steps"][2]["action_id"] == "scale_deployment"
        assert d["steps"][0]["chain_step_index"] == 0
