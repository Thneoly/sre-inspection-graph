import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import NodeDetailPanel from '../components/Graph/NodeDetailPanel';
import { makeWrapper } from './helpers/queryWrapper';

// 隔离子组件(它们各自调 API,与本测试无关)
vi.mock('../components/Recovery/RecoveryActionsSection', () => ({
  default: () => <div data-testid="recovery-section" />,
}));
vi.mock('../components/Graph/ChangeTimelineSection', () => ({
  default: () => <div data-testid="change-section" />,
}));

vi.mock('../api/client', async () => {
  const actual = await vi.importActual<typeof import('../api/client')>('../api/client');
  return {
    ...actual,
    fetchResourceMetrics: vi.fn(),
    postReportGenerate: vi.fn(),
  };
});

import { postReportGenerate } from '../api/client';

describe('NodeDetailPanel — 自检报告按钮(PRD-003 Sprint 1)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('shows generate-report card only for Application nodes', () => {
    const { rerender } = render(
      <NodeDetailPanel
        selectedId="app:order"
        nodeType="Application"
        nodeProperties={{ health_status: 'normal' }}
        onClose={() => {}}
      />,
      { wrapper: makeWrapper() },
    );

    expect(screen.getByText('📄 自检报告')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /生成健康报告/ })).toBeInTheDocument();

    // 切到非 Application 节点 —— Card 应消失
    rerender(
      <NodeDetailPanel
        selectedId="pod:order-api-1"
        nodeType="Pod"
        nodeProperties={{ health_status: 'critical' }}
        onClose={() => {}}
      />,
    );
    expect(screen.queryByText('📄 自检报告')).not.toBeInTheDocument();
  });

  it('clicking generate button calls postReportGenerate with application_id', async () => {
    vi.mocked(postReportGenerate).mockResolvedValue({
      data: { report_id: 'rpt-1', status: 'pending', estimated_completion_seconds: 5 },
    } as never);

    render(
      <NodeDetailPanel
        selectedId="app:order"
        nodeType="Application"
        nodeProperties={{ health_status: 'normal' }}
        onClose={() => {}}
      />,
      { wrapper: makeWrapper() },
    );

    await userEvent.click(screen.getByRole('button', { name: /生成健康报告/ }));

    await waitFor(() => expect(postReportGenerate).toHaveBeenCalledTimes(1));
    const arg = vi.mocked(postReportGenerate).mock.calls[0][0];
    expect(arg.template_id).toBe('application_health');
    expect(arg.scope.application_id).toBe('app:order');
    expect(arg.modules).toContain('health_score');
  });

  it('does not render report card when no node selected', () => {
    render(
      <NodeDetailPanel selectedId={null} onClose={() => {}} />,
      { wrapper: makeWrapper() },
    );
    expect(screen.queryByText('📄 自检报告')).not.toBeInTheDocument();
  });
});
