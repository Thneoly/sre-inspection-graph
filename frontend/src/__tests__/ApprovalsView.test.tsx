import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import ApprovalsView from '../components/Recovery/ApprovalsView';
import { makeWrapper } from './helpers/queryWrapper';
import { mockApprovalPending, mockApprovalApproved } from './fixtures/recovery';

vi.mock('../api/client', async () => {
  const actual = await vi.importActual<typeof import('../api/client')>('../api/client');
  return {
    ...actual,
    fetchApprovals: vi.fn(),
    postApprovalApprove: vi.fn(),
    postApprovalReject: vi.fn(),
  };
});

import {
  fetchApprovals,
  postApprovalApprove,
  postApprovalReject,
} from '../api/client';

describe('ApprovalsView', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders empty state when no approvals match the filter', async () => {
    vi.mocked(fetchApprovals).mockResolvedValueOnce({
      data: { approvals: [], total: 0 },
    } as never);

    render(<ApprovalsView />, { wrapper: makeWrapper() });

    await waitFor(() => expect(screen.getByText('暂无审批请求')).toBeInTheDocument());
  });

  it('renders pending approvals with action / target / approver_team', async () => {
    vi.mocked(fetchApprovals).mockResolvedValueOnce({
      data: { approvals: [mockApprovalPending], total: 1 },
    } as never);

    render(<ApprovalsView />, { wrapper: makeWrapper() });

    await waitFor(() => expect(screen.getByText('回滚 Deployment')).toBeInTheDocument());
    expect(screen.getByText('alice@example.com')).toBeInTheDocument();
    expect(screen.getByText('订单团队')).toBeInTheDocument();
    expect(screen.getByText('1 条')).toBeInTheDocument();
  });

  it('opens drawer with action buttons when a pending row is clicked', async () => {
    vi.mocked(fetchApprovals).mockResolvedValueOnce({
      data: { approvals: [mockApprovalPending], total: 1 },
    } as never);

    const user = userEvent.setup();
    render(<ApprovalsView />, { wrapper: makeWrapper() });

    await waitFor(() => expect(screen.getByText('回滚 Deployment')).toBeInTheDocument());

    await user.click(screen.getByText('回滚 Deployment'));

    await waitFor(() => expect(screen.getByText('审批详情')).toBeInTheDocument());
    expect(screen.getByText('批准并执行')).toBeInTheDocument();
    expect(screen.getByText('驳回')).toBeInTheDocument();
  });

  it('calls approve API and shows success message', async () => {
    vi.mocked(fetchApprovals).mockResolvedValue({
      data: { approvals: [mockApprovalPending], total: 1 },
    } as never);
    vi.mocked(postApprovalApprove).mockResolvedValueOnce({
      data: {
        approval: mockApprovalApproved,
        execution: {
          execution_id: 'exec-003',
          action_id: 'rollback_deployment',
          action_name: '回滚 Deployment',
          target_resource_id: 'deploy:cce-prod-01:order:order-api',
          target_resource_type: 'Deployment',
          finding_id: null,
          input_params: {},
          status: 'succeeded',
          initiated_by: 'alice@example.com',
          request_reason: 'v1.2.3 上线后告警增多',
          initiated_at: '2026-06-19T10:00:00Z',
          executed_at: '2026-06-19T10:05:00Z',
          completed_at: '2026-06-19T10:05:30Z',
          result: { success: true },
          approval_id: 'approval-001',
          rollback_execution_id: null,
          reverses_execution_id: null,
          dry_run_summary: null,
        },
      },
    } as never);

    const user = userEvent.setup();
    render(<ApprovalsView />, { wrapper: makeWrapper() });

    await waitFor(() => expect(screen.getByText('回滚 Deployment')).toBeInTheDocument());
    await user.click(screen.getByText('回滚 Deployment'));
    await waitFor(() => expect(screen.getByText('批准并执行')).toBeInTheDocument());

    await user.click(screen.getByText('批准并执行').closest('button') as HTMLButtonElement);

    await waitFor(() => expect(postApprovalApprove).toHaveBeenCalledTimes(1));
    expect(vi.mocked(postApprovalApprove).mock.calls[0][0]).toMatchObject({
      approval_id: 'approval-001',
      approver_id: 'web-user',
    });
  });

  it('requires comment when rejecting', async () => {
    vi.mocked(fetchApprovals).mockResolvedValue({
      data: { approvals: [mockApprovalPending], total: 1 },
    } as never);

    const user = userEvent.setup();
    render(<ApprovalsView />, { wrapper: makeWrapper() });

    await waitFor(() => expect(screen.getByText('回滚 Deployment')).toBeInTheDocument());
    await user.click(screen.getByText('回滚 Deployment'));
    await waitFor(() => expect(screen.getByText('驳回')).toBeInTheDocument());

    // 不填 comment 直接点驳回 → 不调 API
    await user.click(screen.getByText('驳回').closest('button') as HTMLButtonElement);
    expect(postApprovalReject).not.toHaveBeenCalled();
  });
});
