import { describe, expect, it } from "vitest";
import {
  graphToElements,
  healthFill,
  riskBorder,
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

describe("healthFill", () => {
  it("maps health to fill colors, unknown/missing -> gray", () => {
    expect(healthFill("normal")).toBe("#3fb950");
    expect(healthFill("warning")).toBe("#d29922");
    expect(healthFill("critical")).toBe("#f85149");
    expect(healthFill("unknown")).toBe("#8b949e");
    expect(healthFill(undefined)).toBe("#8b949e");
    expect(healthFill(null)).toBe("#8b949e");
  });
});

describe("riskBorder", () => {
  it("maps risk to border color + width, unknown/missing -> gray thin", () => {
    expect(riskBorder("high")).toEqual({ color: "#f85149", width: "3px" });
    expect(riskBorder("medium")).toEqual({ color: "#d29922", width: "2px" });
    expect(riskBorder("low")).toEqual({ color: "#238636", width: "1px" });
    expect(riskBorder("unknown")).toEqual({ color: "#8b949e", width: "1px" });
    expect(riskBorder(undefined)).toEqual({ color: "#8b949e", width: "1px" });
  });
});

describe("graphToElements health/risk", () => {
  it("carries health/risk onto element data and derives fill/border", () => {
    const elements = graphToElements(
      graph({
        nodes: [
          {
            id: "pod:a",
            label: "a",
            type: "Pod",
            properties: { health_status: "critical", risk_level: "high" },
          },
          {
            id: "pod:b",
            label: "b",
            type: "Pod",
            properties: { health_status: "normal", risk_level: "low" },
          },
          {
            id: "pod:c",
            label: "c",
            type: "Pod",
            properties: {},
          },
        ],
        edges: [],
      })
    );
    const a = elements[0].data;
    expect(a.health).toBe("critical");
    expect(a.risk).toBe("high");
    expect(a.fill).toBe("#f85149"); // critical -> red
    expect(a.borderColor).toBe("#f85149"); // high -> red
    expect(a.borderWidth).toBe("3px");

    const b = elements[1].data;
    expect(b.fill).toBe("#3fb950"); // normal -> green
    expect(b.borderColor).toBe("#238636"); // low -> green

    // 缺 health/risk -> gray fill + gray thin border
    const c = elements[2].data;
    expect(c.fill).toBe("#8b949e");
    expect(c.borderColor).toBe("#8b949e");
    expect(c.borderWidth).toBe("1px");
  });
});
