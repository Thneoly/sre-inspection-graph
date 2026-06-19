import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import ExecutionsView from '../components/Recovery/ExecutionsView';
import { makeWrapper } from './helpers/queryWrapper';
import { mockExecutionSucceeded, mockExecutionFailed } from './fixtures/recovery';

vi.mock('../api/client', async () => {
  const actual = await vi.importActual<typeof import('../api/client')>('../api/client');
  return {
    ...actual,
    fetchRecoveryExecutions: vi.fn(),
    postExecutionRollback: vi.fn(),
  };
});

import { fetchRecoveryExecutions, postExecutionRollback } from '../api/client';

describe('ExecutionsView', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders empty state when there are no executions', async () => {
    vi.mocked(fetchRecoveryExecutions).mockResolvedValueOnce({
      data: { executions: [], total: 0 },
    } as never);

    render(<ExecutionsView />, { wrapper: makeWrapper() });

    await waitFor(() =>
      expect(screen.getByText('还没有恢复动作执行记录')).toBeInTheDocument(),
    );
  });

  it('renders one row per execution with action and target columns', async () => {
    vi.mocked(fetchRecoveryExecutions).mockResolvedValueOnce({
      data: {
        executions: [mockExecutionSucceeded, mockExecutionFailed],
        total: 2,
      },
    } as never);

    render(<ExecutionsView />, { wrapper: makeWrapper() });

    await waitFor(() => expect(screen.getByText('扩缩 Deployment')).toBeInTheDocument());
    expect(screen.getByText('重启 Service')).toBeInTheDocument();
    // total tag in header
    expect(screen.getByText('2 条')).toBeInTheDocument();
    // status tags rendered
    expect(screen.getByText('succeeded')).toBeInTheDocument();
    expect(screen.getByText('failed')).toBeInTheDocument();
  });

  it('shows initiated_by and request_reason for the succeeded record', async () => {
    vi.mocked(fetchRecoveryExecutions).mockResolvedValueOnce({
      data: { executions: [mockExecutionSucceeded], total: 1 },
    } as never);

    render(<ExecutionsView />, { wrapper: makeWrapper() });

    await waitFor(() =>
      expect(screen.getByText('alice@example.com')).toBeInTheDocument(),
    );
    expect(screen.getByText('业务低峰期扩容')).toBeInTheDocument();
  });

  it('opens detail drawer when a row is clicked', async () => {
    vi.mocked(fetchRecoveryExecutions).mockResolvedValueOnce({
      data: { executions: [mockExecutionSucceeded], total: 1 },
    } as never);

    const user = userEvent.setup();
    render(<ExecutionsView />, { wrapper: makeWrapper() });

    await waitFor(() => expect(screen.getByText('扩缩 Deployment')).toBeInTheDocument());

    // Click the row by clicking the action_name cell
    await user.click(screen.getByText('扩缩 Deployment'));

    // Drawer reveals additional sections
    await waitFor(() =>
      expect(screen.getByText('执行详情')).toBeInTheDocument(),
    );
    expect(screen.getByText('输入参数')).toBeInTheDocument();
    expect(screen.getByText('执行结果')).toBeInTheDocument();
  });

  it('shows 回滚 button only for succeeded executions with rollback_action_id', async () => {
    // mockExecutionSucceeded: scale_deployment + rollback_action_id="scale_deployment" → 显示回滚
    // mockExecutionFailed: failed + rollback_action_id=null → 不显示
    vi.mocked(fetchRecoveryExecutions).mockResolvedValueOnce({
      data: { executions: [mockExecutionSucceeded, mockExecutionFailed], total: 2 },
    } as never);

    render(<ExecutionsView />, { wrapper: makeWrapper() });

    await waitFor(() => expect(screen.getByText('扩缩 Deployment')).toBeInTheDocument());
    // 只有一行有"回滚"按钮(succeeded + 有 rollback_action_id)
    const rollbackButtons = screen.getAllByRole('button', { name: /回滚/ });
    expect(rollbackButtons).toHaveLength(1);
  });

  it('triggers rollback API after confirmation modal', async () => {
    vi.mocked(fetchRecoveryExecutions).mockResolvedValue({
      data: { executions: [mockExecutionSucceeded], total: 1 },
    } as never);
    vi.mocked(postExecutionRollback).mockResolvedValueOnce({
      data: {
        ...mockExecutionSucceeded,
        execution_id: 'rb-001',
        reverses_execution_id: mockExecutionSucceeded.execution_id,
      },
    } as never);

    const user = userEvent.setup();
    render(<ExecutionsView />, { wrapper: makeWrapper() });

    await waitFor(() => expect(screen.getByText('扩缩 Deployment')).toBeInTheDocument());

    // Click the rollback button → opens Modal.confirm
    await user.click(screen.getByRole('button', { name: /回滚/ }));

    // Confirm modal action — Modal.confirm 渲染一个 OK 按钮(text 为"确认回滚")
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /确认回滚/ })).toBeInTheDocument(),
    );
    await user.click(screen.getByRole('button', { name: /确认回滚/ }));

    await waitFor(() => expect(postExecutionRollback).toHaveBeenCalledTimes(1));
    expect(vi.mocked(postExecutionRollback).mock.calls[0][0]).toMatchObject({
      execution_id: mockExecutionSucceeded.execution_id,
      initiated_by: 'web-ui',
    });
  });
});
