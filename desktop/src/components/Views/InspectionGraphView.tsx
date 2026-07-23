import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Card, Select, Slider, Space, Alert, Empty, Tag, Typography } from "antd";
import { TopologyView, type GraphResponse } from "../../views/TopologyView";
import { listResourcesByTypes } from "../../api/client";

/**
 * Phase 5 - 巡检视图通用骨架。4 个图遍历视图(node/config/access/image)共用:
 * 起点 Select(按 resourceTypes 拉候选)+ depth Slider + `<TopologyView graph={sub}/>`。
 * 视图差异(起点类型 / 默认 depth / fetch fn / 空提示)由父 page 注入。
 *
 * 子图来自后端 `engine_identity::views::subgraph` -> `topology_to_graph`,已是
 * GraphResponse,直接喂 TopologyView 渲染 Cytoscape(shape=type/fill=health/border=risk)。
 */
interface Props {
  /** Card 标题。 */
  title: string;
  /** 起点候选节点的 resource_type 列表(选择器数据源)。 */
  resourceTypes: string[];
  /** 子图 fetch(view-specific command wrapper)。 */
  fetch: (resourceId: string, depth: number) => Promise<GraphResponse>;
  /** 默认 / 初始 depth。 */
  defaultDepth: number;
  /** 空图或未选起点时的提示。 */
  emptyHint: string;
}

export default function InspectionGraphView({
  title,
  resourceTypes,
  fetch,
  defaultDepth,
  emptyHint,
}: Props) {
  const [resourceId, setResourceId] = useState<string | undefined>(undefined);
  const [depth, setDepth] = useState<number>(defaultDepth);

  const { data: options } = useQuery({
    queryKey: ["resources-by-type", resourceTypes],
    queryFn: () => listResourcesByTypes(resourceTypes),
  });

  const enabled = !!resourceId;
  const { data: graph, isFetching } = useQuery({
    queryKey: ["inspection-view", title, resourceId, depth],
    queryFn: () => fetch(resourceId!, depth),
    enabled,
  });

  const summary = graph?.summary;

  return (
    <Card
      title={title}
      extra={
        summary ? (
          <Typography.Text type="secondary">
            {summary.total_nodes} node · {summary.total_edges} edge{isFetching ? " · loading" : ""}
          </Typography.Text>
        ) : undefined
      }
    >
      <Space direction="vertical" style={{ width: "100%" }}>
        <Space wrap align="center">
          <Select
            style={{ minWidth: 360 }}
            placeholder={`选择 ${resourceTypes.join(" / ")}`}
            value={resourceId}
            onChange={setResourceId}
            showSearch
            optionFilterProp="label"
            options={(options ?? []).map((o) => ({
              value: o.resource_id,
              label: `${o.label}  (${o.resource_type})`,
            }))}
          />
          <div style={{ width: 200 }}>
            <Slider min={1} max={10} value={depth} onChange={setDepth} />
          </div>
          <Tag color="blue">depth = {depth}</Tag>
          {(options?.length ?? 0) === 0 && (
            <Typography.Text type="warning">
              无 {resourceTypes.join("/")} 节点(先 Sync 拉拓扑)
            </Typography.Text>
          )}
        </Space>

        {!enabled ? (
          <Alert type="info" showIcon message="选择起点节点" description={emptyHint} />
        ) : graph && graph.nodes.length > 0 ? (
          <TopologyView graph={graph} />
        ) : (
          <Empty description={isFetching ? "加载中…" : emptyHint} />
        )}

        {summary && graph && graph.nodes.length > 0 && (
          <Space wrap style={{ marginTop: 8 }}>
            {Object.entries(summary.health_counts).map(([k, v]) => (
              <Tag key={k} color={k === "critical" ? "red" : k === "warning" ? "orange" : "green"}>
                {k}={v}
              </Tag>
            ))}
            {Object.entries(summary.risk_counts).map(([k, v]) => (
              <Tag key={k} color={k === "high" ? "red" : k === "medium" ? "orange" : "green"}>
                {k}={v}
              </Tag>
            ))}
          </Space>
        )}
      </Space>
    </Card>
  );
}
