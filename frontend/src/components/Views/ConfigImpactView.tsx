import { useState, useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Card, Empty, List, Select, Tag, Typography } from 'antd';
import { ReloadOutlined } from '@ant-design/icons';
import { fetchConfigImpact, fetchChangeEvents } from '../../api/client';
import GraphCanvas from '../Graph/GraphCanvas';
import GraphStatsBar from '../Graph/GraphStatsBar';
import GraphToolbar from '../Graph/GraphToolbar';
import NodeDetailPanel from '../Graph/NodeDetailPanel';
import LayerToggle from '../Graph/LayerToggle';
import { filterGraphData, defaultLayers, type LayerName } from '../../utils/layers';

const { Text } = Typography;

/** 24 小时的 ISO 时间戳。useQuery 的 queryKey 不包含它,避免每次重渲染重拉。 */
function isoTwentyFourHoursAgo(): string {
  return new Date(Date.now() - 24 * 3600 * 1000).toISOString();
}

export default function ConfigImpactView() {
  const [resourceId, setResourceId] = useState('secret:cce-prod-01:order:order-api-secret');
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [isEdge, setIsEdge] = useState(false);
  const [layers, setLayers] = useState<Set<LayerName>>(defaultLayers());

  const { data, isLoading } = useQuery({
    queryKey: ['config-impact', resourceId],
    queryFn: () => fetchConfigImpact(resourceId).then(r => r.data),
    enabled: !!resourceId,
    refetchInterval: 3000,
  });

  const filteredData = useMemo(() => filterGraphData(data, layers), [data, layers]);
  const selectedNode = !isEdge ? data?.nodes.find(n => n.id === selectedId) : undefined;
  const selectedEdge = isEdge ? data?.edges.find(e => e.id === selectedId) : undefined;

  // PRD-002 Sprint 2 — 近 24h 变更资源 Card 数据
  const { data: recentChanges } = useQuery({
    queryKey: ['change-events-24h'],
    queryFn: () => fetchChangeEvents({ since: isoTwentyFourHoursAgo(), limit: 200 }).then(r => r.data),
    refetchInterval: 30_000,
  });

  // 聚合到当前可见图节点的 24h 变更
  const visibleChanges = useMemo(() => {
    if (!recentChanges || !filteredData) return [];
    const visibleIds = new Set(filteredData.nodes.map(n => n.id));
    const counts = new Map<string, { count: number; latestAt: string; resource_type: string }>();
    for (const ev of recentChanges.events) {
      if (!visibleIds.has(ev.target_resource_id)) continue;
      const cur = counts.get(ev.target_resource_id);
      if (!cur) {
        counts.set(ev.target_resource_id, {
          count: 1,
          latestAt: ev.changed_at,
          resource_type: ev.target_resource_type,
        });
      } else {
        cur.count += 1;
        if (ev.changed_at > cur.latestAt) cur.latestAt = ev.changed_at;
      }
    }
    return Array.from(counts.entries())
      .map(([id, info]) => ({ id, ...info, name: filteredData.nodes.find(n => n.id === id)?.name ?? id }))
      .sort((a, b) => b.count - a.count)
      .slice(0, 20);
  }, [recentChanges, filteredData]);

  return (
    <div style={{ height: 'calc(100vh - 80px)', display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
      <GraphToolbar>
        <Typography.Text>配置/密钥:</Typography.Text>
        <Select value={resourceId} onChange={setResourceId} style={{ width: 420 }} options={[
          { value: 'secret:cce-prod-01:order:order-api-secret', label: 'Secret: order-api-secret' },
          { value: 'cm:cce-prod-01:order:order-api-config', label: 'ConfigMap: order-api-config' },
        ]} />
        <LayerToggle activeLayers={layers} onChange={setLayers} />
      </GraphToolbar>
      <div style={{ display: 'flex', flex: 1, overflow: 'hidden' }}>
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0, minHeight: 0 }}>
          <GraphCanvas data={filteredData} isLoading={isLoading} onNodeSelect={(id) => { setSelectedId(id); setIsEdge(false); }} onEdgeSelect={(id) => { setSelectedId(id); setIsEdge(true); }} selectedNodeId={selectedId} />
          <GraphStatsBar summary={filteredData?.summary} />
        </div>

        {/* PRD-002 Sprint 2 — 24h 变更资源侧栏 */}
        <Card
          size="small"
          title={<span><ReloadOutlined /> 近 24h 变更资源</span>}
          style={{ flex: '0 0 280px', borderLeft: '1px solid #f0f0f0', overflow: 'auto' }}
          styles={{ body: { padding: 8 } }}
          aria-label="recent-changes-panel"
        >
          {visibleChanges.length === 0 ? (
            <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="无变更" />
          ) : (
            <List
              size="small"
              dataSource={visibleChanges}
              renderItem={(item) => (
                <List.Item
                  style={{ cursor: 'pointer', padding: '6px 4px' }}
                  onClick={() => { setSelectedId(item.id); setIsEdge(false); }}
                >
                  <div style={{ width: '100%' }}>
                    <div>
                      <Tag color="blue" style={{ marginRight: 4 }}>{item.count}次</Tag>
                      <Text strong style={{ fontSize: 12 }}>{item.name}</Text>
                    </div>
                    <div style={{ marginTop: 2 }}>
                      <Text type="secondary" style={{ fontSize: 11 }}>
                        {item.resource_type || '-'}
                      </Text>
                    </div>
                  </div>
                </List.Item>
              )}
            />
          )}
        </Card>

        <NodeDetailPanel
          selectedId={selectedId}
          nodeType={selectedNode?.type || selectedEdge?.type || ''}
          nodeProperties={selectedNode?.properties || selectedEdge?.properties || {}}
          isEdge={isEdge} allNodes={data?.nodes} allEdges={data?.edges} onClose={() => setSelectedId(null)}
        />
      </div>
    </div>
  );
}
