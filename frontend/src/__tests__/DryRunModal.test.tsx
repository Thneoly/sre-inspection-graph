import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import DryRunModal from '../components/Recovery/DryRunModal';
import { makeWrapper } from './helpers/queryWrapper';
import {
  mockActionScale,
  mockActionRollback,
  mockDryRunValid,
  mockDryRunInvalid,
  mockExecutionSucceeded,
} from './fixtures/recovery';

vi.mock('../api/client', async () => {
  const actual = await vi.importActual<typeof import('../api/client')>('../api/client');
  return {
    ...actual,
    postRecoveryDryRun: vi.fn(),
    postRecoveryExecute: vi.fn(),
  };
});

import { postRecoveryDryRun, postRecoveryExecute } from '../api/client';

describe('DryRunModal', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders nothing when no action is selected', () => {
    const { container } = render(
      <DryRunModal
        open
        action={null}
        targetResourceId="deploy:foo"
        onClose={() => {}}
      />,
      { wrapper: makeWrapper() },
    );
    // Modal returns null when action is null — nothing in the document body
    expect(container.querySelector('.ant-modal-root')).not.toBeInTheDocument();
  });

  it('runs dry-run on click and shows affected resources for valid target', async () => {
    vi.mocked(postRecoveryDryRun).mockResolvedValueOnce({
      data: mockDryRunValid,
    } as never);

    const user = userEvent.setup();
    render(
      <DryRunModal
        open
        action={mockActionScale}
        targetResourceId="deploy:cce-prod-01:order:order-api"
        onClose={() => {}}
      />,
      { wrapper: makeWrapper() },
    );

    await user.click(screen.getByRole('button', { name: /预演/ }));

    await waitFor(() => expect(postRecoveryDryRun).toHaveBeenCalledTimes(1));
    expect(vi.mocked(postRecoveryDryRun).mock.calls[0][0]).toMatchObject({
      action_id: 'scale_deployment',
      target_resource_id: 'deploy:cce-prod-01:order:order-api',
    });

    // Affected count tag + 2 affected pod names
    await waitFor(() =>
      expect(screen.getByText(/影响 2 个资源/)).toBeInTheDocument(),
    );
    expect(screen.getByText('order-api-1')).toBeInTheDocument();
    expect(screen.getByText('order-api-2')).toBeInTheDocument();
  });

  it('shows validation error when target is invalid', async () => {
    vi.mocked(postRecoveryDryRun).mockResolvedValueOnce({
      data: mockDryRunInvalid,
    } as never);

    const user = userEvent.setup();
    render(
      <DryRunModal
        open
        action={mockActionScale}
        targetResourceId="deploy:not-exist"
        onClose={() => {}}
      />,
      { wrapper: makeWrapper() },
    );

    await user.click(screen.getByRole('button', { name: /预演/ }));

    await waitFor(() =>
      expect(screen.getByText('目标校验失败')).toBeInTheDocument(),
    );
    expect(screen.getByText('目标资源不存在')).toBeInTheDocument();
  });

  it('enables 执行 button only after a successful low_risk dry-run', async () => {
    vi.mocked(postRecoveryDryRun).mockResolvedValueOnce({
      data: mockDryRunValid,
    } as never);

    const user = userEvent.setup();
    render(
      <DryRunModal
        open
        action={mockActionScale}
        targetResourceId="deploy:cce-prod-01:order:order-api"
        onClose={() => {}}
      />,
      { wrapper: makeWrapper() },
    );

    // Helper — antd Button puts the PlayCircleOutlined aria-label in the accessible
    // name, so we anchor on visible text and walk up to the <button>.
    const findExecuteBtn = () =>
      screen.getByText('执行').closest('button') as HTMLButtonElement;

    // Before dry-run: 执行 button is disabled
    expect(findExecuteBtn()).toBeDisabled();

    await user.click(screen.getByRole('button', { name: /预演/ }));
    await waitFor(() => expect(postRecoveryDryRun).toHaveBeenCalled());

    // After dry-run: enabled
    await waitFor(() => expect(findExecuteBtn()).not.toBeDisabled());
  });

  it('keeps 执行 button disabled for high_risk actions even after dry-run', async () => {
    // Build a high_risk dry-run result
    vi.mocked(postRecoveryDryRun).mockResolvedValueOnce({
      data: { ...mockDryRunValid, risk_level: 'high', requires_approval: true },
    } as never);

    const user = userEvent.setup();
    render(
      <DryRunModal
        open
        action={mockActionRollback}
        targetResourceId="deploy:cce-prod-01:order:order-api"
        onClose={() => {}}
      />,
      { wrapper: makeWrapper() },
    );

    await user.click(screen.getByRole('button', { name: /预演/ }));
    await waitFor(() => expect(postRecoveryDryRun).toHaveBeenCalled());

    // High-risk + requires_approval → execute button stays disabled, label says 审批后执行
    const btn = screen.getByRole('button', { name: /审批后执行/ });
    expect(btn).toBeDisabled();
  });

  it('calls execute API and notifies parent via onExecuted on success', async () => {
    vi.mocked(postRecoveryDryRun).mockResolvedValueOnce({
      data: mockDryRunValid,
    } as never);
    vi.mocked(postRecoveryExecute).mockResolvedValueOnce({
      data: mockExecutionSucceeded,
    } as never);

    const onExecuted = vi.fn();
    const onClose = vi.fn();
    const user = userEvent.setup();

    render(
      <DryRunModal
        open
        action={mockActionScale}
        targetResourceId="deploy:cce-prod-01:order:order-api"
        onClose={onClose}
        onExecuted={onExecuted}
      />,
      { wrapper: makeWrapper() },
    );

    await user.click(screen.getByRole('button', { name: /预演/ }));
    await waitFor(() =>
      expect((screen.getByText('执行').closest('button') as HTMLButtonElement)).not.toBeDisabled(),
    );

    await user.click(screen.getByText('执行').closest('button') as HTMLButtonElement);

    await waitFor(() => expect(postRecoveryExecute).toHaveBeenCalledTimes(1));
    expect(vi.mocked(postRecoveryExecute).mock.calls[0][0]).toMatchObject({
      action_id: 'scale_deployment',
      target_resource_id: 'deploy:cce-prod-01:order:order-api',
      initiated_by: 'web-ui',
    });

    await waitFor(() => expect(onExecuted).toHaveBeenCalledWith(mockExecutionSucceeded));
    expect(onClose).toHaveBeenCalled();
  });
});
