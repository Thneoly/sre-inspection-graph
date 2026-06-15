import axios from 'axios';

const api = axios.create({
  baseURL: import.meta.env.VITE_API_BASE_URL || '/api/v1',
  timeout: 15000,
});

// Graph types
export interface GraphNode {
  id: string;
  label: string;
  type: string;
  properties: Record<string, unknown>;
}

export interface GraphEdge {
  id: string;
  source: string;
  target: string;
  type: string;
  properties: Record<string, unknown>;
}

export interface GraphSummary {
  total_nodes: number;
  total_edges: number;
  risk_counts: Record<string, number>;
  health_counts: Record<string, number>;
}

export interface GraphResponse {
  nodes: GraphNode[];
  edges: GraphEdge[];
  summary: GraphSummary;
}

export interface MetricSnapshotOut {
  id: string;
  metric_name: string;
  current_value: number;
  unit: string;
  fetched_at: string;
  is_stale: boolean;
  warning_breached: boolean;
  critical_breached: boolean;
  warning_threshold: number | null;
  critical_threshold: number | null;
}

export interface ResourceMetricsResponse {
  resource_id: string;
  metrics: MetricSnapshotOut[];
}

// API functions
export function fetchTopology(appCode: string, depth = 5) {
  return api.get<GraphResponse>(`/topology/app/${appCode}`, { params: { depth } });
}

export function fetchAccessLink(appCode: string) {
  return api.get<GraphResponse>(`/access-link/${appCode}`);
}

export function fetchNodeImpact(nodeId: string, depth = 4) {
  return api.get<GraphResponse>(`/node-impact/${encodeURIComponent(nodeId)}`, { params: { depth } });
}

export function fetchConfigImpact(resourceId: string) {
  return api.get<GraphResponse>(`/config-impact/${encodeURIComponent(resourceId)}`);
}

export function fetchImageRisk(imageId: string) {
  return api.get<GraphResponse>(`/image-risk/${encodeURIComponent(imageId)}`);
}

export function fetchAlertAggregation(severity?: string) {
  return api.get<GraphResponse>('/alert-aggregation', { params: { severity } });
}

export function fetchResourceMetrics(resourceId: string) {
  return api.get<ResourceMetricsResponse>(`/metrics/${encodeURIComponent(resourceId)}`);
}

export default api;
