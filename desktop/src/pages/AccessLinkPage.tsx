import InspectionGraphView from "../components/Views/InspectionGraphView";
import { accessLink } from "../api/client";

/** Phase 5 - 访问链:选 Application,无向遍历 ROUTES_TO/EXPOSES/CONTAINS 等访问链。 */
export default function AccessLinkPage() {
  return (
    <InspectionGraphView
      title="访问链"
      resourceTypes={["Application"]}
      fetch={(id, depth) => accessLink(id, depth)}
      defaultDepth={5}
      emptyHint="选一个 Application,无向遍历其访问链(ROUTES_TO / EXPOSES / CONTAINS / BELONGS_TO …)。真集群需先 Connect + Sync。"
    />
  );
}
