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
  };
});

import { fetchRecoveryExecutions } from '../api/client';

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
});
