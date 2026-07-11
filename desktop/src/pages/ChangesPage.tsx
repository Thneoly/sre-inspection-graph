import ChangeTimelineView from "../components/Views/ChangeTimelineView";
import AlertsView from "../components/Views/AlertsView";

/** Phase 3.6 - Changes 页:变更事件时间线(+ 影响图/告警/恢复建议 drawer)+ 告警列表。 */
export default function ChangesPage() {
  return (
    <div>
      <ChangeTimelineView />
      <AlertsView />
    </div>
  );
}
