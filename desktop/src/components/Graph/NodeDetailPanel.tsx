import { Drawer, Descriptions, Tabs } from "antd";
import type { GraphNodeDto } from "../../views/TopologyView";
import RecoveryActionsSection from "../Recovery/RecoveryActionsSection";
import ChangeTimelineSection from "./ChangeTimelineSection";

interface Props {
  node: GraphNodeDto | null;
  open: boolean;
  onClose: () => void;
}

/**
 * 节点详情 Drawer(移植自 reference `NodeDetailPanel.tsx`)。拓扑点节点弹出,
 * 集成 RecoveryActionsSection(按 target_type 过滤动作)+ ChangeTimelineSection(per-node)。
 */
export default function NodeDetailPanel({ node, open, onClose }: Props) {
  if (!node) {
    return <Drawer open={open} onClose={onClose} width={560} />;
  }
  const props = node.properties ?? {};
  const resourceId = node.id;
  const resourceType = node.type;
  return (
    <Drawer open={open} onClose={onClose} width={560} title={`${resourceType}: ${node.label}`}>
      <Descriptions
        size="small"
        column={1}
        bordered
        title="属性"
        items={Object.entries(props).map(([k, v]) => ({ key: k, label: k, children: String(v) }))}
      />
      <Tabs
        style={{ marginTop: 16 }}
        items={[
          { key: "recovery", label: "恢复动作", children: <RecoveryActionsSection resourceId={resourceId} resourceType={resourceType} /> },
          { key: "changes", label: "变更历史", children: <ChangeTimelineSection resourceId={resourceId} /> },
        ]}
      />
    </Drawer>
  );
}
