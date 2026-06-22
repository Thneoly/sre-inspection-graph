"""Redis 客户端 — PRD-001 Phase 2 clear_cache handler 用。

redis-py 同步实现。连接信息优先 DSS 节点 properties(host/port/db),
缺失走 settings 全局默认。三种 scope:FLUSHALL / FLUSHDB / SCAN+DEL(pattern)。
"""
from __future__ import annotations

import logging

from app.config import settings


logger = logging.getLogger(__name__)


class RedisClient:
    def __init__(self, host: str = "", port: int = 6379, password: str = "", db: int = 0):
        self.host = host
        self.port = port
        self.password = password
        self.db = db
        self._client = None

    @classmethod
    def from_node(cls, node) -> "RedisClient":
        """从 DSS Redis 节点 properties 构造,缺失走 settings 默认。"""
        props = (node.properties or {}) if node else {}
        host = props.get("host") or settings.redis_host
        port = int(props.get("port") or settings.redis_port)
        password = props.get("password") or settings.redis_password
        db = int(props.get("db") or settings.redis_db)
        if not host:
            raise ValueError(
                "Redis host not configured (neither node.properties.host nor REDIS_HOST env)"
            )
        return cls(host=host, port=port, password=password, db=db)

    def connect(self):
        if self._client is not None:
            return self._client
        import redis
        self._client = redis.Redis(
            host=self.host, port=self.port, password=self.password or None,
            db=self.db, socket_connect_timeout=10, socket_timeout=30,
        )
        # 主动 ping 验证连接
        self._client.ping()
        logger.info("RedisClient connected to %s:%s db=%s", self.host, self.port, self.db)
        return self._client

    def flush_all(self) -> int:
        """FLUSHALL — 清所有 db。返回删除的 key 数(异步统计,approx)。"""
        client = self.connect()
        return client.flushall()

    def flush_db(self, db_index: int) -> int:
        """FLUSHDB on 指定 db。"""
        client = self.connect()
        if db_index != self.db:
            # 切到目标 db
            import redis
            other = redis.Redis(
                host=self.host, port=self.port, password=self.password or None,
                db=db_index, socket_connect_timeout=10, socket_timeout=30,
            )
            try:
                return other.flushdb()
            finally:
                other.close()
        return client.flushdb()

    def delete_pattern(self, pattern: str) -> int:
        """SCAN + DEL 匹配 pattern 的 key(不用 KEYS 避免阻塞)。返回删除数。"""
        client = self.connect()
        deleted = 0
        pipe = client.pipeline()
        for key in client.scan_iter(match=pattern, count=500):
            pipe.delete(key)
            deleted += 1
            if deleted % 500 == 0:
                pipe.execute()
                pipe = client.pipeline()
        pipe.execute()
        return deleted

    def close(self) -> None:
        if self._client is not None:
            try:
                self._client.close()
            except Exception:
                pass
            self._client = None
