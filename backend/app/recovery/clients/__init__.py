"""PRD-001 Phase 2 真实外部客户端(MySQL / Redis)。

handler 在 real 模式下用这些客户端调真实集群。同步实现 —— 无 async/sync mismatch。
连接信息优先取 DSS 节点 properties(host/port),缺失走 settings 全局默认。
"""
