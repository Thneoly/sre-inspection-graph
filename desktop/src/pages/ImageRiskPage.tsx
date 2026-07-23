import InspectionGraphView from "../components/Views/InspectionGraphView";
import { imageRisk } from "../api/client";

/** Phase 5 - 镜像风险:选 ContainerImage,看谁跑这个镜像。
 * 真集群当前 k8s connector 不产 ContainerImage 节点 -> 通常空图(预期)。 */
export default function ImageRiskPage() {
  return (
    <InspectionGraphView
      title="镜像风险"
      resourceTypes={["ContainerImage"]}
      fetch={(id, depth) => imageRisk(id, depth)}
      defaultDepth={4}
      emptyHint="选一个 ContainerImage,经 USES/STORED_IN 反向查跑该镜像的负载。当前 connector 尚未产 ContainerImage 节点,真集群上通常为空。"
    />
  );
}
