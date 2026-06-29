import { describe, expect, it } from "vitest";
import { factsToElements, shapeFor, type FactDto } from "./TopologyView";

function fact(overrides: Partial<FactDto>): FactDto {
  return {
    id: "fact-1",
    kind: "resource",
    source: "test",
    resource_id: "cluster:demo",
    resource_type: "Cluster",
    timestamp: 1,
    attributes_json: "{}",
    ...overrides,
  };
}

describe("factsToElements", () => {
  it("builds stable nodes and parent edges from topology facts", () => {
    const elements = factsToElements([
      fact({
        id: "pod-1",
        resource_id: "pod:demo:default:web-0",
        resource_type: "Pod",
        timestamp: 3,
        attributes_json: JSON.stringify({ parent_resource_id: "ns:demo:default" }),
      }),
      fact({
        id: "cluster",
        resource_id: "cluster:demo",
        resource_type: "Cluster",
        timestamp: 1,
        attributes_json: "{}",
      }),
      fact({
        id: "ns",
        resource_id: "ns:demo:default",
        resource_type: "Namespace",
        timestamp: 2,
        attributes_json: JSON.stringify({ parent_resource_id: "cluster:demo" }),
      }),
    ]);

    expect(elements.map((e) => e.data.id)).toEqual([
      "cluster:demo",
      "ns:demo:default",
      "pod:demo:default:web-0",
      "cluster:demo->ns:demo:default",
      "ns:demo:default->pod:demo:default:web-0",
    ]);
    expect(elements[0].data.shape).toBe("hexagon");
    expect(elements[1].data.shape).toBe("round-rectangle");
    expect(elements[2].data.shape).toBe("ellipse");
  });

  it("keeps the newest fact per resource and drops dangling or malformed edges", () => {
    const elements = factsToElements([
      fact({
        id: "old-pod",
        resource_id: "pod:demo:default:web-0",
        resource_type: "Pod",
        timestamp: 1,
        attributes_json: JSON.stringify({ parent_resource_id: "missing-parent" }),
      }),
      fact({
        id: "new-pod",
        resource_id: "pod:demo:default:web-0",
        resource_type: "Pod",
        timestamp: 2,
        attributes_json: JSON.stringify({ parent_resource_id: "pod:demo:default:web-0" }),
      }),
      fact({
        id: "bad-json",
        resource_id: "service:demo:default:web",
        resource_type: "Service",
        timestamp: 1,
        attributes_json: "not-json",
      }),
    ]);

    expect(elements.map((e) => e.data.id)).toEqual([
      "pod:demo:default:web-0",
      "service:demo:default:web",
    ]);
  });
});

describe("shapeFor", () => {
  it("maps known resource types and falls back to ellipse", () => {
    expect(shapeFor("Cluster")).toBe("hexagon");
    expect(shapeFor("Namespace")).toBe("round-rectangle");
    expect(shapeFor("UnknownThing")).toBe("ellipse");
  });
});
