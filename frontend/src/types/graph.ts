// Re-export types from api/client for convenience
export type { GraphNode, GraphEdge, GraphSummary, GraphResponse } from '../api/client';

// Cytoscape-compatible element format
export interface CytoNode {
  data: {
    id: string;
    label: string;
    type: string;
    color: string;
    shape: string;
    size: number;
    health_status: string;
    risk_level: string;
    [key: string]: unknown;
  };
}

export interface CytoEdge {
  data: {
    id: string;
    source: string;
    target: string;
    label: string;
    color: string;
    width: number;
    lineStyle: string;
    [key: string]: unknown;
  };
}

// For setting up cytoscape stylesheet
export type CytoscapeStylesheet = Record<string, unknown>[];
