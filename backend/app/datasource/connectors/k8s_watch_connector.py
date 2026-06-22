"""K8sWatchConnector — 真 `watch.Watch().stream()` 实时监听资源变更 → ChangeEvent。

PRD-002 Phase 2。区别于 K8sEventConnector(30s 轮询 list_namespaced_event),
本 connector 用 K8s watch API 长连接,实时捕获 ConfigMap / Secret / Deployment 的
spec 变化,产出含 `yaml_diff` 的 ChangeEvent。

工作流:
- 三类资源(ConfigMap / Secret / Deployment)各自一个 watch task,`asyncio.gather` 并发
- 内存持 `{kind: {name: last_spec}}` 快照 + `{kind: last_resource_version}`
- MODIFIED → `compute_yaml_diff(old, new)` → record_change(configmap_updated / secret_rotated / deployment_rolled)
- ADDED:首轮只建快照不发事件(防启动炸历史);快照已有时视为新增
- DELETED:从快照移除,不发事件
- 断线重连:watch 抛异常或连接断 → sleep 5s 带 `resource_version=<last>` 续传;
  resource_version 过期(Gone)→ 全量 list 重建快照 + 重置 rv 再 watch

设计选择:
- **重写 `start()` / `_run_loop`** —— watch 是长连接阻塞,不能按 BaseConnector 的
  sync_interval 轮询模式跑。`sync_once()` 保留作 /sync-now 兜底(走一次 list)
- K8s auth 复用 `k8s_client.ensure_kube_loaded`(单集群,settings.active_cluster)
- gate `settings.k8s_watch_enabled`(默认 0)—— 测试与现有 e2e 不受影响,vm 集群才开
- 三类资源 kind → change_type 映射固定;target DSS id 用 k8s_mapper 工厂
"""
from __future__ import annotations

import asyncio
import logging
from typing import Any, Optional

from app.config import settings
from app.datasource.connectors.base import BaseConnector, SyncResult
from app.datasource.connectors.k8s_mapper import (
    configmap_id,
    deployment_id,
    secret_id,
)


logger = logging.getLogger(__name__)


# kind → (change_type, id_factory)
_KIND_MAP: dict[str, tuple[str, Any]] = {
    "ConfigMap": ("configmap_updated", configmap_id),
    "Secret": ("secret_rotated", secret_id),
    "Deployment": ("deployment_rolled", deployment_id),
}

# watch 断线后的重连间隔
_RECONNECT_DELAY = 5


class K8sWatchConnector(BaseConnector):
    name = "k8s_watch"
    sync_interval_seconds = 30  # 仅 sync_once(list 兜底)用,watch 走自己的 loop

    def __init__(
        self,
        cluster_id: Optional[str] = None,
        namespace: Optional[str] = None,
    ):
        super().__init__()
        self.cluster_id = cluster_id or settings.active_cluster
        self.namespace = namespace or settings.k8s_namespace
        # {kind: {name: last_spec_dict}}
        self._snapshots: dict[str, dict[str, dict]] = {k: {} for k in _KIND_MAP}
        # {kind: last_resource_version}
        self._resource_versions: dict[str, str] = {k: "" for k in _KIND_MAP}
        self._first_sync: dict[str, bool] = {k: True for k in _KIND_MAP}

    # ============================================================
    # 重写生命周期 —— watch 是长连接,不走轮询 loop
    # ============================================================

    async def start(self):
        if not settings.k8s_watch_enabled:
            logger.info("k8s_watch connector disabled (K8S_WATCH_ENABLED=0)")
            return
        if self._task is not None and not self._task.done():
            logger.warning("connector %s already running", self.name)
            return
        self._stop_event.clear()
        self._task = asyncio.create_task(self._run_watch(), name=f"connector-{self.name}")
        logger.info(
            "connector %s started (watch mode, cluster=%s, namespace=%s)",
            self.name, self.cluster_id, self.namespace,
        )

    async def _run_watch(self):
        """并发跑三类资源的 watch,任一断线各自重连。"""
        from app.datasource.connectors.k8s_client import ensure_kube_loaded

        try:
            await ensure_kube_loaded(self.cluster_id)
        except Exception as e:  # noqa: BLE001
            logger.error("k8s_watch failed to load kubeconfig: %s", e)
            self._last_error_message = f"kubeconfig load failed: {e}"
            return

        tasks = [asyncio.create_task(self._watch_kind(kind)) for kind in _KIND_MAP]
        try:
            await asyncio.gather(*tasks, return_exceptions=True)
        except asyncio.CancelledError:
            pass
        finally:
            for t in tasks:
                if not t.done():
                    t.cancel()

    async def _watch_kind(self, kind: str):
        """单个 kind 的 watch 循环,断线重连。

        标准 list-then-watch 模式:首次(无 resource_version)先 list 一次建快照 +
        拿 resource_version + 翻 first_sync=False,再起 watch。后续 watch 只收
        真实变更事件,避免启动时 ADDED 炸历史。
        """
        from kubernetes_asyncio import client
        from kubernetes_asyncio.client.api_client import ApiClient
        from kubernetes_asyncio.watch import Watch

        list_fn_name = {
            "ConfigMap": "list_namespaced_config_map",
            "Secret": "list_namespaced_secret",
            "Deployment": "list_namespaced_deployment",
        }[kind]

        while not self._stop_event.is_set():
            try:
                # 首次或 rv 失效 → list bootstrap
                if not self._resource_versions[kind]:
                    await self._bootstrap_list(kind)

                async with ApiClient() as api:
                    core_or_apps = client.CoreV1Api(api) if kind in ("ConfigMap", "Secret") else client.AppsV1Api(api)
                    list_fn = getattr(core_or_apps, list_fn_name)
                    watch = Watch()
                    kwargs = {"namespace": self.namespace}
                    if self._resource_versions[kind]:
                        kwargs["resource_version"] = self._resource_versions[kind]
                    async for event in watch.stream(list_fn, **kwargs):
                        if self._stop_event.is_set():
                            break
                        self._handle_watch_event(kind, event)
            except asyncio.CancelledError:
                raise
            except Exception as e:  # noqa: BLE001 — watch 断线/异常一律重连
                logger.warning(
                    "k8s_watch %s stream error: %s: %s — reconnecting in %ds",
                    kind, type(e).__name__, e, _RECONNECT_DELAY,
                )
                self._last_error_message = f"{kind} watch: {type(e).__name__}: {e}"
                # resource_version 过期(Gone)→ 重建快照
                if self._is_gone(e):
                    logger.info("k8s_watch %s resource_version gone — rebuilding snapshot", kind)
                    self._snapshots[kind] = {}
                    self._resource_versions[kind] = ""
                    self._first_sync[kind] = True
                try:
                    await asyncio.wait_for(
                        self._stop_event.wait(), timeout=_RECONNECT_DELAY
                    )
                    return  # stop 了
                except asyncio.TimeoutError:
                    pass  # 重连

    async def _bootstrap_list(self, kind: str):
        """list 一次建快照 + 拿 resource_version + 翻 first_sync=False。"""
        from kubernetes_asyncio import client
        from kubernetes_asyncio.client.api_client import ApiClient

        list_fn_name = {
            "ConfigMap": "list_namespaced_config_map",
            "Secret": "list_namespaced_secret",
            "Deployment": "list_namespaced_deployment",
        }[kind]

        async with ApiClient() as api:
            core_or_apps = client.CoreV1Api(api) if kind in ("ConfigMap", "Secret") else client.AppsV1Api(api)
            list_fn = getattr(core_or_apps, list_fn_name)
            resp = await list_fn(self.namespace)
            for item in (resp.items or []):
                d = self._to_dict(item)
                name = d.get("metadata", {}).get("name", "")
                if name:
                    self._snapshots[kind][name] = d
            rv = getattr(resp.metadata, "resource_version", "") if resp.metadata else ""
            if rv:
                self._resource_versions[kind] = rv
        self._first_sync[kind] = False
        logger.info("k8s_watch %s bootstrapped: %d items, rv=%s",
                    kind, len(self._snapshots[kind]), self._resource_versions[kind])

    @staticmethod
    def _is_gone(e: Exception) -> bool:
        msg = str(e).lower()
        return "gone" in msg or "expired" in msg or "410" in msg

    # ============================================================
    # 事件处理 —— 纯函数式,可单测
    # ============================================================

    def _handle_watch_event(self, kind: str, event: dict) -> Optional[dict]:
        """处理一个 watch 事件 dict {type, object}。

        返回 record_change 的 kwargs(测试断言用),None 表示跳过。
        side-effect: 更新快照 + resource_version + 写 ChangeEvent。
        """
        from app.changes.event_service import record_change
        from app.changes.yaml_diff import compute_yaml_diff, summarize_diff

        etype = event.get("type", "")
        obj = event.get("object")
        if obj is None:
            return None

        # kubernetes_asyncio 的 object 可能是 model 对象或 dict;统一转 dict
        obj_dict = self._to_dict(obj)
        name = obj_dict.get("metadata", {}).get("name", "")
        rv = obj_dict.get("metadata", {}).get("resourceVersion", "")
        if not name:
            return None
        if rv:
            self._resource_versions[kind] = rv

        change_type, id_factory = _KIND_MAP[kind]
        target_id = id_factory(self.cluster_id, self.namespace, name)

        if etype == "ADDED":
            old = self._snapshots[kind].get(name)
            self._snapshots[kind][name] = obj_dict
            if self._first_sync[kind]:
                return None  # 首轮只建快照
            if old is None:
                # 真新增(快照里没有)→ 记一个 add 事件
                diff = compute_yaml_diff({}, obj_dict, name=name)
                return self._emit(change_type, target_id, obj_dict, diff,
                                  source="k8s_api", op="added", record_fn=record_change)
            # 快照里有 → 当 MODIFIED 处理
            return self._on_modified(kind, name, old, obj_dict, change_type, target_id,
                                     record_fn=record_change, yaml_fn=compute_yaml_diff,
                                     summary_fn=summarize_diff)

        if etype == "MODIFIED":
            old = self._snapshots[kind].get(name, {})
            self._snapshots[kind][name] = obj_dict
            if self._first_sync[kind]:
                return None
            return self._on_modified(kind, name, old, obj_dict, change_type, target_id,
                                     record_fn=record_change, yaml_fn=compute_yaml_diff,
                                     summary_fn=summarize_diff)

        if etype == "DELETED":
            self._snapshots[kind].pop(name, None)
            return None

        return None

    def _on_modified(self, kind, name, old, new, change_type, target_id,
                     record_fn, yaml_fn, summary_fn) -> dict:
        diff = yaml_fn(old, new, name=name)
        if not diff:
            return None  # 噪声过滤后无业务差异
        summary = summary_fn(diff)
        return self._emit(change_type, target_id, new, diff,
                          source="k8s_api", op="modified", record_fn=record_fn,
                          diff_summary=summary)

    def _emit(self, change_type, target_id, obj_dict, diff, source, op,
              record_fn, diff_summary=None) -> dict:
        kwargs = {
            "change_type": change_type,
            "target_resource_id": target_id,
            "source": source,
            "description": f"k8s watch {op}: {target_id}",
            "diff_summary": diff_summary or {},
            "cluster_id": self.cluster_id,
            "yaml_diff": diff,
        }
        try:
            record_fn(**kwargs)
            if self._last_result is None:
                self._last_result = SyncResult()
            self._last_result.events_added += 1
        except Exception as e:  # noqa: BLE001
            logger.warning("k8s_watch record_change failed for %s: %s", target_id, e)
        return kwargs

    @staticmethod
    def _to_dict(obj: Any) -> dict:
        """kubernetes_asyncio model → dict(model 有 to_dict);dict 原样返回。"""
        if isinstance(obj, dict):
            return obj
        to_dict = getattr(obj, "to_dict", None)
        if to_dict is not None:
            return to_dict()
        return {}

    def _mark_first_sync_done(self):
        """首轮 ADDED 处理完后调 —— 后续 ADDED/MODIFIED 才发事件。"""
        for k in _KIND_MAP:
            self._first_sync[k] = False

    # ============================================================
    # sync_once —— /sync-now 兜底,走一次 list(不走 watch)
    # ============================================================

    async def sync_once(self) -> SyncResult:
        result = SyncResult()
        if not settings.k8s_watch_enabled:
            result.notes.append("k8s_watch disabled")
            return result
        from app.datasource.connectors.k8s_client import ensure_kube_loaded
        from kubernetes_asyncio import client
        from kubernetes_asyncio.client.api_client import ApiClient

        try:
            await ensure_kube_loaded(self.cluster_id)
        except Exception as e:  # noqa: BLE001
            result.notes.append(f"kubeconfig load failed: {e}")
            return result

        async with ApiClient() as api:
            core = client.CoreV1Api(api)
            apps = client.AppsV1Api(api)
            list_calls = {
                "ConfigMap": core.list_namespaced_config_map,
                "Secret": core.list_namespaced_secret,
                "Deployment": apps.list_namespaced_deployment,
            }
            for kind, fn in list_calls.items():
                resp = await fn(self.namespace)
                for item in (resp.items or []):
                    d = self._to_dict(item)
                    name = d.get("metadata", {}).get("name", "")
                    if name:
                        self._snapshots[kind][name] = d
                result.notes.append(f"{kind}_listed={len(resp.items or [])}")
        self._mark_first_sync_done()
        return result

    # ============================================================
    # 状态
    # ============================================================

    def status(self) -> dict:
        base = super().status()
        base.update({
            "mode": "watch",
            "cluster_id": self.cluster_id,
            "namespace": self.namespace,
            "watched_kinds": list(_KIND_MAP.keys()),
            "snapshot_sizes": {k: len(v) for k, v in self._snapshots.items()},
        })
        return base
