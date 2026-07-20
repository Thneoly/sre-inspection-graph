import ReportsView from "../components/Reports/ReportsView";
import SubscriptionsView from "../components/Reports/SubscriptionsView";
import SentEmailsView from "../components/Reports/SentEmailsView";

/**
 * Phase 4.3 - 报告页:报告列表/生成/查看 + 订阅管理 + 已发邮件调试。
 *
 * 对齐 reference 报告 UI 全功能:报告(手动生成 + markdown 查看)+ 订阅(cron 调度 +
 * 手动触发 + 启停/删除)+ sent-emails(InMemory 调试)。
 */
export default function ReportsPage() {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "1rem" }}>
      <ReportsView />
      <SubscriptionsView />
      <SentEmailsView />
    </div>
  );
}
