/**
 * Test fixtures for Reports components — PRD-003 Sprint 1 + Sprint 2.
 *
 * 字段镜像后端 backend/app/reports/store.py:ReportTask.to_dict()。
 */
import type {
  ReportTask,
  ReportTemplate,
  ReportSubscription,
} from '../../api/client';

export const mockReportPending: ReportTask = {
  report_id: 'rpt-aaa111222333',
  template_id: 'application_health' as ReportTemplate,
  scope: { application_id: 'app:order', time_range_start: '2026-06-13T00:00:00Z' },
  modules: ['health_score', 'seven_views', 'risk_list', 'recommended_actions', 'historical_trends'],
  format: 'markdown',
  status: 'pending',
  progress: 0,
  current_step: '',
  error_message: null,
  has_markdown: false,
  file_path: null,
  created_at: '2026-06-20T03:00:00Z',
  completed_at: '',
};

export const mockReportGenerating: ReportTask = {
  ...mockReportPending,
  report_id: 'rpt-gen111222333',
  status: 'generating',
  progress: 45,
  current_step: '采集 risk_list',
};

export const mockReportCompleted: ReportTask = {
  ...mockReportPending,
  report_id: 'rpt-done11122233',
  status: 'completed',
  progress: 100,
  current_step: '完成',
  has_markdown: true,
  file_path: '/tmp/reports/rpt-done11122233.md',
  completed_at: '2026-06-20T03:00:05Z',
};

export const mockReportFailed: ReportTask = {
  ...mockReportPending,
  report_id: 'rpt-fail11122233',
  status: 'failed',
  error_message: 'RuntimeError: boom',
};

export const mockReportListResponse = {
  reports: [mockReportCompleted, mockReportGenerating, mockReportPending],
  total: 3,
  returned: 3,
};

export const mockGenerateResponse = {
  report_id: 'rpt-new111222333',
  status: 'pending' as const,
  estimated_completion_seconds: 5,
};


// ============================================================
// PRD-003 Sprint 2 — 订阅 fixtures
// ============================================================

export const mockSubscriptionWeekly: ReportSubscription = {
  subscription_id: 'sub-weekly-001',
  template_id: 'application_health',
  scope: { application_id: 'app:order' },
  modules: ['health_score', 'seven_views', 'risk_list', 'recommended_actions', 'historical_trends'],
  cron: '0 9 * * 1',
  recipients: ['sre@example.com'],
  enabled: true,
  created_at: '2026-06-20T03:00:00Z',
  last_run_at: '2026-06-20T09:00:00Z',
  last_status: 'ok',
  last_error: '',
  last_report_id: 'rpt-done11122233',
};

export const mockSubscriptionFailed: ReportSubscription = {
  ...mockSubscriptionWeekly,
  subscription_id: 'sub-failed-002',
  scope: { cluster_id: 'vm-cluster' },
  template_id: 'cluster_overview',
  cron: '0 9 1 * *',
  recipients: ['ops@example.com'],
  last_status: 'failed',
  last_error: 'SMTPException: refused',
};

export const mockSubscriptionDisabled: ReportSubscription = {
  ...mockSubscriptionWeekly,
  subscription_id: 'sub-disabled-003',
  enabled: false,
  last_status: 'never',
  last_run_at: '',
};

export const mockSubscriptionList = {
  subscriptions: [mockSubscriptionWeekly, mockSubscriptionFailed, mockSubscriptionDisabled],
  total: 3,
};
