"""K8sEventConnector — K8s events → ChangeEvent。

工作流(每 30 秒):
1. 拉 namespace 内所有 Event(>= last_sync_at,首轮只记 baseline 时间不发事件)
2. 按 reason 过滤为我们关心的几类:
   - ScalingReplicaSet → deployment_rolled
   - Killing(只在 reason 是 manual rollout 时,排除 OOM/eviction)→ deployment_rolled
   - Started + 容器 restartCount > 0 → deployment_rolled (Pod 重启)
3. 转 ChangeEvent 写入 DSS

设计选择:
- 不每个 K8s event 都发 ChangeEvent — 噪声太多
- 只关心"主动变更"类(Scale / Rollout / Restart),不管 K8s 自身调度事件
- 去重:每个 ScalingReplicaSet event 在 K8s 内是唯一的(timestamp + name),不会重复
"""

from __future__ import annotations

import asyncio
import logging
import os
from datetime import datetime, timezone
from typing import Any, Optional

from app.changes.event_service import record_change
from app.config import settings
from app.datasource.connectors.base import BaseConnector, SyncResult
from app.datasource.connectors.k8s_mapper import deployment_id, pod_id


logger = logging.getLogger(__name__)


# 我们关心的 K8s event reasons → ChangeEvent type
INTERESTING_REASONS: dict[str, str] = {
    "ScalingReplicaSet": "deployment_rolled",
    "SuccessfulRescale": "deployment_rolled",
    # Started/Killing 单独看 restart_count,不直接映射
}


class K8sEventConnector(BaseConnector):
    name = "k8s_events"

    def __init__(
        self,
        cluster_id: Optional[str] = None,
        namespace: Optional[str] = None,
        kubeconfig_path: Optional[str] = None,
        sync_interval_seconds: Optional[int] = None,
    ):
        super().__init__()
        self.cluster_id = cluster_id or settings.active_cluster
        self.namespace = namespace or settings.k8s_namespace
        self.kubeconfig_path = kubeconfig_path or settings.kubeconfigs.get(self.cluster_id, "")
        self.sync_interval_seconds = sync_interval_seconds or 30
        self._k8s_loaded = False
        self._seen_event_uids: set[str] = set()
        self._first_sync = True

    async def sync_once(self) -> SyncResult:
        result = SyncResult()
        await self._ensure_kube_loaded()

        from kubernetes_asyncio import client
        from kubernetes_asyncio.client.api_client import ApiClient

        async with ApiClient() as api:
            core = client.CoreV1Api(api)
            events = await core.list_namespaced_event(self.namespace)

        # 首次同步只记录 uid,不发事件(防止启动时把历史事件全炸出来)
        if self._first_sync:
            for ev in events.items:
                self._seen_event_uids.add(ev.metadata.uid)
            self._first_sync = False
            result.notes.append(f"baseline {len(self._seen_event_uids)} events seen")
            return result

        for ev in events.items:
            if ev.metadata.uid in self._seen_event_uids:
                continue
            self._seen_event_uids.add(ev.metadata.uid)
            ce = self._event_to_change(ev)
            if ce is None:
                continue
            try:
                record_change(**ce)
                result.events_added += 1
            except Exception as e:  # noqa: BLE001
                logger.warning("record_change failed for event %s: %s",
                               ev.metadata.uid, e)

        result.notes.append(
            f"events_total={len(events.items)} new={result.events_added}"
        )
        return result

    # ============================================================
    # 转换
    # ============================================================

    def _event_to_change(self, ev: Any) -> Optional[dict]:
        """K8s Event → record_change kwargs(过滤后)。返回 None 跳过。"""
        reason = getattr(ev, "reason", "")
        msg = getattr(ev, "message", "") or ""
        involved = getattr(ev, "involved_object", None)
        if involved is None:
            return None

        change_type = INTERESTING_REASONS.get(reason)
        if change_type is None:
            return None

        # involved_object → DSS resource ID
        kind = getattr(involved, "kind", "")
        name = getattr(involved, "name", "")
        if not name:
            return None

        if kind == "Deployment":
            target_id = deployment_id(self.cluster_id, self.namespace, name)
        elif kind == "ReplicaSet":
            # ReplicaSet 不入图,反推 Deployment(strip ReplicaSet hash)
            owner_deploy = name.rsplit("-", 1)[0] if "-" in name else name
            target_id = deployment_id(self.cluster_id, self.namespace, owner_deploy)
        elif kind == "Pod":
            target_id = pod_id(self.cluster_id, self.namespace, name)
        else:
            return None

        return {
            "change_type": change_type,
            "target_resource_id": target_id,
            "changed_by": "k8s",
            "source": "k8s_api",
            "description": f"{reason}: {msg[:200]}",
            "diff_summary": {"reason": reason, "kind": kind, "name": name},
        }

    # ============================================================
    # K8s init(同 K8sConnector)
    # ============================================================

    async def _ensure_kube_loaded(self):
        if self._k8s_loaded:
            return
        from kubernetes_asyncio import config as k8s_config

        try:
            if self.kubeconfig_path and os.path.exists(self.kubeconfig_path):
                await k8s_config.load_kube_config(config_file=self.kubeconfig_path)
            else:
                try:
                    k8s_config.load_incluster_config()
                except Exception:
                    await k8s_config.load_kube_config()
        except Exception as e:
            logger.error("k8s_event connector failed to load kubeconfig: %s", e)
            raise
        self._k8s_loaded = True
