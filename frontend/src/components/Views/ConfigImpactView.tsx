import { useState, useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Select, Typography } from 'antd';
import { fetchConfigImpact } from '../../api/client';
import GraphCanvas from '../Graph/GraphCanvas';
import GraphStatsBar from '../Graph/GraphStatsBar';
import GraphToolbar from '../Graph/GraphToolbar';
import NodeDetailPanel from '../Graph/NodeDetailPanel';
import LayerToggle from '../Graph/LayerToggle';
import { filterGraphData, defaultLayers, type LayerName } from '../../utils/layers';

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
