"""Recovery Action Handlers — 每种动作的真实执行逻辑。

Sprint 2 范围:3 种 low_risk 动作可端到端执行(scale_deployment / kill_query
/ restart_service)。

设计:
- 每个 handler 是一个**纯函数** `execute(target, params, context) -> dict`
- 不直接调真实 K8s/MySQL/Redis API(Sprint 2 没有真实环境)——
  改为**模拟执行**:在 DSS 内存孪生体里更新节点状态,代表"动作生效"
- Phase 2(2027 H1)接入真实 K8s 时,handler 内部换成 client-go 调用即可

注册器(`HANDLERS` dict)是单一查询入口,新增动作只需:
  1. 在 ACTION_DEFS 加配置
  2. 在 handlers/ 加一个 .py
  3. 在 handlers/__init__.py 注册
"""

from app.recovery.handlers.scale_deployment import execute as scale_deployment_execute
from app.recovery.handlers.kill_query import execute as kill_query_execute
from app.recovery.handlers.restart_service import execute as restart_service_execute


HANDLERS: dict[str, callable] = {
    "scale_deployment": scale_deployment_execute,
    "kill_query": kill_query_execute,
    "restart_service": restart_service_execute,
}


def get_handler(action_id: str):
    """获取指定动作的 execute handler。

    返回 None 表示该动作 Sprint 2 暂未实现执行。
    """
    return HANDLERS.get(action_id)


def is_executable(action_id: str) -> bool:
    """判断动作是否在 Sprint 2 可执行清单。"""
    return action_id in HANDLERS
