"""MySQL 客户端 — PRD-001 Phase 2 kill_query handler 用。

pymysql 同步实现。连接信息优先 DSS 节点 properties(host/port/db),
缺失走 settings 全局默认(MYSQL_HOST 等)。都缺 → connect() 抛 ValueError。

`SHOW PROCESSLIST` 列出连接,`KILL <conn_id>` 终止。query_id 在 DSS 语义里
就是 MySQL 的 connection id(与 PRD-001 mock 对齐)。
"""
from __future__ import annotations

import logging
from typing import Any

from app.config import settings


logger = logging.getLogger(__name__)


class MySQLClient:
    def __init__(self, host: str = "", port: int = 3306, user: str = "",
                 password: str = "", database: str = ""):
        self.host = host
        self.port = port
        self.user = user
        self.password = password
        self.database = database
        self._conn = None

    @classmethod
    def from_node(cls, node) -> "MySQLClient":
        """从 DSS MySQL 节点 properties 构造,缺失字段走 settings 默认。"""
        props = (node.properties or {}) if node else {}
        host = props.get("host") or settings.mysql_host
        port = int(props.get("port") or settings.mysql_port)
        user = props.get("user") or settings.mysql_user
        password = props.get("password") or settings.mysql_password
        database = props.get("database") or settings.mysql_database
        if not host:
            raise ValueError(
                "MySQL host not configured (neither node.properties.host nor MYSQL_HOST env)"
            )
        return cls(host=host, port=port, user=user, password=password, database=database)

    def connect(self):
        if self._conn is not None:
            return self._conn
        import pymysql
        self._conn = pymysql.connect(
            host=self.host, port=self.port, user=self.user,
            password=self.password, database=self.database or None,
            connect_timeout=10, read_timeout=30,
        )
        logger.info("MySQLClient connected to %s:%s db=%s", self.host, self.port, self.database)
        return self._conn

    def list_processes(self) -> list[dict[str, Any]]:
        """SHOW PROCESSLIST → 列表(dict per row)。"""
        conn = self.connect()
        with conn.cursor() as cur:
            cur.execute("SHOW PROCESSLIST")
            cols = [d[0] for d in cur.description]
            return [dict(zip(cols, row)) for row in cur.fetchall()]

    def kill(self, conn_id: int) -> None:
        """KILL <conn_id>。conn_id 即 MySQL processlist 的 Id 字段。"""
        conn = self.connect()
        with conn.cursor() as cur:
            cur.execute(f"KILL {int(conn_id)}")
        conn.commit()

    def close(self) -> None:
        if self._conn is not None:
            try:
                self._conn.close()
            except Exception:
                pass
            self._conn = None
