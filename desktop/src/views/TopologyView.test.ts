import { describe, expect, it } from "vitest";
import {
  graphToElements,
  shapeFor,
  type GraphResponse,
} from "./TopologyView";

function graph(overrides: Partial<GraphResponse>): GraphResponse {
  return {
    nodes: [],
    edges: [],
    summary: {
      total_nodes: 0,
      total_edges: 0,
      risk_counts: { high: 0, medium: 0, low: 0, unknown: 0 },
      health_counts: { normal: 0, warning: 0, critical: 0, unknown: 0 },
    },
    ...overrides,
  };
}

describe("graphToElements", () => {
  it("maps GraphResponse nodes + edges to cytoscape elements (nodes first)", () => {
    const elements = graphToElements(
      graph({
        nodes: [
          { id: "cluster:demo", label: "demo", type: "Cluster", properties: {} },
          { id: "ns:demo:default", label: "default", type: "Namespace", properties: {} },
          { id: "pod:demo:default:web-0", label: "web-0", type: "Pod", properties: {} },
        ],
        edges: [
          {
            id: "cluster:demo->ns:demo:default",
            source: "cluster:demo",
            target: "ns:demo:default",
            type: "CONTAINS",
            properties: { derived: true },
          },
          {
            id: "ns:demo:default->pod:demo:default:web-0",
            source: "ns:demo:default",
            target: "pod:demo:default:web-0",
            type: "CONTAINS",
            properties: { derived: true },
          },
        ],
      })
    );

    // nodes 先于 edges,顺序保持后端给的顺序(后端已按 resource_id 排序)
    expect(elements.map((e) => e.data.id)).toEqual([
      "cluster:demo",
      "ns:demo:default",
      "pod:demo:default:web-0",
      "cluster:demo->ns:demo:default",
      "ns:demo:default->pod:demo:default:web-0",
    ]);
    // shape 由 type 决定
    expect(elements[0].data.shape).toBe("hexagon");
    expect(elements[1].data.shape).toBe("round-rectangle");
    expect(elements[2].data.shape).toBe("ellipse");
    // label 为 `type\nlabel`
    expect(elements[0].data.label).toBe("Cluster\ndemo");
  });

  it("carries resourceType and edgeType onto element data", () => {
    const elements = graphToElements(
      graph({
        nodes: [{ id: "svc:demo", label: "web", type: "Service", properties: {} }],
        edges: [
          {
            id: "ns:demo->svc:demo",
            source: "ns:demo",
            target: "svc:demo",
            type: "CONTAINS",
            properties: { derived: true },
          },
        ],
      })
    );
    expect(elements[0].data.resourceType).toBe("Service");
    expect(elements[0].data.shape).toBe("diamond");
    const edge = elements.find((e) => e.data.id === "ns:demo->svc:demo");
    expect(edge?.data.edgeType).toBe("CONTAINS");
  });

  it("returns empty for an empty graph", () => {
    expect(graphToElements(graph({}))).toEqual([]);
  });
});

describe("shapeFor", () => {
  it("maps known resource types and falls back to ellipse", () => {
    expect(shapeFor("Cluster")).toBe("hexagon");
    expect(shapeFor("Namespace")).toBe("round-rectangle");
    expect(shapeFor("UnknownThing")).toBe("ellipse");
  });
});
