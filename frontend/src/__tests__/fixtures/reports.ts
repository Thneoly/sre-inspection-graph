/**
 * Test fixtures for Reports components — PRD-003 Sprint 1.
 *
 * 字段镜像后端 backend/app/reports/store.py:ReportTask.to_dict()。
 */
import type {
  ReportTask,
  ReportTemplate,
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
