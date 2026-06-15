import { useEffect, useRef } from 'react';
import cytoscape, { Core } from 'cytoscape';
import dagre from 'cytoscape-dagre';
import type { GraphResponse } from '../../api/client';
import {
  getNodeStyle, getEdgeColor, getEdgeWidth, getEdgeLineStyle,
  getHealthFillColor, getHealthTextColor, getRiskBorder,
} from '../../utils/graphStyles';

cytoscape.use(dagre);

interface GraphCanvasProps {
  data: GraphResponse | undefined;
  isLoading: boolean;
  onNodeSelect: (nodeId: string) => void;
  onEdgeSelect?: (edgeId: string) => void;
  selectedNodeId?: string | null;
}

export default function GraphCanvas({ data, isLoading, onNodeSelect, onEdgeSelect, selectedNodeId }: GraphCanvasProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const cyRef = useRef<Core | null>(null);
  const onNodeSelectRef = useRef(onNodeSelect);
  onNodeSelectRef.current = onNodeSelect;
  const onEdgeSelectRef = useRef(onEdgeSelect);
  onEdgeSelectRef.current = onEdgeSelect;
  const prevNodeIds = useRef<Set<string>>(new Set());
  const prevEdgeIds = useRef<Set<string>>(new Set());

  useEffect(() => {
    if (!containerRef.current) return;
    const cy = cytoscape({
      container: containerRef.current,
      style: [],
      layout: { name: 'dagre', rankDir: 'TB', spacingFactor: 1.2, rankSep: 40, nodeSep: 30 },
      wheelSensitivity: 0.3, minZoom: 0.1, maxZoom: 3,
    });
    cy.on('tap', 'node', (evt) => onNodeSelectRef.current(evt.target.id()));
    cy.on('tap', 'edge', (evt) => onEdgeSelectRef.current?.(evt.target.id()));
    cyRef.current = cy;
    return () => { cy.destroy(); };
  }, []);

  useEffect(() => {
    const cy = cyRef.current;
    if (!cy || !data) return;

    // Check if graph structure changed (new/removed nodes or edges)
    const newNodeIds = new Set(data.nodes.map(n => n.id));
    const newEdgeIds = new Set(data.edges.map(e => e.id));
    const structureChanged =
      prevNodeIds.current.size === 0 ||
      !setsEqual(prevNodeIds.current, newNodeIds) ||
      !setsEqual(prevEdgeIds.current, newEdgeIds);
    prevNodeIds.current = newNodeIds;
    prevEdgeIds.current = newEdgeIds;

    const zoom = cy.zoom();
    const pan = { ...cy.pan() };

    const elements: cytoscape.ElementDefinition[] = [];
    for (const node of data.nodes) {
      const style = getNodeStyle(node.type);
      const health = String(node.properties.health_status || 'normal');
      elements.push({
        data: {
          id: node.id,
          label: (node.properties.name as string) || node.type,
          fillColor: getHealthFillColor(health),
          textColor: getHealthTextColor(health),
          shape: style.shape,
          size: style.size,
          health_status: health,
          risk_level: node.properties.risk_level || 'low',
          ...Object.fromEntries(Object.entries(node.properties).map(([k, v]) => [k, typeof v === 'object' ? JSON.stringify(v) : v])),
        },
      });
    }
    for (const edge of data.edges) {
      const s = String(edge.properties.dependency_strength || '中');
      elements.push({
        data: {
          id: edge.id, source: edge.source, target: edge.target,
          label: edge.type,
          edgeColor: getEdgeColor(s), edgeWidth: getEdgeWidth(s), edgeLine: getEdgeLineStyle(s),
        },
      });
    }

    cy.elements().remove();
    cy.add(elements);

    // Only re-layout when graph structure changes — just update colors on poll refresh
    if (structureChanged) {
      cy.style()
        .selector('node')
        .style({
          'label': 'data(label)',
          'background-color': 'data(fillColor)',
          'shape': 'data(shape)',
          'width': 'data(size)',
          'height': 'data(size)',
          'font-size': '12px',
          'color': 'data(textColor)',
          'text-valign': 'bottom',
          'text-halign': 'center',
          'text-margin-y': 4,
          'border-width': (ele: cytoscape.NodeSingular) =>
            (String(ele.data('risk_level')) === 'high' || String(ele.data('risk_level')) === 'critical') ? 3 : 1.5,
          'border-color': (ele: cytoscape.NodeSingular) => getRiskBorder(String(ele.data('risk_level'))),
          'text-wrap': 'wrap',
          'text-max-width': '140px',
        })
        .selector('node:selected')
        .style({ 'border-width': 3, 'border-color': '#1976D2' })
        .selector('edge')
        .style({
          'width': 'data(edgeWidth)',
          'line-color': 'data(edgeColor)',
          'line-style': 'data(edgeLine)',
          'target-arrow-color': 'data(edgeColor)',
          'target-arrow-shape': 'triangle',
          'curve-style': 'bezier',
          'label': 'data(label)',
          'font-size': '11px',
          'color': '#546E7A',
          'font-weight': '500',
          'text-rotation': 'autorotate',
        });
      cy.layout({ name: 'dagre', rankDir: 'TB', spacingFactor: 1.2, rankSep: 40, nodeSep: 30 }).run();
      cy.fit(undefined, 50);
    } else {
      // Hot-refresh: restore viewport without re-layout
      cy.viewport({ zoom, pan });
    }
  }, [data]);

  // Only re-fit when detail panel toggles (null ↔ id), not on every node click
  const prevHadPanel = useRef(false);
  useEffect(() => {
    const hasPanel = !!selectedNodeId;
    if (hasPanel === prevHadPanel.current) return;
    prevHadPanel.current = hasPanel;
    const cy = cyRef.current;
    if (!cy) return;
    const t = setTimeout(() => { cy.resize(); cy.fit(undefined, 50); }, 50);
    return () => clearTimeout(t);
  }, [selectedNodeId]);

  return (
    <div className="graph-container">
      <div ref={containerRef} className="graph-canvas" />
      {isLoading && <div className="graph-loading">加载中...</div>}
    </div>
  );
}

function setsEqual(a: Set<string>, b: Set<string>): boolean {
  if (a.size !== b.size) return false;
  for (const v of a) if (!b.has(v)) return false;
  return true;
}
