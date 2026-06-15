import type { GraphResponse } from '../api/client';

// Relationship types grouped by layer
export const LAYERS = {
  topology: {
    label: '基础拓扑',
    color: '#4A90D9',
    relTypes: [
      'CONTAINS', 'DEPLOYED_AS', 'DEPLOYED_IN', 'BELONGS_TO',
      'USES', 'DEPENDS_ON', 'RUNS', 'SCHEDULED_ON',
      'EXPOSES', 'ROUTES_TO', 'STORED_IN', 'REGISTERS_IN',
    ],
  },
  observability: {
    label: '可观测',
    color: '#F5A623',
    relTypes: [
      'MONITORS', 'VISUALIZES', 'HAS_ALERT_RULE', 'HAS_DASHBOARD',
    ],
  },
  risk: {
    label: '风险巡检',
    color: '#E53935',
    relTypes: [
      'AFFECTS', 'FIRED_ON', 'GENERATED', 'VIOLATES', 'PROPAGATES_TO', 'MEASURES',
    ],
  },
  // Aggregation types from alert view
  alertAggregation: {
    label: '告警归并',
    color: '#FF7043',
    relTypes: ['AGGREGATES_TO'],
  },
};

export type LayerName = keyof typeof LAYERS;

// Filter graph data to only include edges from active layers, and only nodes connected by those edges
export function filterGraphData(
  data: GraphResponse | undefined,
  activeLayers: Set<LayerName>,
): GraphResponse | undefined {
  if (!data) return undefined;

  // Collect all allowed relationship types
  const allowedTypes = new Set<string>();
  for (const layer of activeLayers) {
    for (const t of LAYERS[layer].relTypes) {
      allowedTypes.add(t);
    }
  }

  // If "topology" is always on, just filter edges
  const filteredEdges = data.edges.filter(e => allowedTypes.has(e.type));
  const connectedNodeIds = new Set<string>();
  for (const e of filteredEdges) {
    connectedNodeIds.add(e.source);
    connectedNodeIds.add(e.target);
  }

  return {
    ...data,
    nodes: data.nodes.filter(n => connectedNodeIds.has(n.id)),
    edges: filteredEdges,
  };
}

// Default: topology only
export function defaultLayers(): Set<LayerName> {
  return new Set<LayerName>(['topology']);
}
