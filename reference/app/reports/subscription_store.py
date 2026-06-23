"""订阅存储 — PRD-003 Sprint 2。

`ReportSubscription` 描述一条订阅规则:模板 + scope + cron + 收件人。
进程级单例 `subscription_store`,模式同 DSS store / report_store。

Neo4j dual-write 在 `app.reports.persistence`,启动时 `load_subscriptions_from_neo4j`
反向 hydrate 进 store,uvicorn 重启订阅不丢。
"""

from __future__ import annotations

import uuid
from dataclasses import dataclass, field
from typing import Any, Optional


@dataclass
class ReportSubscription:
    """一条订阅规则。"""

    subscription_id: str
    template_id: str
    scope: dict[str, Any]
    modules: list[str]
    cron: str                       # 5-field cron, 例 "0 9 * * 1"
    recipients: list[str]           # email 列表
    enabled: bool = True
    created_at: str = ""
    last_run_at: str = ""
    last_status: str = "never"      # never | ok | failed
    last_error: str = ""
    last_report_id: str = ""

    def to_dict(self) -> dict[str, Any]:
        return {
            "subscription_id": self.subscription_id,
            "template_id": self.template_id,
            "scope": self.scope,
            "modules": list(self.modules),
            "cron": self.cron,
            "recipients": list(self.recipients),
            "enabled": self.enabled,
            "created_at": self.created_at,
            "last_run_at": self.last_run_at,
            "last_status": self.last_status,
            "last_error": self.last_error,
            "last_report_id": self.last_report_id,
        }


class SubscriptionStore:
    def __init__(self) -> None:
        self.subscriptions: dict[str, ReportSubscription] = {}

    def add(self, sub: ReportSubscription) -> ReportSubscription:
        self.subscriptions[sub.subscription_id] = sub
        return sub

    def get(self, sub_id: str) -> Optional[ReportSubscription]:
        return self.subscriptions.get(sub_id)

    def update(self, sub_id: str, **fields: Any) -> Optional[ReportSubscription]:
        sub = self.subscriptions.get(sub_id)
        if sub is None:
            return None
        for k, v in fields.items():
            if hasattr(sub, k):
                setattr(sub, k, v)
        return sub

    def delete(self, sub_id: str) -> bool:
        return self.subscriptions.pop(sub_id, None) is not None

    def list(
        self,
        template_id: Optional[str] = None,
        application_id: Optional[str] = None,
    ) -> list[ReportSubscription]:
        subs = list(self.subscriptions.values())
        if template_id:
            subs = [s for s in subs if s.template_id == template_id]
        if application_id:
            subs = [s for s in subs if s.scope.get("application_id") == application_id]
        subs.sort(key=lambda s: s.created_at, reverse=True)
        return subs

    def clear(self) -> None:
        self.subscriptions.clear()


subscription_store = SubscriptionStore()


def new_subscription_id() -> str:
    return f"sub-{uuid.uuid4().hex[:12]}"
