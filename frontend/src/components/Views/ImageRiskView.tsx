import { useState, useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Tag } from 'antd';
import { fetchImageRisk } from '../../api/client';
import GraphCanvas from '../Graph/GraphCanvas';
import GraphStatsBar from '../Graph/GraphStatsBar';
import GraphToolbar from '../Graph/GraphToolbar';
import NodeDetailPanel from '../Graph/NodeDetailPanel';
import LayerToggle from '../Graph/LayerToggle';
import { filterGraphData, defaultLayers, type LayerName } from '../../utils/layers';

export default function ImageRiskView() {
  const [imageId] = useState('image:order-api:1.2.3');
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [isEdge, setIsEdge] = useState(false);
  const [layers, setLayers] = useState<Set<LayerName>>(defaultLayers());

  const { data, isLoading } = useQuery({
    queryKey: ['image-risk', imageId],
    queryFn: () => fetchImageRisk(imageId).then(r => r.data),
    refetchInterval: 3000,
  });

  const filteredData = useMemo(() => filterGraphData(data, layers), [data, layers]);
  const selectedNode = !isEdge ? data?.nodes.find(n => n.id === selectedId) : undefined;
  const selectedEdge = isEdge ? data?.edges.find(e => e.id === selectedId) : undefined;

  return (
    <div style={{ height: 'calc(100vh - 80px)', display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
      <GraphToolbar>
        <span>容器镜像: <Tag color="processing">{imageId}</Tag></span>
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
