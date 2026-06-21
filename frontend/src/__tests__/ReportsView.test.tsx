import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import ReportsView from '../components/Views/ReportsView';
import { makeWrapper } from './helpers/queryWrapper';
import {
  mockReportListResponse,
  mockReportCompleted,
  mockReportGenerating,
  mockGenerateResponse,
} from './fixtures/reports';

vi.mock('../api/client', async () => {
  const actual = await vi.importActual<typeof import('../api/client')>('../api/client');
  return {
    ...actual,
    fetchReports: vi.fn(),
    postReportGenerate: vi.fn(),
    downloadReport: vi.fn(),
  };
});

vi.mock('../utils/download', () => ({
  downloadBlob: vi.fn(),
}));

import { fetchReports, postReportGenerate, downloadReport } from '../api/client';
import { downloadBlob } from '../utils/download';

describe('ReportsView', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders report list with status tags', async () => {
    vi.mocked(fetchReports).mockResolvedValue({ data: mockReportListResponse } as never);

    render(<ReportsView />, { wrapper: makeWrapper() });

    // 等表格行渲染(report_id 截断 16 位显示),而不是等 Card 标题
    await waitFor(() => expect(screen.getByText(/rpt-done/)).toBeInTheDocument());

    // 状态 tag
    expect(screen.getByText('已完成')).toBeInTheDocument();
    expect(screen.getByText('生成中')).toBeInTheDocument();
    expect(screen.getByText('排队中')).toBeInTheDocument();
  });

  it('shows empty state when no reports', async () => {
    vi.mocked(fetchReports).mockResolvedValue({
      data: { reports: [], total: 0, returned: 0 },
    } as never);

    render(<ReportsView />, { wrapper: makeWrapper() });

    await waitFor(() =>
      expect(screen.getByText('暂无报告,点击右上角「生成新报告」')).toBeInTheDocument(),
    );
  });

  it('opens generate modal and submits with form values', async () => {
    vi.mocked(fetchReports).mockResolvedValue({
      data: { reports: [], total: 0, returned: 0 },
    } as never);
    vi.mocked(postReportGenerate).mockResolvedValue({ data: mockGenerateResponse } as never);

    render(<ReportsView />, { wrapper: makeWrapper() });

    // 点 Card 右上角「生成新报告」按钮打开 Modal
    const openBtn = await screen.findByRole('button', { name: /生成新报告/ });
    await userEvent.click(openBtn);

    // Modal 的「生成」ok 按钮。antd 对两个中文字符按钮会自动插入空格 → accessible name "生 成",
    // 用 /^生\s?成$/ 精确匹配,避免误匹配 Card 上的"生成新报告"。
    const okBtn = await screen.findByRole('button', { name: /^生\s?成$/ });
    await userEvent.click(okBtn);

    await waitFor(() => expect(postReportGenerate).toHaveBeenCalled());
    const arg = vi.mocked(postReportGenerate).mock.calls[0][0];
    expect(arg.template_id).toBe('application_health');
    expect(arg.scope.application_id).toBe('app:order');
    expect(arg.format).toBe('markdown');
    expect(arg.modules).toContain('health_score');
  });

  it('download button calls downloadReport + downloadBlob on completed row', async () => {
    vi.mocked(fetchReports).mockResolvedValue({
      data: { reports: [mockReportCompleted], total: 1, returned: 1 },
    } as never);
    const blob = new Blob(['# md'], { type: 'text/markdown' });
    vi.mocked(downloadReport).mockResolvedValue({ data: blob } as never);

    render(<ReportsView />, { wrapper: makeWrapper() });

    // 等行渲染
    await waitFor(() => expect(screen.getByText(/rpt-done/)).toBeInTheDocument());

    const downloadBtn = screen.getByRole('button', { name: /下载/ });
    await userEvent.click(downloadBtn);

    // React Query 的 mutationFn 调用会带额外 context 参数,只校验首参(report_id)
    await waitFor(() => expect(downloadReport).toHaveBeenCalled());
    expect(vi.mocked(downloadReport).mock.calls[0][0]).toBe('rpt-done11122233');
    await waitFor(() => expect(downloadBlob).toHaveBeenCalledWith(blob, 'rpt-done11122233.md'));
  });

  it('download button disabled on generating row', async () => {
    vi.mocked(fetchReports).mockResolvedValue({
      data: { reports: [mockReportGenerating], total: 1, returned: 1 },
    } as never);

    render(<ReportsView />, { wrapper: makeWrapper() });

    await waitFor(() => expect(screen.getByText('生成中')).toBeInTheDocument());
    const downloadBtn = screen.getByRole('button', { name: /下载/ });
    expect(downloadBtn).toBeDisabled();
  });
});
