"""从 Neo4j 加载基线数据到 DSS"""
from app.datasource.models import DataNode, DataEdge
from app.datasource.store import store
from app.db.neo4j_client import run_query


def load_baseline():
    """从 Neo4j 加载所有节点和边到内存，并恢复活跃故障状态"""
    if store._initialized:
        return

    eid_to_nid: dict[str, str] = {}

    # Load nodes
    rows = run_query("MATCH (n:ResourceInstance) RETURN n")
    for row in rows:
        n = row["n"]
        node_id = str(n.get("node_id", n.element_id))
        eid_to_nid[n.element_id] = node_id

        props = {}
        for k, v in dict(n).items():
            if k.startswith("_"):
                continue
            if hasattr(v, 'isoformat'):
                props[k] = v.isoformat()
            elif isinstance(v, (str, int, float, bool, type(None))):
                props[k] = v
            else:
                props[k] = str(v)

        node = DataNode(
            id=node_id,
            type=str(n.get("label", "Unknown")),
            name=str(n.get("name", node_id)),
            properties=props,
        )
        store.upsert_node(node)

    # Load edges — must explicitly return start/end nodes with properties
    edge_rows = run_query("MATCH (a:ResourceInstance)-[r:RELATES_TO]->(b:ResourceInstance) RETURN a, r, b")
    for row in edge_rows:
        r = row["r"]
        a = row["a"]
        b = row["b"]

        src_id = str(a.get("node_id", a.element_id))
        tgt_id = str(b.get("node_id", b.element_id))
        edge_id = str(r.get("edge_id", r.element_id))

        props = {}
        for k, v in dict(r).items():
            if k.startswith("_"):
                continue
            if hasattr(v, 'isoformat'):
                props[k] = v.isoformat()
            elif isinstance(v, (str, int, float, bool, type(None))):
                props[k] = v
            else:
                props[k] = str(v)

        edge = DataEdge(
            id=edge_id,
            source_id=src_id,
            target_id=tgt_id,
            relationship_type=str(r.get("relationship_type", r.type)),
            relationship_name=str(r.get("relationship_name", "")),
            properties=props,
        )
        store.upsert_edge(edge)

    store._initialized = True

    # Recover active faults from Neo4j
    _recover_faults()

    print(f"DSS loaded: {len(store.nodes)} nodes, {len(store.edges)} edges")


def sync_to_neo4j():
    """将 DSS 中的实时属性同步回 Neo4j"""
    from app.db.neo4j_client import get_driver
    driver = get_driver()
    with driver.session() as s:
        for node in store.get_all_nodes():
            health = node.properties.get("health_status", "normal")
            risk = node.properties.get("risk_level", "low")
            s.run("MATCH (n:ResourceInstance {node_id: $id}) SET n.health_status = $h, n.risk_level = $r, n.updated_at = datetime()",
                  id=node.id, h=health, r=risk)
        for edge in store.get_all_edges():
            health = edge.properties.get("health_status", "normal")
            s.run("MATCH ()-[r:RELATES_TO {edge_id: $id}]->() SET r.health_status = $h",
                  id=edge.id, h=health)
    print(f"DSS synced {len(store.nodes)} nodes, {len(store.edges)} edges → Neo4j")


def reset_dss():
    """重置 DSS：清空实时数据，重新加载基线"""
    store.reset()
    store._initialized = False
    load_baseline()
    print(f"DSS reset complete. {len(store.nodes)} nodes at baseline.")


def _recover_faults():
    """从 Neo4j 恢复活跃故障状态到 DSS"""
    from app.datasource.fault_injector import FAULT_DEFS, FaultStage, FaultInjection
    from datetime import datetime, timezone

    rows = run_query("MATCH (fs:FaultScenario) WHERE fs.status IN ['injected','escalating','propagating'] RETURN fs")
    if not rows:
        return

    for row in rows:
        fs = row["fs"]
        fid = fs.get("scenario_id", "")
        ft_code = fs.get("fault_type", "")
        tid = fs.get("target_resource_id", "")
        stage = int(fs.get("current_stage", 0))
        ft = FAULT_DEFS.get(ft_code)
        if not ft:
            continue

        # Reconstruct fault with stages
        stages = []
        for i, s in enumerate(ft["stages"]):
            stages.append(FaultStage(
                sequence=i, offset_seconds=s["s"],
                health=s["h"], risk=s["r"],
                metric_name=s.get("m", ""), metric_value=s.get("v", 0.0),
                unit=s.get("u", "percent"),
                triggers_alert=s.get("alert", False),
                triggers_finding=s.get("finding", False),
            ))

        fault = FaultInjection(
            injection_id=fid, fault_type=ft_code, target_id=tid,
            current_stage=stage, total_stages=len(stages),
            status=fs.get("status", "injected"),
            injected_at=str(fs.get("injected_at", "")),
            stages=stages,
        )
        store.add_fault(fault)

        # Re-apply current stage to target and blast radius
        from app.datasource.fault_injector import _apply_stage, _apply_blast_radius
        _apply_stage(fault, stage)
        _apply_blast_radius(fault, stage)

        # Re-propagate
        stg = stages[stage]
        from app.datasource.fault_injector import _propagate
        _propagate(tid, stg.health, stg.risk, ft)

    print(f"DSS recovered {len(rows)} active faults from Neo4j")
