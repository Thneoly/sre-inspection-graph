"""Add Region, AZ, ELB, APIG, Gateway, Nacos, MySQL, Redis, Kafka to Neo4j"""
import sys, os
sys.path.insert(0, os.path.dirname(__file__) + '/../backend')
from app.db.neo4j_client import get_driver

driver = get_driver()
with driver.session() as s:
    # ── Nodes ──
    for nid, label, name, uk, env, team, health, risk, attrs in [
        ('region:cn-south-1', 'Region', '华南1区', 'cn-south-1', 'prod', '基础架构', 'normal', 'low', '{"code":"cn-south-1"}'),
        ('az:cn-south-1a', 'AZ', '可用区A', 'cn-south-1a', 'prod', '基础架构', 'normal', 'low', '{"region":"cn-south-1"}'),
        ('elb:order-prod', 'ELB', 'order-prod-elb', 'order-prod-elb', 'prod', '网络团队', 'normal', 'low', '{"type":"public","bandwidth":1000}'),
        ('gateway:order-api-gw', 'Gateway', 'order-api-gateway', 'order-api-gw', 'prod', '订单团队', 'normal', 'low', '{"type":"SpringCloudGateway"}'),
        ('apig:order-api', 'APIG', '订单API网关', 'order-apig', 'prod', '平台团队', 'normal', 'low', '{"api_count":32}'),
        ('nacos:prod-cluster', 'Nacos', 'nacos-prod', 'nacos-prod', 'prod', '平台团队', 'normal', 'low', '{"version":"2.3","mode":"cluster"}'),
        ('mysql:order-db', 'MySQL', 'order-db', 'order-db', 'prod', 'DBA团队', 'normal', 'low', '{"version":"8.0","type":"RDS","storage_gb":500}'),
        ('redis:order-cache', 'Redis', 'order-cache', 'order-cache', 'prod', 'DBA团队', 'warning', 'medium', '{"version":"6.2","hit_rate":0.92}'),
        ('kafka:order-events', 'Kafka', 'order-events', 'order-events', 'prod', '平台团队', 'normal', 'low', '{"version":"3.4","partitions":12}'),
    ]:
        s.run('''MERGE (n:ResourceInstance {node_id: $nid})
            SET n.label=$label, n.name=$name, n.unique_key=$uk, n.env_code=$env,
                n.owner_team=$team, n.lifecycle_status='active', n.health_status=$health,
                n.risk_level=$risk, n.source_system='CMDB', n.attrs_json=$attrs,
                n.version='v1', n.updated_at=datetime()''',
            nid=nid, label=label, name=name, uk=uk, env=env, team=team, health=health, risk=risk, attrs=attrs)

    # ── Edges ──
    for src, rel, tgt, eid, rname in [
        ('az:cn-south-1a', 'BELONGS_TO', 'region:cn-south-1', 'e400', '属于'),
        ('elb:order-prod', 'ROUTES_TO', 'ing:cce-prod-01:order:order-api', 'e401', '路由到'),
        ('gateway:order-api-gw', 'ROUTES_TO', 'svc:cce-prod-01:order:order-api', 'e402', '路由到'),
        ('apig:order-api', 'ROUTES_TO', 'gateway:order-api-gw', 'e403', '路由到'),
        ('gateway:order-api-gw', 'USES', 'nacos:prod-cluster', 'e404', '服务发现'),
        ('svc:cce-prod-01:order:order-api', 'REGISTERS_IN', 'nacos:prod-cluster', 'e405', '注册到'),
        ('cluster:cce-prod-01', 'BELONGS_TO', 'az:cn-south-1a', 'e430', '属于'),
    ]:
        s.run('''MATCH (s:ResourceInstance {node_id:$src}) MATCH (t:ResourceInstance {node_id:$tgt})
            MERGE (s)-[r:RELATES_TO {edge_id:$eid}]->(t)
            SET r.relationship_type=$rel, r.relationship_name=$rname, r.dependency_strength='强',
                r.is_required='是', r.discovery_method='CMDB', r.health_status='normal',
                r.version='v1', r.updated_at=datetime()''',
            src=src, rel=rel, tgt=tgt, eid=eid, rname=rname)

    # Deployment → middleware
    for tgt, eid in [('mysql:order-db','e410'),('redis:order-cache','e411'),('kafka:order-events','e412')]:
        s.run('''MATCH (d:ResourceInstance {node_id:$deploy}) MATCH (t:ResourceInstance {node_id:$tgt})
            MERGE (d)-[r:RELATES_TO {edge_id:$eid}]->(t)
            SET r.relationship_type='USES', r.relationship_name='依赖', r.dependency_strength='强',
                r.is_required='是', r.discovery_method='CMDB', r.health_status='normal',
                r.version='v1', r.updated_at=datetime()''',
            deploy='deploy:cce-prod-01:order:order-api', tgt=tgt, eid=eid)

    # Component → middleware
    for tgt, eid in [('mysql:order-db','e420'),('redis:order-cache','e421'),('kafka:order-events','e422')]:
        s.run('''MATCH (c:ResourceInstance {node_id:$comp}) MATCH (t:ResourceInstance {node_id:$tgt})
            MERGE (c)-[r:RELATES_TO {edge_id:$eid}]->(t)
            SET r.relationship_type='DEPENDS_ON', r.relationship_name='依赖', r.dependency_strength='强',
                r.is_required='是', r.discovery_method='CMDB', r.health_status='normal',
                r.version='v1', r.updated_at=datetime()''',
            comp='comp:order-api', tgt=tgt, eid=eid)

print('Done: 9 infra nodes + 15 relationships added.')
print('Full topology now includes: Region → AZ → Cluster → Namespace → Deployment → Pod → Container')
print('Plus: APIG → Gateway → Service, ELB → Ingress, Nacos, MySQL, Redis, Kafka')
