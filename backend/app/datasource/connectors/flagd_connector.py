"""FlagdConnector — flagd /ResolveAll → ChangeEvent。

工作流(每 20 秒):
1. POST flagd.evaluation.v1.Service/ResolveAll → 拿全部 flag 当前状态
2. diff 上一轮 snapshot,任何 (variant 或 value 变化) → 一条 ChangeEvent
3. ChangeEvent 写到 DSS,target_resource_id 指向 OTel demo 的 flagd ConfigMap

为什么走 ChangeEvent 而不是节点更新:
- flag 翻转是"变更事件",符合 PRD-002 ChangeEvent 语义
- propagated_to 自动推导(基于 flagd ConfigMap 被哪些 Pod 挂载)
- 与 /api/v1/change-events/correlated 现有查询天然兼容

第一次 sync 不发 ChangeEvent(只记 baseline),第二次起才 diff。
"""

from __future__ import annotations

import logging
from typing import Any, Optional

import httpx

from app.changes.event_service import record_change
from app.config import settings
from app.datasource.connectors.base import BaseConnector, SyncResult
from app.datasource.connectors.k8s_mapper import configmap_id


logger = logging.getLogger(__name__)


class FlagdConnector(BaseConnector):
    name = "flagd"

    def __init__(
        self,
        flagd_url: Optional[str] = None,
        cluster_id: Optional[str] = None,
        namespace: Optional[str] = None,
        sync_interval_seconds: Optional[int] = None,
        flagd_configmap_name: str = "otel-demo-flagd-config",
        timeout_seconds: float = 5.0,
    ):
        super().__init__()
        self.flagd_url = (flagd_url or settings.flagd_url).rstrip("/")
        self.cluster_id = cluster_id or settings.active_cluster
        self.namespace = namespace or settings.k8s_namespace
        self.sync_interval_seconds = sync_interval_seconds or settings.flagd_sync_interval_seconds
        self.flagd_configmap_name = flagd_configmap_name
        self.timeout_seconds = timeout_seconds
        self._last_snapshot: Optional[dict[str, dict]] = None

    async def sync_once(self) -> SyncResult:
        result = SyncResult()
        if not self.flagd_url:
            result.notes.append("flagd_url is empty, skipping")
            return result

        try:
            current = await self._fetch_state()
        except Exception as e:  # noqa: BLE001
            result.notes.append(f"fetch state failed: {e}")
            return result

        if self._last_snapshot is None:
            self._last_snapshot = current
            result.notes.append(f"baseline {len(current)} flags, no events emitted on first sync")
            return result

        target_id = configmap_id(self.cluster_id, self.namespace, self.flagd_configmap_name)
        for flag_name, new_state in current.items():
            old_state = self._last_snapshot.get(flag_name)
            if old_state is None:
                # 新增 flag
                _try_record(
                    target_id, flag_name,
                    old=None, new=_extract_value(new_state),
                    description=f"flag added: {flag_name}",
                )
                result.events_added += 1
                continue
            if _state_differs(old_state, new_state):
                _try_record(
                    target_id, flag_name,
                    old=_extract_value(old_state),
                    new=_extract_value(new_state),
                    description=f"flag {flag_name}: variant={old_state.get('variant')} → {new_state.get('variant')}",
                )
                result.events_added += 1

        # flag 删除(罕见)
        for old_name in self._last_snapshot:
            if old_name not in current:
                _try_record(
                    target_id, old_name,
                    old=_extract_value(self._last_snapshot[old_name]),
                    new=None,
                    description=f"flag removed: {old_name}",
                )
                result.events_added += 1

        self._last_snapshot = current
        result.notes.append(f"flags={len(current)} changes={result.events_added}")
        return result

    # ============================================================
    # flagd HTTP API
    # ============================================================

    async def _fetch_state(self) -> dict[str, dict]:
        """POST /flagd.evaluation.v1.Service/ResolveAll → {flag_name: state}"""
        async with httpx.AsyncClient(timeout=self.timeout_seconds) as client:
            resp = await client.post(
                f"{self.flagd_url}/flagd.evaluation.v1.Service/ResolveAll",
                json={},
                headers={"Content-Type": "application/json"},
            )
            resp.raise_for_status()
            body = resp.json()
            return body.get("flags", {}) or {}


# ============================================================
# Helpers
# ============================================================

def _extract_value(state: dict) -> Any:
    """从 ResolveAll 状态拿真实值(boolValue / doubleValue / stringValue / objectValue)。"""
    for k in ("boolValue", "doubleValue", "stringValue", "intValue", "objectValue"):
        if k in state:
            return state[k]
    return state.get("variant", "")


def _state_differs(old: dict, new: dict) -> bool:
    """variant 或具体值变了都算变更。"""
    if old.get("variant") != new.get("variant"):
        return True
    return _extract_value(old) != _extract_value(new)


def _try_record(target_id: str, flag_name: str, old: Any, new: Any, description: str):
    """包一层 try — record_change 在 target 不在 DSS 时仍能写,但其他异常吞掉避免 connector 挂掉。"""
    try:
        record_change(
            change_type="configmap_updated",
            target_resource_id=target_id,
            changed_by="flagd",
            source="flagd",
            description=description,
            diff_summary={flag_name: {"old": old, "new": new}},
        )
    except Exception as e:  # noqa: BLE001
        logger.warning("record_change failed for flag %s: %s", flag_name, e)
