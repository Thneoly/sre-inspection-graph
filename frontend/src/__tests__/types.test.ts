import { describe, it, expect } from 'vitest';
import type { GraphNode, GraphEdge, GraphSummary, GraphResponse } from '../api/client';

describe('Graph Type Guards (compile-time verified at runtime)', () => {
  it('GraphNode should have required fields', () => {
    const node: GraphNode = {
      id: 'pod:test',
      label: 'Pod',
      type: 'Pod',
      properties: { pod_ip: '10.0.0.1', restart_count: 3 },
    };
    expect(node.id).toBe('pod:test');
    expect(node.type).toBe('Pod');
    expect(node.properties.restart_count).toBe(3);
  });

  it('GraphEdge should have source and target', () => {
    const edge: GraphEdge = {
      id: 'e1',
      source: 'app:x',
      target: 'comp:y',
      type: 'CONTAINS',
      properties: { dependency_strength: '强' },
    };
    expect(edge.source).toBe('app:x');
    expect(edge.target).toBe('comp:y');
  });

  it('GraphSummary should count risks', () => {
    const summary: GraphSummary = {
      total_nodes: 10,
      total_edges: 15,
      risk_counts: { high: 1, medium: 3, low: 6, unknown: 0 },
      health_counts: { normal: 8, warning: 1, critical: 1, unknown: 0 },
    };
    expect(summary.risk_counts.high).toBe(1);
    expect(summary.risk_counts.medium).toBe(3);
    expect(summary.risk_counts.low).toBe(6);
  });

  it('GraphResponse should contain nodes + edges + summary', () => {
    const response: GraphResponse = {
      nodes: [{ id: 'n1', label: 'Pod', type: 'Pod', properties: {} }],
      edges: [{ id: 'e1', source: 'n1', target: 'n2', type: 'SCHEDULED_ON', properties: {} }],
      summary: {
        total_nodes: 1, total_edges: 1,
        risk_counts: { high: 0, medium: 0, low: 1, unknown: 0 },
        health_counts: { normal: 1, warning: 0, critical: 0, unknown: 0 },
      },
    };
    expect(response.nodes).toHaveLength(1);
    expect(response.edges).toHaveLength(1);
    expect(response.summary.total_nodes).toBe(1);
  });
});
