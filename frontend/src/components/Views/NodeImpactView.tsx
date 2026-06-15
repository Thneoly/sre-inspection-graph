import { useState, useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Select, Slider, Space, Typography } from 'antd';
import { fetchNodeImpact } from '../../api/client';
import GraphCanvas from '../Graph/GraphCanvas';
import GraphStatsBar from '../Graph/GraphStatsBar';
import GraphToolbar from '../Graph/GraphToolbar';
import NodeDetailPanel from '../Graph/NodeDetailPanel';
import LayerToggle from '../Graph/LayerToggle';
import { filterGraphData, type LayerName } from '../../utils/layers';

const riskLayers = (): Set<LayerName> => new Set(['topology', 'risk']);

export default function NodeImpactView() {
  const [nodeId, setNodeId] = useState('node:cce-prod-01:worker-01');
  const [depth, setDepth] = useState(4);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [isEdge, setIsEdge] = useState(false);
  const [layers, setLayers] = useState<Set<LayerName>>(riskLayers());

  const { data, isLoading } = useQuery({
    queryKey: ['node-impact', nodeId, depth],
    queryFn: () => fetchNodeImpact(nodeId, depth).then(r => r.data),
    enabled: !!nodeId,
    refetchInterval: 3000,
  });

  const filteredData = useMemo(() => filterGraphData(data, layers), [data, layers]);
  const selectedNode = !isEdge ? data?.nodes.find(n => n.id === selectedId) : undefined;
  const selectedEdge = isEdge ? data?.edges.find(e => e.id === selectedId) : undefined;

  return (
    <div style={{ height: 'calc(100vh - 80px)', display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
      <GraphToolbar>
        <Space>
          <Typography.Text>节点:</Typography.Text>
          <Select value={nodeId} onChange={setNodeId} style={{ width: 320 }} options={[
            { value: 'node:cce-prod-01:worker-01', label: 'worker-01' },
            { value: 'node:cce-prod-01:worker-02', label: 'worker-02' },
            { value: 'node:cce-prod-01:worker-03', label: 'worker-03' },
          ]} />
        </Space>
        <Space>
          <Typography.Text>爆炸半径: {depth}</Typography.Text>
          <Slider min={1} max={10} value={depth} onChange={setDepth} style={{ width: 120 }} />
        </Space>
        <LayerToggle activeLayers={layers} onChange={setLayers} />
      </GraphToolbar>
      <div style={{ display: 'flex', flex: 1, overflow: 'hidden' }}>
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0, minHeight: 0 }}>
          <GraphCanvas data={filteredData} isLoading={isLoading} onNodeSelect={(id) => { setSelectedId(id); setIsEdge(false); }} onEdgeSelect={(id) => { setSelectedId(id); setIsEdge(true); }} selectedNodeId={selectedId} />
          <GraphStatsBar summary={filteredData?.summary} />
        </div>
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
