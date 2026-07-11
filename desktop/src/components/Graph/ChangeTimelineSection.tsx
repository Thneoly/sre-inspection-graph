import { useQuery } from "@tanstack/react-query";
import { Timeline, Typography, Tag, Empty } from "antd";
import { listChangeEvents, type ChangeEvent } from "../../api/client";

const SEV_COLOR: Record<string, string> = { high: "red", medium: "orange", low: "green" };

/**
 * 节点详情里的变更历史(移植自 reference `ChangeTimelineSection.tsx`)。
 * 列 target == resourceId 的最近 50 条变更。
 */
export default function ChangeTimelineSection({ resourceId }: { resourceId: string }) {
  const { data: events } = useQuery({
    queryKey: ["change-events-target", resourceId],
    queryFn: () => listChangeEvents({ targetResourceId: resourceId, limit: 50 }),
    enabled: !!resourceId,
  });
  if (!events || events.length === 0) {
    return <Empty description="无关联变更" image={Empty.PRESENTED_IMAGE_SIMPLE} />;
  }
  return (
    <>
      <Typography.Text strong>变更历史({resourceId})</Typography.Text>
      <Timeline style={{ marginTop: 12 }} items={events.map((e: ChangeEvent) => ({
        color: SEV_COLOR[e.severity_estimate] ?? "green",
        children: (
          <span>
            <code>{e.changed_at}</code> <Tag>{e.change_type}</Tag>
            {" "}{e.description || e.change_event_id}{" "}
            <Tag color={SEV_COLOR[e.severity_estimate] ?? "green"}>{e.severity_estimate}</Tag>
            {e.propagated_to.length > 0 && <Tag>propagated: {e.propagated_to.length}</Tag>}
          </span>
        ),
      }))} />
    </>
  );
}
