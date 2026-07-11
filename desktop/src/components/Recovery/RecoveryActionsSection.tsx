import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { List, Button, Tag, Typography } from "antd";
import { listRecoveryActions, type ActionDef } from "../../api/client";
import DryRunModal from "./DryRunModal";

/**
 * 节点详情里的恢复动作区(移植自 reference `RecoveryActionsSection.tsx`)。
 * 按 target_type 过滤动作,每条「预演」-> DryRunModal。
 */
export default function RecoveryActionsSection({
  resourceId,
  resourceType,
}: {
  resourceId: string;
  resourceType: string;
}) {
  const { data: actions } = useQuery({
    queryKey: ["recovery-actions", resourceType],
    queryFn: () => listRecoveryActions({ targetType: resourceType }),
    enabled: !!resourceType,
  });
  const [selected, setSelected] = useState<ActionDef | null>(null);

  return (
    <>
      <Typography.Text strong>恢复动作(target_type={resourceType || "?"})</Typography.Text>
      <List
        size="small"
        style={{ marginTop: 8 }}
        dataSource={actions ?? []}
        locale={{ emptyText: "无可用动作" }}
        renderItem={(a) => (
          <List.Item actions={[<Button size="small" onClick={() => setSelected(a)}>预演</Button>]}>
            <List.Item.Meta
              title={<span>{a.name} <Tag color={a.risk_level === "high" ? "red" : a.risk_level === "medium" ? "orange" : "green"}>{a.risk_level}</Tag></span>}
              description={<code>{a.action_id}</code>}
            />
          </List.Item>
        )}
      />
      <DryRunModal
        open={!!selected}
        action={selected}
        targetResourceId={resourceId}
        onClose={() => setSelected(null)}
      />
    </>
  );
}
