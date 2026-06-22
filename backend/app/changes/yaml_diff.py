"""YAML diff 工具 — PRD-002 Phase 2。

把两个 K8s 资源对象(spec / data / 字段子集)对比成 unified diff 文本,
供 ChangeEvent.yaml_diff 字段存档 + 前端 `<pre>` 渲染。

设计:
- 纯标准库(yaml + difflib),无第三方依赖
- 默认剔除 K8s 噪声字段(managedFields / resourceVersion / uid / creationTimestamp /
  generation / selfLink / resourceVersion / etag),避免每次变更都报一堆元数据 diff
- `keys` 参数可限定只对比某些顶层 key(如只看 data / spec.template)
"""
from __future__ import annotations

import difflib
from typing import Any, Iterable, Optional

import yaml


# K8s 对象里纯元数据、变更无业务意义的字段 —— diff 前剔除
_NOISE_KEYS: set[str] = {
    "managedFields",
    "resourceVersion",
    "uid",
    "creationTimestamp",
    "generation",
    "selfLink",
    "etag",
    "last-applied-configuration",
    "annotations",  # 常含 kubectl/argo 注入的噪声,业务无关
    "managedVersion",
}


def _strip_noise(obj: Any) -> Any:
    """递归剔除噪声字段(dict 层级)。"""
    if isinstance(obj, dict):
        return {
            k: _strip_noise(v)
            for k, v in obj.items()
            if k not in _NOISE_KEYS
        }
    if isinstance(obj, list):
        return [_strip_noise(item) for item in obj]
    return obj


def _select_keys(obj: dict, keys: Optional[Iterable[str]]) -> dict:
    """只保留指定顶层 key;keys=None 则全保留。"""
    if keys is None:
        return obj
    wanted = set(keys)
    return {k: v for k, v in obj.items() if k in wanted}


def compute_yaml_diff(
    old_obj: Optional[dict],
    new_obj: Optional[dict],
    keys: Optional[Iterable[str]] = None,
    name: str = "resource",
) -> str:
    """对比两个对象 → unified diff 文本。

    - old_obj / new_obj 为空 dict / None 视为新增 / 删除
    - keys 限定只对比某些顶层 key(如 ("data", "spec"))
    - 返回空串表示无业务差异(剔除噪声后一致)
    """
    old = _strip_noise(_select_keys(old_obj or {}, keys))
    new = _strip_noise(_select_keys(new_obj or {}, keys))

    old_yaml = yaml.safe_dump(old, default_flow_style=False, sort_keys=True, allow_unicode=True)
    new_yaml = yaml.safe_dump(new, default_flow_style=False, sort_keys=True, allow_unicode=True)

    if old_yaml == new_yaml:
        return ""

    diff_lines = list(difflib.unified_diff(
        old_yaml.splitlines(keepends=True),
        new_yaml.splitlines(keepends=True),
        fromfile=f"{name}.old",
        tofile=f"{name}.new",
        n=3,
    ))
    return "".join(diff_lines).rstrip("\n")


def summarize_diff(diff_text: str) -> dict[str, Any]:
    """从 unified diff 文本解析统计,给 ChangeEvent.diff_summary 用。

    返回 {added, removed, changed_keys}。changed_keys 是去重的顶层 key 名
    (从 `@@ ... @@ key: value` 行启发式提取,粗粒度)。
    """
    if not diff_text:
        return {"added": 0, "removed": 0, "changed_keys": []}

    added = 0
    removed = 0
    changed_keys: set[str] = set()

    for line in diff_text.splitlines():
        if line.startswith("+++") or line.startswith("---"):
            continue
        if line.startswith("+"):
            added += 1
            key = _extract_key(line[1:])
            if key:
                changed_keys.add(key)
        elif line.startswith("-"):
            removed += 1
            key = _extract_key(line[1:])
            if key:
                changed_keys.add(key)

    return {
        "added": added,
        "removed": removed,
        "changed_keys": sorted(changed_keys),
    }


def _extract_key(line: str) -> str:
    """从 YAML 行提取顶层 key(如 `  max_pool_size: 50` → `max_pool_size`)。

    粗粒度:取第一个非空白、非 `-` 的 `:` 前部分。嵌套 key 不展开。
    """
    stripped = line.lstrip()
    if not stripped or stripped.startswith("-") or stripped.startswith("#"):
        return ""
    if ":" not in stripped:
        return ""
    return stripped.split(":", 1)[0].strip()
