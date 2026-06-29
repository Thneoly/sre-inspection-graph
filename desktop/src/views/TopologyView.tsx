import { useEffect, useRef } from "react";
import cytoscape, { Core, ElementDefinition, StylesheetCSS } from "cytoscape";

/**
 * Phase 1 Step 2 — 最小拓扑视图。
 *
 * 把 `sync_all_now` 返的 FactDto[] 转 Cytoscape `{nodes, edges}`,渲染 DAG。
 * 设计要点:
 *
 * 1. **节点视觉规则** 对照 reference/frontend/src/utils/graphStyles.ts:
 *    - shape = resource_type(Cluster=hexagon / Node=octagon / Namespace=
 *      round-rectangle / Pod=ellipse / Service=diamond / 其它=ellipse)
 *    - fill = health → 本期固定 green(Phase 2 接 metric 后改真值)
 *    - border = risk → 本期固定 thin green
 *
 * 2. **edge 来源**:Fact.attributes_json 解 JSON 找 `parent_resource_id`,
 *    存在则连一条 source=parent → target=self 的有向边。k8s-mini
 *    with_topology=true 的 Fact 全部带此字段(除 Cluster 是根)。
 *
 * 3. **dedup**:Fact 按 resource_id 去重(同一资源多 connector 报多次只画一次)。
 *    去重后取最新 timestamp 的那条作 canonical(stable sort)。
 *
 * 4. **layout**:用内置 `breadthfirst`(从无入边节点起按 BFS 分层),
 *    parent_resource_id 形成的层级正好对得上 K8s 拓扑(cluster → ns → pod)。
 *
 * 5. **生命周期**:cytoscape Core 一次创建,facts 变化时 `cy.elements()`
 *    全量替换(本期 Fact 量在 ~20 以内,全替成本可忽略);组件卸载时
 *    `cy.destroy()`,避免 DOM ref 泄露。
 *
 * Phase 2 改造:接入 health/risk 真值;布局换 fcose;支持点击节点抽屉
 * 显示 attributes_json。
 */

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
  facts: FactDto[];
}

// resource_type → cytoscape shape。无 fallback 时给 ellipse(K8s 原生类型外的兜底)
const SHAPE_BY_TYPE: Record<string, string> = {
  Cluster: "hexagon",
  Node: "octagon",
  Namespace: "round-rectangle",
  Pod: "ellipse",
  Service: "diamond",
  Deployment: "round-octagon",
  Service_Account: "tag",
};

export function shapeFor(resourceType: string): string {
  return SHAPE_BY_TYPE[resourceType] ?? "ellipse";
}

/** 把 Fact 数组转 Cytoscape elements(nodes + edges)。 */
export function factsToElements(facts: FactDto[]): ElementDefinition[] {
  // dedup by resource_id,保留 timestamp 最新的那条
  const byId = new Map<string, FactDto>();
  for (const f of facts) {
    const prev = byId.get(f.resource_id);
    if (!prev || f.timestamp > prev.timestamp) {
      byId.set(f.resource_id, f);
    }
  }

  const nodes: ElementDefinition[] = [];
  const edges: ElementDefinition[] = [];
  const orderedFacts = Array.from(byId.values()).sort((a, b) =>
    a.resource_id.localeCompare(b.resource_id)
  );

  for (const f of orderedFacts) {
    nodes.push({
      group: "nodes",
      data: {
        id: f.resource_id,
        label: shortLabel(f.resource_id, f.resource_type),
        resourceType: f.resource_type,
        shape: shapeFor(f.resource_type),
      },
    });

    // 解 attributes_json 找 parent_resource_id;失败静默忽略(根节点无 parent)
    let parent: string | undefined;
    try {
      const attrs = JSON.parse(f.attributes_json);
      if (typeof attrs.parent_resource_id === "string") {
        parent = attrs.parent_resource_id;
      }
    } catch {
      // attributes_json 不是合法 JSON — 当作没 parent,继续渲染节点
    }

    if (parent && parent !== f.resource_id) {
      edges.push({
        group: "edges",
        data: {
          id: `${parent}->${f.resource_id}`,
          source: parent,
          target: f.resource_id,
        },
      });
    }
  }

  // 过滤掉指向未知节点的 edge(防止悬空边)—— 父节点没在本批 Fact 里出现时跳过
  const nodeIds = new Set(nodes.map((n) => n.data.id as string));
  const validEdges = edges.filter(
    (e) => nodeIds.has(e.data.source as string) && nodeIds.has(e.data.target as string)
  );

  return [...nodes, ...validEdges];
}

/** `cluster:demo` → `demo` / `pod:demo:default:app-0-0` → `app-0-0` 用作节点标签。 */
function shortLabel(resourceId: string, resourceType: string): string {
  const parts = resourceId.split(":");
  const name = parts[parts.length - 1] || resourceId;
  return `${resourceType}\n${name}`;
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
      "background-color": "#3fb950", // green = healthy(本期固定)
      "border-color": "#238636", // thin green border = low risk
      "border-width": "1px",
      color: "#fff",
      "text-outline-color": "#1f6b30",
      "text-outline-width": "1px",
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

export function TopologyView({ facts }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const cyRef = useRef<Core | null>(null);

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
    cyRef.current = cy;
    return () => {
      cy.destroy();
      cyRef.current = null;
    };
  }, []);

  // facts 变化 → 全量替换 elements + 重跑 layout
  useEffect(() => {
    const cy = cyRef.current;
    if (!cy) return;
    cy.elements().remove();
    const elements = factsToElements(facts);
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
  }, [facts]);

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
