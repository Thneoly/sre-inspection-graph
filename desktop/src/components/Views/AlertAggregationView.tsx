import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Card, Select, Slider, Space, Alert, Empty, Tag, Typography } from "antd";
import { TopologyView, type GraphResponse } from "../../views/TopologyView";
import { alertAggregation } from "../../api/client";

/**
 * Phase 5 - alert-aggregation 视图(reference view6)。与其余 4 视图不同:无起点 Select,
 * 展示全部 firing 告警的聚合图 —— 每个告警一个 AlertEvent 节点 + FIRED_ON 边到其
 * resource_ref + resource 邻域。severity 过滤 + depth slider。
 *
 * 告警需先录入(Changes 页「录入告警」)—— 本视图无 live 源(k8s-watch/webhook 延后)。
 */
type Sev = "critical" | "warning";

export default function AlertAggregationView() {
  const [severity, setSeverity] = useState<Sev | undefined>(undefined);
  const [depth, setDepth] = useState<number>(3);

  const { data: graph, isFetching } = useQuery({
    queryKey: ["alert-aggregation", severity, depth],
    queryFn: () => alertAggregation(severity, depth),
  });

  const summary = graph?.summary;
  const alertCount = graph?.nodes.filter((n) => n.type === "AlertEvent").length ?? 0;

  return (
    <Card
      title="告警聚合"
      extra={
        summary ? (
          <Typography.Text type="secondary">
            {alertCount} 告警 · {summary.total_nodes} node · {summary.total_edges} edge
            {isFetching ? " · loading" : ""}
          </Typography.Text>
        ) : undefined
      }
    >
      <Space direction="vertical" style={{ width: "100%" }}>
        <Space wrap align="center">
          <Select
            style={{ width: 160 }}
            placeholder="severity"
            allowClear
            value={severity}
            onChange={(v) => setSeverity(v as Sev | undefined)}
            options={[
              { value: "critical", label: "critical" },
              { value: "warning", label: "warning" },
            ]}
          />
          <div style={{ width: 200 }}>
            <Slider min={1} max={6} value={depth} onChange={setDepth} />
          </div>
          <Tag color="blue">depth = {depth}</Tag>
          <Typography.Text type="secondary">
            firing 告警聚合(AlertEvent 节点 → FIRED_ON → resource 邻域)
          </Typography.Text>
        </Space>

        {graph && graph.nodes.length > 0 ? (
          <TopologyView graph={graph} />
        ) : (
          <Empty description={isFetching ? "加载中…" : "无 firing 告警(先在 Changes 页「录入告警」)"} />
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
