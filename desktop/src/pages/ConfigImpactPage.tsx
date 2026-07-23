import InspectionGraphView from "../components/Views/InspectionGraphView";
import { configImpact } from "../api/client";

/** Phase 5 - 配置影响:选 Secret/ConfigMap,看谁 USES 它(pod -> service -> deployment)。 */
export default function ConfigImpactPage() {
  return (
    <InspectionGraphView
      title="配置影响"
      resourceTypes={["Secret", "ConfigMap"]}
      fetch={(id, depth) => configImpact(id, depth)}
      defaultDepth={4}
      emptyHint="选一个 Secret / ConfigMap,经 USES 反向查消费它的 Pod 及上游。真集群需先 Connect + Sync。"
    />
  );
}
