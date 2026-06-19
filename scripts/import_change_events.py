#!/usr/bin/env python3
"""CSV → Neo4j bulk import for ChangeEvent — PRD-002 Sprint 2。

读 scripts/output/change_events.csv (`generate_change_events.py --csv` 的输出),
批量 MERGE 到 Neo4j。结构对标 backend/app/changes/event_service.py:_persist_change_event。

用法:
  python scripts/import_change_events.py
  python scripts/import_change_events.py --csv /path/to/change_events.csv

设计:
- 直接走 Python driver 的 UNWIND-based batch import,不用 LOAD CSV
  (LOAD CSV 需要把文件拷到 Neo4j 容器的 /import/,运维麻烦)
- target 节点不存在则只丢边,不创 stub
- 主存储仍是 DSS;本脚本只补刀 Neo4j(测试 / 演示用)
"""

from __future__ import annotations

import argparse
import csv
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "backend"))

from app.db import neo4j_client as n4j  # noqa: E402


DEFAULT_CSV = REPO_ROOT / "scripts" / "output" / "change_events.csv"

# 一次 UNWIND 批量大小 — 200 行单事务,Neo4j 5 推荐 ≤10k
BATCH_SIZE = 200


# ============================================================
# Cypher 模板 — 与 _persist_change_event 完全对齐
# ============================================================

MERGE_NODE_BATCH = """
UNWIND $events AS ev
MERGE (e:ChangeEvent:ResourceInstance {node_id: ev.change_event_id})
SET e.change_event_id = ev.change_event_id,
    e.change_type = ev.change_type,
    e.target_resource_id = ev.target_resource_id,
    e.target_resource_type = ev.target_resource_type,
    e.changed_at = ev.changed_at,
    e.changed_by = ev.changed_by,
    e.source = ev.source,
    e.description = ev.description,
    e.diff_summary_json = ev.diff_summary_json,
    e.related_commit = '',
    e.related_pr = '',
    e.severity_estimate = ev.severity_estimate,
    e.propagated_to = ev.propagated_to,
    e.propagated_count = ev.propagated_count,
    e.label = 'ChangeEvent',
    e.name = ev.change_type,
    e.health_status = 'green',
    e.version = 'v1',
    e.updated_at = datetime()
"""

MERGE_EDGE_BATCH = """
UNWIND $events AS ev
MATCH (e:ChangeEvent {change_event_id: ev.change_event_id})
MATCH (t:ResourceInstance {node_id: ev.target_resource_id})
MERGE (e)-[r:RELATES_TO {edge_id: 'change_target_' + ev.change_event_id}]->(t)
SET r.relationship_type = 'CHANGED',
    r.relationship_name = '变更',
    r.dependency_strength = '弱',
    r.last_verified_at = datetime(),
    r.version = 'v1'
"""


def _read_csv(path: Path) -> list[dict]:
    """读 CSV 转字典列表。CSV 没有 target_resource_type / severity / propagated_to —
    bulk import 走简化模式(target_type 留空,severity=low,propagated_to=[])。
    record_change 走运行时路径才有完整字段。"""
    if not path.exists():
        raise FileNotFoundError(
            f"CSV 不存在:{path}\n"
            f"先跑:python scripts/generate_change_events.py --csv"
        )
    rows: list[dict] = []
    with path.open("r", encoding="utf-8") as f:
        reader = csv.DictReader(f)
        for row in reader:
            rows.append({
                "change_event_id": row["change_event_id"],
                "change_type": row["change_type"],
                "target_resource_id": row["target_resource_id"],
                "target_resource_type": "",  # CSV 不带,留空
                "changed_at": row["changed_at"],
                "changed_by": row["changed_by"],
                "source": row["source"],
                "description": row["description"],
                "diff_summary_json": row.get("diff_summary_json", "{}"),
                "severity_estimate": "low",  # CSV 不带,简化
                "propagated_to": [],
                "propagated_count": 0,
            })
    return rows


def _batch(seq: list, n: int):
    for i in range(0, len(seq), n):
        yield seq[i : i + n]


def import_events(csv_path: Path) -> tuple[int, int]:
    """主流程。返回 (节点写入数, 边写入数)。"""
    rows = _read_csv(csv_path)
    if not rows:
        print("CSV 空,什么都不做")
        return 0, 0

    print(f"读到 {len(rows)} 个 ChangeEvent,开始 batch import")

    driver = n4j.get_driver()
    nodes_written = 0
    edges_written = 0

    with driver.session() as s:
        # Phase 1 — 全部节点先 MERGE 完
        for batch in _batch(rows, BATCH_SIZE):
            s.run(MERGE_NODE_BATCH, events=batch)
            nodes_written += len(batch)
            print(f"  nodes: {nodes_written}/{len(rows)}")

        # Phase 2 — 边(target 不在则被 MATCH 过滤掉)
        for batch in _batch(rows, BATCH_SIZE):
            res = s.run(MERGE_EDGE_BATCH + " RETURN count(r) AS n", events=batch)
            n = res.single()["n"]
            edges_written += n

    return nodes_written, edges_written


def main():
    ap = argparse.ArgumentParser(description="CSV → Neo4j bulk import for ChangeEvent")
    ap.add_argument("--csv", type=Path, default=DEFAULT_CSV,
                    help=f"输入 CSV(默认 {DEFAULT_CSV})")
    args = ap.parse_args()

    nodes, edges = import_events(args.csv)
    print(f"\n✓ 导入完成 — {nodes} 节点 / {edges} 边")
    print("  验证: cypher-shell -u neo4j -p sre-inspection "
          "'MATCH (e:ChangeEvent) RETURN count(e)'")


if __name__ == "__main__":
    main()
