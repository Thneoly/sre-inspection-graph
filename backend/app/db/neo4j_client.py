"""Neo4j Client — 单例驱动封装"""
from neo4j import GraphDatabase, Driver
from app.config import settings

_driver: Driver | None = None


def get_driver() -> Driver:
    """获取 Neo4j 驱动实例（懒加载单例）"""
    global _driver
    if _driver is None:
        _driver = GraphDatabase.driver(
            settings.neo4j_uri,
            auth=(settings.neo4j_user, settings.neo4j_password),
            max_connection_lifetime=settings.neo4j_max_connection_lifetime,
            max_connection_pool_size=settings.neo4j_max_connection_pool_size,
            connection_acquisition_timeout=settings.neo4j_connection_acquisition_timeout,
        )
    return _driver


def close_driver():
    """关闭驱动"""
    global _driver
    if _driver is not None:
        _driver.close()
        _driver = None


def run_query(cypher: str, params: dict | None = None) -> list:
    """执行只读查询，返回 Neo4j 原生 Record 对象列表（保留 Path/Node/Relationship 类型）"""
    driver = get_driver()
    with driver.session() as session:
        result = session.run(cypher, params or {})
        return list(result)


def check_connection() -> bool:
    """检查 Neo4j 连接是否正常"""
    try:
        driver = get_driver()
        with driver.session() as session:
            result = session.run("RETURN 1 AS ok")
            return result.single().get("ok") == 1
    except Exception:
        return False
