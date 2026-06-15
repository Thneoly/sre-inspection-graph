import { useState, useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Select, Typography } from 'antd';
import { fetchAlertAggregation } from '../../api/client';
import GraphCanvas from '../Graph/GraphCanvas';
import GraphStatsBar from '../Graph/GraphStatsBar';
import GraphToolbar from '../Graph/GraphToolbar';
import NodeDetailPanel from '../Graph/NodeDetailPanel';
import LayerToggle from '../Graph/LayerToggle';
import { filterGraphData, type LayerName } from '../../utils/layers';

const alertLayers = (): Set<LayerName> => new Set(['topology', 'risk', 'alertAggregation']);

export default function AlertAggregationView() {
  const [severity, setSeverity] = useState<string | undefined>(undefined);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [isEdge, setIsEdge] = useState(false);
  const [layers, setLayers] = useState<Set<LayerName>>(alertLayers());

  const { data, isLoading } = useQuery({
    queryKey: ['alert-aggregation', severity],
    queryFn: () => fetchAlertAggregation(severity).then(r => r.data),
  });

  const filteredData = useMemo(() => filterGraphData(data, layers), [data, layers]);
  const selectedNode = !isEdge ? data?.nodes.find(n => n.id === selectedId) : undefined;
  const selectedEdge = isEdge ? data?.edges.find(e => e.id === selectedId) : undefined;

  return (
    <div style={{ height: 'calc(100vh - 80px)', display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
      <GraphToolbar>
        <Typography.Text>告警级别:</Typography.Text>
        <Select allowClear placeholder="全部" value={severity} onChange={(v) => setSeverity(v || undefined)} style={{ width: 160 }}
          options={[{ value: 'critical', label: 'Critical' }, { value: 'warning', label: 'Warning' }]} />
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
