import { useEffect, useRef } from "react";
import cytoscape, { Core, ElementDefinition, StylesheetCSS } from "cytoscape";

/**
 * Phase 2.4 + 2.7 - 拓扑视图(吃 GraphResponse 契约)。
 *
 * Phase 1 时这里自己解 `FactDto[]`(去重 / parent_resource_id 连边 / 悬空过滤)。
 * 2.4 起这套领域逻辑回收到 Rust(`engine_core::facts_to_graph`),前端只把后端
 * 已成图的 `GraphResponse { nodes, edges, summary }` 映射成 Cytoscape element。
 * 契约对齐 `reference/app/models/graph.py::GraphResponse`,为 Phase 2.6+ 多视图
 * 迁移铺路。
 *
 * 设计要点:
 *
 * 1. **节点视觉规则** 对照 reference/frontend/src/utils/graphStyles.ts(CLAUDE.md
 *    「节点视觉规则」):**shape = 资源类型,fill = health,border = risk**。
 *    - shape = type(Cluster=hexagon / Node=octagon / Namespace=round-rectangle
 *      / Pod=ellipse / Service=diamond / Deployment=round-octagon / 其它=ellipse)
 *    - fill = health(normal=green / warning=yellow / critical=red / unknown=gray)
 *    - border = risk(high=thick red / medium=yellow / low=thin green / unknown=gray)
 *    Phase 2.7 起用真值:engine-identity merge 后 health 含 prometheus metric 信号。
 *
 * 2. **edge**:直接用 `GraphResponse.edges`(后端已派生 + 过滤悬空),不再
 *    client 端解 JSON。
 *
 * 3. **layout**:内置 `breadthfirst`(从无入边节点起 BFS 分层),CONTAINS 父子边
 *    形成的层级正好对得上 K8s 拓扑(cluster -> ns -> pod)。
 *
 * 4. **生命周期**:cytoscape Core 一次创建,graph 变化时 `cy.elements()` 全量
 *    替换;卸载时 `cy.destroy()`。
 *
 * Phase 2 后续:布局换 fcose;点击节点抽屉显示 properties。
 */

/** engine_core::Fact 的 serde 镜像 -- 诊断表 / get_topology 仍用。 */
export interface FactDto {
  id: string;
  kind: string;
  source: string;
  resource_id: string;
  resource_type: string;
  timestamp: number;
  attributes_json: string;
}

interface Props {
  graph: GraphResponse;
  /** Phase 3.6 - 点击节点回调(打开 NodeDetailPanel)。 */
  onSelectNode?: (nodeId: string) => void;
}

// resource_type -> cytoscape shape。无 fallback 时给 ellipse(K8s 原生类型外的兜底)
const SHAPE_BY_TYPE: Record<string, string> = {
  Cluster: "hexagon",
  Node: "octagon",
  Namespace: "round-rectangle",
  Pod: "ellipse",
  Container: "barrel",
  ContainerImage: "vee",
  Service: "diamond",
  Deployment: "round-octagon",
  Service_Account: "tag",
  AlertEvent: "triangle",
  Application: "round-triangle",
  ApplicationComponent: "round-diamond",
};

export function shapeFor(resourceType: string): string {
  return SHAPE_BY_TYPE[resourceType] ?? "ellipse";
}

/** health_status -> 节点填充色(对齐 reference graphStyles)。unknown/缺省 -> gray。 */
export function healthFill(health: string | undefined | null): string {
  switch (health) {
    case "normal":
      return "#3fb950"; // green
    case "warning":
      return "#d29922"; // yellow
    case "critical":
      return "#f85149"; // red
    default:
      return "#8b949e"; // gray = unknown / missing
  }
}

/** risk_level -> 节点边框 {color, width}(对齐 reference graphStyles)。unknown/缺省 -> gray thin。 */
export function riskBorder(
  risk: string | undefined | null
): { color: string; width: string } {
  switch (risk) {
    case "high":
      return { color: "#f85149", width: "3px" }; // thick red
    case "medium":
      return { color: "#d29922", width: "2px" }; // medium yellow
    case "low":
      return { color: "#238636", width: "1px" }; // thin green
    default:
      return { color: "#8b949e", width: "1px" }; // gray = unknown / missing
  }
}

/** GraphResponse 节点 -- 对齐 engine_core::GraphNode(JSON key `type`)。 */
export interface GraphNodeDto {
  id: string;
  label: string;
  type: string;
  properties: Record<string, unknown>;
}

/** GraphResponse 边 -- 对齐 engine_core::GraphEdge。 */
export interface GraphEdgeDto {
  id: string;
  source: string;
  target: string;
  type: string;
  properties: Record<string, unknown>;
}

/** GraphResponse summary -- 对齐 engine_core::GraphSummary。 */
export interface GraphSummaryDto {
  total_nodes: number;
  total_edges: number;
  risk_counts: Record<string, number>;
  health_counts: Record<string, number>;
}

/** 三层契约 B 层图响应 -- 对齐 engine_core::GraphResponse。 */
export interface GraphResponse {
  nodes: GraphNodeDto[];
  edges: GraphEdgeDto[];
  summary: GraphSummaryDto;
}

/**
 * 把后端 GraphResponse 映射成 Cytoscape elements。
 *
 * 纯映射 -- 去重 / 连边 / 悬空过滤都已在 Rust(`facts_to_graph`)完成,这里
 * 不做任何图逻辑,只做 node->shape/health/risk + label 拼装。health/risk 走
 * `data(fill)` / `data(borderColor)` / `data(borderWidth)` mapper 上色。
 */
export function graphToElements(graph: GraphResponse): ElementDefinition[] {
  const nodes: ElementDefinition[] = graph.nodes.map((n) => {
    const health = String(n.properties.health_status ?? "");
    const risk = String(n.properties.risk_level ?? "");
    const border = riskBorder(risk);
    return {
      group: "nodes",
      data: {
        id: n.id,
        label: `${n.type}\n${n.label}`,
        resourceType: n.type,
        shape: shapeFor(n.type),
        health,
        risk,
        fill: healthFill(health),
        borderColor: border.color,
        borderWidth: border.width,
      },
    };
  });
  const edges: ElementDefinition[] = graph.edges.map((e) => ({
    group: "edges",
    data: {
      id: e.id,
      source: e.source,
      target: e.target,
      edgeType: e.type,
    },
  }));
  return [...nodes, ...edges];
}

const STYLE: StylesheetCSS[] = [
  {
    selector: "node",
    css: {
      label: "data(label)",
      "text-wrap": "wrap",
      "text-valign": "center",
      "text-halign": "center",
      "font-size": "10px",
      // fill = health / border = risk -- 经 data(...) mapper 从 node data 取色
      "background-color": "data(fill)",
      "border-color": "data(borderColor)",
      "border-width": "data(borderWidth)",
      color: "#fff",
      "text-outline-color": "#1f2328",
      "text-outline-width": "2px",
      width: "70px",
      height: "70px",
    },
  },
  {
    selector: "node[resourceType='Cluster']",
    css: { shape: "hexagon", width: "90px", height: "90px", "font-size": "11px" },
  },
  {
    selector: "node[resourceType='Node']",
    css: { shape: "octagon", width: "78px", height: "78px" },
  },
  {
    selector: "node[resourceType='Namespace']",
    css: { shape: "round-rectangle", width: "100px", height: "55px" },
  },
  {
    selector: "node[resourceType='Pod']",
    css: { shape: "ellipse" },
  },
  {
    selector: "node[resourceType='Service']",
    css: { shape: "diamond", width: "75px", height: "75px" },
  },
  {
    selector: "node[resourceType='Deployment']",
    css: { shape: "round-octagon", width: "80px", height: "80px" },
  },
  {
    selector: "edge",
    css: {
      width: 1.5,
      "line-color": "#8b949e",
      "target-arrow-color": "#8b949e",
      "target-arrow-shape": "triangle",
      "curve-style": "bezier",
      opacity: 0.7,
    },
  },
];

export function TopologyView({ graph, onSelectNode }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const cyRef = useRef<Core | null>(null);
  // 最新回调存 ref,避免 cy.on 闭包捕获 stale prop。
  const onSelectRef = useRef(onSelectNode);
  onSelectRef.current = onSelectNode;

  // 初始化 + 卸载
  useEffect(() => {
    if (!containerRef.current) return;
    const cy = cytoscape({
      container: containerRef.current,
      elements: [],
      style: STYLE,
      layout: { name: "preset" }, // 初始无元素,layout 占位
      wheelSensitivity: 0.2,
    });
    // Phase 3.6 - 点节点 -> 回调(打开 NodeDetailPanel)
    cy.on("tap", "node", (evt) => onSelectRef.current?.(evt.target.id()));
    cyRef.current = cy;
    return () => {
      cy.destroy();
      cyRef.current = null;
    };
  }, []);

  // graph 变化 -> 全量替换 elements + 重跑 layout
  useEffect(() => {
    const cy = cyRef.current;
    if (!cy) return;
    cy.elements().remove();
    const elements = graphToElements(graph);
    if (elements.length === 0) return;
    cy.add(elements);
    cy.layout({
      name: "breadthfirst",
      directed: true,
      spacingFactor: 1.3,
      padding: 24,
      avoidOverlap: true,
    }).run();
    cy.fit(undefined, 30);
  }, [graph]);

  return (
    <div
      ref={containerRef}
      data-testid="topology-view"
      style={{
        width: "100%",
        height: "480px",
        border: "1px solid #d0d7de",
        borderRadius: "6px",
        background: "#fafbfc",
      }}
    />
  );
}
