"""应用配置 — 从环境变量加载"""
import os
from dataclasses import dataclass


@dataclass
class Settings:
    neo4j_uri: str = os.getenv("NEO4J_URI", "bolt://localhost:7687")
    neo4j_user: str = os.getenv("NEO4J_USER", "neo4j")
    neo4j_password: str = os.getenv("NEO4J_PASSWORD", "sre-inspection")
    neo4j_max_connection_lifetime: int = 3600
    neo4j_max_connection_pool_size: int = 50
    neo4j_connection_acquisition_timeout: int = 10


settings = Settings()
