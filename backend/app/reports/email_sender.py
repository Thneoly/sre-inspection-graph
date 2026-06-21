"""邮件发送 — PRD-003 Sprint 2。

抽象 EmailSender + 两实现:
- `SmtpEmailSender`:stdlib smtplib,生产用(SMTP_HOST env)
- `InMemoryEmailSender`:默认 / 测试,把 (recipients, subject, body, attachments) 存 self.sent

工厂 `get_email_sender()` 按环境变量决定 + 进程级单例。
"""

from __future__ import annotations

import logging
import os
import smtplib
from abc import ABC, abstractmethod
from email.mime.multipart import MIMEMultipart
from email.mime.text import MIMEText
from typing import Any, Optional


logger = logging.getLogger(__name__)


class EmailSender(ABC):
    @abstractmethod
    def send(
        self,
        recipients: list[str],
        subject: str,
        body: str,
        attachments: Optional[list[dict[str, Any]]] = None,
    ) -> None:
        """发送邮件。attachments 为 [{filename, content, mimetype}]。"""

    def name(self) -> str:
        return self.__class__.__name__


class InMemoryEmailSender(EmailSender):
    """测试 / 本地默认实现。所有发送累计到 self.sent。"""

    def __init__(self) -> None:
        self.sent: list[dict[str, Any]] = []

    def send(
        self,
        recipients: list[str],
        subject: str,
        body: str,
        attachments: Optional[list[dict[str, Any]]] = None,
    ) -> None:
        self.sent.append({
            "recipients": list(recipients),
            "subject": subject,
            "body": body,
            "attachments": list(attachments or []),
        })

    def clear(self) -> None:
        self.sent.clear()


class SmtpEmailSender(EmailSender):
    """生产 SMTP 实现。stdlib smtplib + email.mime。"""

    def __init__(
        self,
        host: str,
        port: int = 25,
        user: Optional[str] = None,
        password: Optional[str] = None,
        from_addr: str = "sre-platform@example.com",
        use_tls: bool = False,
    ) -> None:
        self.host = host
        self.port = port
        self.user = user
        self.password = password
        self.from_addr = from_addr
        self.use_tls = use_tls

    def send(
        self,
        recipients: list[str],
        subject: str,
        body: str,
        attachments: Optional[list[dict[str, Any]]] = None,
    ) -> None:
        msg = MIMEMultipart()
        msg["From"] = self.from_addr
        msg["To"] = ", ".join(recipients)
        msg["Subject"] = subject
        # Markdown body as plain text — 多数邮件客户端 monospace 渲染足够清晰
        msg.attach(MIMEText(body, "plain", "utf-8"))

        for att in attachments or []:
            payload = MIMEText(att.get("content", ""), "plain", "utf-8")
            payload.add_header(
                "Content-Disposition", "attachment",
                filename=att.get("filename", "attachment.md"),
            )
            msg.attach(payload)

        try:
            with smtplib.SMTP(self.host, self.port, timeout=20) as smtp:
                if self.use_tls:
                    smtp.starttls()
                if self.user:
                    smtp.login(self.user, self.password or "")
                smtp.sendmail(self.from_addr, recipients, msg.as_string())
        except Exception:
            logger.exception("SMTP send failed: host=%s recipients=%s", self.host, recipients)
            raise


_sender_singleton: Optional[EmailSender] = None


def get_email_sender() -> EmailSender:
    """单例工厂。SMTP_HOST 配置 → SmtpEmailSender;否则 InMemoryEmailSender。

    切换:`reset_email_sender()` 后再次调用。
    """
    global _sender_singleton
    if _sender_singleton is not None:
        return _sender_singleton

    host = os.environ.get("SMTP_HOST", "").strip()
    if host:
        _sender_singleton = SmtpEmailSender(
            host=host,
            port=int(os.environ.get("SMTP_PORT", "25")),
            user=os.environ.get("SMTP_USER") or None,
            password=os.environ.get("SMTP_PASSWORD") or None,
            from_addr=os.environ.get("SMTP_FROM", "sre-platform@example.com"),
            use_tls=os.environ.get("SMTP_USE_TLS", "false").lower() == "true",
        )
    else:
        _sender_singleton = InMemoryEmailSender()
    return _sender_singleton


def reset_email_sender() -> None:
    """清单例(测试用 + lifespan 重置)。"""
    global _sender_singleton
    _sender_singleton = None
