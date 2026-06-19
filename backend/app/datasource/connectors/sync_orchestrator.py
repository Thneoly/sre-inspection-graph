"""Sync orchestrator — 启动 / 停止所有 connector,暴露给 main.py 用。

Sprint 1 只接 K8sConnector,Sprint 2/3 加 Prometheus / Jaeger / flagd 时
注册一行即可。

不在本模块:
- connector 内部循环逻辑(在 BaseConnector 里)
- 控制 API 端点(在 routers/connectors.py 里)
"""

from __future__ import annotations

import logging
from typing import Optional

from app.config import settings
from app.datasource.connectors.base import BaseConnector
from app.datasource.connectors.k8s_connector import K8sConnector


logger = logging.getLogger(__name__)


class ConnectorRegistry:
    """所有 connector 的中心注册表 — 单例。"""

    def __init__(self):
        self._connectors: dict[str, BaseConnector] = {}

    def register(self, connector: BaseConnector):
        if connector.name in self._connectors:
            logger.warning("connector %s already registered, overwriting", connector.name)
        self._connectors[connector.name] = connector

    def get(self, name: str) -> Optional[BaseConnector]:
        return self._connectors.get(name)

    def all(self) -> list[BaseConnector]:
        return list(self._connectors.values())

    def names(self) -> list[str]:
        return list(self._connectors.keys())

    async def start_all(self):
        for c in self._connectors.values():
            await c.start()

    async def stop_all(self):
        for c in self._connectors.values():
            await c.stop()


registry = ConnectorRegistry()


def init_connectors():
    """启动时调用 — 根据 config 决定要拉起哪些 connector。

    Sprint 1:只在 KUBECONFIGS 有配置时才启动 K8sConnector。
    没配 KUBECONFIGS → 兜底走 ~/.kube/config(本地开发场景)。
    """
    if not settings.connectors_autostart:
        logger.info("connectors_autostart=0, skipping registration")
        return

    # K8sConnector — 至少注册一次(配不配 kubeconfig 由 connector 内部 fallback)
    k8s_conn = K8sConnector()
    registry.register(k8s_conn)
    logger.info(
        "registered k8s connector: cluster=%s namespace=%s kubeconfig=%s",
        k8s_conn.cluster_id, k8s_conn.namespace, k8s_conn.kubeconfig_path or "(default)",
    )


async def start_all_connectors():
    await registry.start_all()


async def stop_all_connectors():
    await registry.stop_all()
