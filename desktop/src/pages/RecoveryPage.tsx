import ExecutionsView from "../components/Recovery/ExecutionsView";
import RecoveryChainsView from "../components/Recovery/RecoveryChainsView";

/** Phase 3.6 - Recovery 页:执行列表(+ 折叠审批)+ 恢复链。 */
export default function RecoveryPage() {
  return (
    <div>
      <ExecutionsView />
      <RecoveryChainsView />
    </div>
  );
}
