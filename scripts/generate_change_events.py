#!/usr/bin/env python3
"""ChangeEvent mock 生成器 — PRD-002 Sprint 1。

用途:为开发与 PoC 演示填充历史变更事件,让 /correlated 和 /timeline 端点返回真实数据。

两种运行方式:

1. **写入运行中的 API**(推荐):
   $ make dev-api &
   $ python scripts/generate_change_events.py --api http://localhost:8000

   通过 POST /api/v1/change-events 调用,DSS 内存里会存住,直到下次 uvicorn 重启。

2. **导出 CSV**(Sprint 2 接 Neo4j 时复用):
   $ python scripts/generate_change_events.py --csv

   写入 scripts/output/change_events.csv,供未来导入 Neo4j。

事件分布(参考 PRD-002 第 2 节):
- configmap_updated 53%
- image_pushed     28%
- deployment_rolled 15%
- secret_rotated    4%

时间分布:跨 7 天均匀,且 30% 事件**故意聚集**在每天的"业务高峰"时段(10/14/20 点),
模拟现实里"白天发版多" + 部分事件提前 5-15 分钟于告警发生。
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import random
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

import urllib.request
import urllib.error


REPO_ROOT = Path(__file__).resolve().parent.parent
OUTPUT_DIR = Path(__file__).resolve().parent / "output"
OUTPUT_DIR.mkdir(exist_ok=True)

CHANGE_TYPE_WEIGHTS: list[tuple[str, float]] = [
    ("configmap_updated", 0.53),
    ("image_pushed", 0.28),
    ("deployment_rolled", 0.15),
    ("secret_rotated", 0.04),
]

SOURCES = ["k8s_api", "argo_cd", "gitops", "manual"]
USERS = [
    "alice@example.com", "bob@example.com", "carol@example.com",
    "ci-bot", "argo-cd", "harbor-webhook",
]

# 描述模板,按 type 区分
DESCRIPTIONS: dict[str, list[str]] = {
    "configmap_updated": [
        "调整数据库连接池配置",
        "下调 JVM heap 上限",
        "切换 feature flag",
        "修订 nginx 路由规则",
        "调整慢查询阈值",
    ],
    "secret_rotated": [
        "数据库密码定期轮换",
        "TLS 证书续签",
        "OAuth client_secret 更新",
        "云厂商 access_key 轮换",
    ],
    "deployment_rolled": [
        "上线 v1.2.3",
        "回滚至 v1.2.1",
        "扩容 3 → 5 副本",
        "切换 image registry",
        "更新 readiness probe 超时",
    ],
    "image_pushed": [
        "build pipeline 完成镜像 push",
        "热修补丁镜像 push",
        "扫描通过后推送镜像",
        "回滚镜像重新推送",
    ],
}


# ============================================================
# 选取目标资源
# ============================================================

def _pick_targets_from_dss():
    """从 DSS 拿真实节点;按 change_type 选合适类型。"""
    sys.path.insert(0, str(REPO_ROOT / "backend"))
    from app.datasource.store import store

    if not store.nodes:
        # 兜底假数据(不依赖 Neo4j)
        return _fallback_targets()

    by_type: dict[str, list[tuple[str, str]]] = {}
    for n in store.get_all_nodes():
        by_type.setdefault(n.type, []).append((n.id, n.type))
    return by_type


def _fallback_targets() -> dict[str, list[tuple[str, str]]]:
    """无 DSS 时的兜底:用 fixture 风格的资源 ID。"""
    return {
        "ConfigMap": [
            ("configmap:cce-prod-01:order:order-config", "ConfigMap"),
            ("configmap:cce-prod-01:order:nginx-config", "ConfigMap"),
            ("configmap:cce-prod-01:payment:payment-config", "ConfigMap"),
        ],
        "Secret": [
            ("secret:cce-prod-01:order:order-db-secret", "Secret"),
            ("secret:cce-prod-01:order:order-tls", "Secret"),
        ],
        "Deployment": [
            ("deploy:cce-prod-01:order:order-api", "Deployment"),
            ("deploy:cce-prod-01:order:order-worker", "Deployment"),
            ("deploy:cce-prod-01:payment:payment-api", "Deployment"),
        ],
        "ContainerImage": [
            ("image:order-api:1.2.3", "ContainerImage"),
            ("image:order-worker:1.0.4", "ContainerImage"),
        ],
    }


def _target_for(change_type: str, by_type: dict[str, list[tuple[str, str]]]) -> tuple[str, str]:
    type_map = {
        "configmap_updated": "ConfigMap",
        "secret_rotated": "Secret",
        "deployment_rolled": "Deployment",
        "image_pushed": "ContainerImage",
    }
    target_type = type_map[change_type]
    candidates = by_type.get(target_type, [])
    if not candidates:
        # DSS 里没有此类型 → 退到 fallback 同类型
        candidates = _fallback_targets().get(target_type, [])
        if not candidates:
            return ("", "")
    return random.choice(candidates)


# ============================================================
# 生成
# ============================================================

def _weighted_change_type() -> str:
    r = random.random()
    cum = 0.0
    for ctype, weight in CHANGE_TYPE_WEIGHTS:
        cum += weight
        if r <= cum:
            return ctype
    return CHANGE_TYPE_WEIGHTS[-1][0]


def _random_changed_at(now: datetime) -> str:
    """7 天内,30% 概率落在白天 10/14/20 点附近(±30 分钟)。"""
    days_back = random.randint(0, 6)
    base = now - timedelta(days=days_back)
    if random.random() < 0.30:
        hour = random.choice([10, 14, 20])
        minute = random.randint(0, 59)
        ts = base.replace(hour=hour, minute=minute, second=random.randint(0, 59), microsecond=0)
    else:
        ts = base.replace(
            hour=random.randint(0, 23),
            minute=random.randint(0, 59),
            second=random.randint(0, 59),
            microsecond=0,
        )
    return ts.strftime("%Y-%m-%dT%H:%M:%SZ")


def _diff_summary_for(change_type: str) -> dict:
    if change_type == "configmap_updated":
        return random.choice([
            {"max_pool_size": {"old": 20, "new": 50}},
            {"log_level": {"old": "INFO", "new": "DEBUG"}},
            {"feature_x_enabled": {"old": False, "new": True}},
        ])
    if change_type == "secret_rotated":
        return {"rotation_id": f"rot-{random.randint(1000, 9999)}"}
    if change_type == "deployment_rolled":
        return {
            "image": {
                "old": f"order-api:1.2.{random.randint(1, 3)}",
                "new": f"order-api:1.2.{random.randint(4, 9)}",
            }
        }
    if change_type == "image_pushed":
        return {"tag": f"1.2.{random.randint(0, 9)}"}
    return {}


def generate_events(count: int, now: datetime) -> list[dict]:
    """返回事件 dict 列表(不直接写入,由调用方决定 API/CSV)。"""
    by_type = _pick_targets_from_dss()
    events: list[dict] = []
    for _ in range(count):
        ctype = _weighted_change_type()
        target_id, target_type = _target_for(ctype, by_type)
        if not target_id:
            continue
        events.append({
            "change_type": ctype,
            "target_resource_id": target_id,
            "changed_by": random.choice(USERS),
            "source": random.choice(SOURCES),
            "description": random.choice(DESCRIPTIONS[ctype]),
            "diff_summary": _diff_summary_for(ctype),
            "changed_at": _random_changed_at(now),
        })
    # 按时间倒序整理(便于人读)
    events.sort(key=lambda e: e["changed_at"], reverse=True)
    return events


# ============================================================
# 输出
# ============================================================

def post_to_api(events: list[dict], api_base: str) -> int:
    """把事件 POST 到运行中的 API。返回成功条数。"""
    success = 0
    for ev in events:
        body = json.dumps(ev).encode("utf-8")
        req = urllib.request.Request(
            url=f"{api_base.rstrip('/')}/api/v1/change-events",
            data=body,
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        try:
            with urllib.request.urlopen(req, timeout=5) as resp:
                if resp.status in (200, 201):
                    success += 1
        except urllib.error.HTTPError as e:
            print(f"[warn] {e.code} for {ev['target_resource_id']}: {e.read().decode()[:120]}",
                  file=sys.stderr)
        except urllib.error.URLError as e:
            print(f"[err] {api_base} 不可达: {e}", file=sys.stderr)
            return success
    return success


def write_csv(events: list[dict], path: Path) -> int:
    fieldnames = [
        "change_event_id", "change_type", "target_resource_id",
        "changed_at", "changed_by", "source", "description",
        "diff_summary_json",
    ]
    with path.open("w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=fieldnames)
        writer.writeheader()
        for i, ev in enumerate(events):
            writer.writerow({
                "change_event_id": f"ce-mock-{i:04d}",
                "change_type": ev["change_type"],
                "target_resource_id": ev["target_resource_id"],
                "changed_at": ev["changed_at"],
                "changed_by": ev["changed_by"],
                "source": ev["source"],
                "description": ev["description"],
                "diff_summary_json": json.dumps(ev["diff_summary"], ensure_ascii=False),
            })
    return len(events)


# ============================================================
# CLI
# ============================================================

def main():
    parser = argparse.ArgumentParser(description="生成 ChangeEvent mock 数据")
    parser.add_argument("--count", type=int, default=150, help="事件数量(默认 150)")
    parser.add_argument("--api", type=str, default="", help="目标 API base(如 http://localhost:8000),省略则不调 API")
    parser.add_argument("--csv", action="store_true", help="额外写入 scripts/output/change_events.csv")
    parser.add_argument("--seed", type=int, default=None, help="随机种子(默认随机)")
    args = parser.parse_args()

    if args.seed is not None:
        random.seed(args.seed)

    now = datetime.now(timezone.utc)
    events = generate_events(args.count, now)
    print(f"生成 {len(events)} 条事件 (跨 7 天,now={now.strftime('%Y-%m-%dT%H:%M:%SZ')})")

    if args.csv:
        csv_path = OUTPUT_DIR / "change_events.csv"
        n = write_csv(events, csv_path)
        print(f"  → 写入 CSV {csv_path} ({n} 条)")

    if args.api:
        n = post_to_api(events, args.api)
        print(f"  → POST 到 {args.api} 成功 {n}/{len(events)} 条")

    if not args.api and not args.csv:
        # 默认仅打印前 5 条预览
        print("(预览前 5 条;加 --api 推送到 API,加 --csv 写文件)")
        for ev in events[:5]:
            print(f"  {ev['changed_at']}  {ev['change_type']:<20}  {ev['target_resource_id']}")


if __name__ == "__main__":
    main()
