"""Sync orchestrator — 启动 / 停止所有 connector,暴露给 main.py 用。

Sprint 1:K8sConnector(拓扑同步)
Sprint 2:PrometheusConnector + JaegerConnector(metric / trace 接入)
Sprint 3:FlagdConnector + K8sEventConnector(变更事件)

不在本模块:
- connector 内部循环逻辑(在 BaseConnector 里)
- 控制 API 端点(在 routers/connectors.py 里)
"""

from __future__ import annotations

import logging
from typing import Optional

from app.config import settings
from app.datasource.connectors.base import BaseConnector
from app.datasource.connectors.flagd_connector import FlagdConnector
from app.datasource.connectors.jaeger_connector import JaegerConnector
from app.datasource.connectors.k8s_connector import K8sConnector
from app.datasource.connectors.k8s_event_connector import K8sEventConnector
from app.datasource.connectors.k8s_watch_connector import K8sWatchConnector
from app.datasource.connectors.prometheus_connector import PrometheusConnector


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
    """启动时调用 — 注册全部 connector。

    每个 connector 自己看 config,如果 URL 空就空跑(不报错)。
    """
    if not settings.connectors_autostart:
        logger.info("connectors_autostart=0, skipping registration")
        return

    registry.register(K8sConnector())
    registry.register(PrometheusConnector())
    registry.register(JaegerConnector())
    registry.register(FlagdConnector())
    registry.register(K8sEventConnector())
    # PRD-002 Phase 2 — K8s watch(gate k8s_watch_enabled,默认关)
    registry.register(K8sWatchConnector())

    logger.info(
        "registered connectors: %s (cluster=%s, namespace=%s)",
        registry.names(), settings.active_cluster, settings.k8s_namespace,
    )


async def start_all_connectors():
    await registry.start_all()


async def stop_all_connectors():
    await registry.stop_all()
