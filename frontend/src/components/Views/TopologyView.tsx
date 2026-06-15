import { useState, useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Select, Slider, Space, Typography } from 'antd';
import { fetchTopology } from '../../api/client';
import GraphCanvas from '../Graph/GraphCanvas';
import GraphStatsBar from '../Graph/GraphStatsBar';
import GraphToolbar from '../Graph/GraphToolbar';
import NodeDetailPanel from '../Graph/NodeDetailPanel';
import LayerToggle from '../Graph/LayerToggle';
import { filterGraphData, defaultLayers, type LayerName } from '../../utils/layers';

export default function TopologyView() {
  const [appCode, setAppCode] = useState('order');
  const [depth, setDepth] = useState(5);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [isEdge, setIsEdge] = useState(false);
  const [layers, setLayers] = useState<Set<LayerName>>(defaultLayers());

  const { data, isLoading } = useQuery({
    queryKey: ['topology', appCode, depth],
    queryFn: () => fetchTopology(appCode, depth).then(r => r.data),
    enabled: !!appCode,
  });

  const filteredData = useMemo(() => filterGraphData(data, layers), [data, layers]);
  const selectedNode = !isEdge ? data?.nodes.find(n => n.id === selectedId) : undefined;
  const selectedEdge = isEdge ? data?.edges.find(e => e.id === selectedId) : undefined;

  const handleNode = (id: string) => { setSelectedId(id); setIsEdge(false); };
  const handleEdge = (id: string) => { setSelectedId(id); setIsEdge(true); };

  return (
    <div style={{ height: 'calc(100vh - 80px)', display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
      <GraphToolbar>
        <Space><Typography.Text>应用:</Typography.Text>
          <Select value={appCode} onChange={setAppCode} style={{ width: 180 }} options={[{ value: 'order', label: '订单应用 (order)' }]} />
        </Space>
        <Space><Typography.Text>深度: {depth}</Typography.Text>
          <Slider min={1} max={10} value={depth} onChange={setDepth} style={{ width: 120 }} />
        </Space>
        <LayerToggle activeLayers={layers} onChange={setLayers} />
      </GraphToolbar>
      <div style={{ display: 'flex', flex: 1, overflow: 'hidden' }}>
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0, minHeight: 0 }}>
          <GraphCanvas data={filteredData} isLoading={isLoading} onNodeSelect={handleNode} onEdgeSelect={handleEdge} selectedNodeId={selectedId} />
          <GraphStatsBar summary={filteredData?.summary} />
        </div>
        <NodeDetailPanel
          selectedId={selectedId}
          nodeType={selectedNode?.type || selectedEdge?.type || ''}
          nodeProperties={selectedNode?.properties || selectedEdge?.properties || {}}
          isEdge={isEdge}
          allNodes={data?.nodes}
          allEdges={data?.edges}
          onClose={() => setSelectedId(null)}
        />
      </div>
    </div>
  );
}
