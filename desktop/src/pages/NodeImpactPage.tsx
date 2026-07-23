import InspectionGraphView from "../components/Views/InspectionGraphView";
import { nodeImpact } from "../api/client";

/** Phase 5 - 节点影响(爆炸半径):选 KubernetesNode,看其上的 pod 及依赖者。 */
export default function NodeImpactPage() {
  return (
    <InspectionGraphView
      title="节点影响(爆炸半径)"
      resourceTypes={["Node"]}
      fetch={(id, depth) => nodeImpact(id, depth)}
      defaultDepth={4}
      emptyHint="选一个 Node(vm1/vm2/vm3),经 SCHEDULED_ON 反向查其上的 Pod(及白名单内的依赖链)。真集群需先 Connect + Sync。"
    />
  );
}
