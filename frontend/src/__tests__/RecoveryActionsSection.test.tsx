import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import RecoveryActionsSection from '../components/Recovery/RecoveryActionsSection';
import { makeWrapper } from './helpers/queryWrapper';
import { mockActionScale, mockActionRollback } from './fixtures/recovery';

// Mock the API client. fetchRecoveryActions is what RecoveryActionsSection calls;
// DryRunModal lives behind a state guard so its mocks only matter when the modal opens.
vi.mock('../api/client', async () => {
  const actual = await vi.importActual<typeof import('../api/client')>('../api/client');
  return {
    ...actual,
    fetchRecoveryActions: vi.fn(),
    postRecoveryDryRun: vi.fn(),
    postRecoveryExecute: vi.fn(),
  };
});

import { fetchRecoveryActions } from '../api/client';

describe('RecoveryActionsSection', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders empty state when no actions are available for the resource type', async () => {
    vi.mocked(fetchRecoveryActions).mockResolvedValueOnce({
      data: { actions: [], total: 0 },
    } as never);

    render(
      <RecoveryActionsSection resourceId="deploy:foo" resourceType="UnknownType" />,
      { wrapper: makeWrapper() },
    );

    await waitFor(() =>
      expect(screen.getByText(/无可用恢复动作/)).toBeInTheDocument(),
    );
    expect(screen.getByText(/UnknownType/)).toBeInTheDocument();
  });

  it('renders one card per action with risk tag and action name', async () => {
    vi.mocked(fetchRecoveryActions).mockResolvedValueOnce({
      data: { actions: [mockActionScale, mockActionRollback], total: 2 },
    } as never);

    render(
      <RecoveryActionsSection
        resourceId="deploy:cce-prod-01:order:order-api"
        resourceType="Deployment"
      />,
      { wrapper: makeWrapper() },
    );

    await waitFor(() => expect(screen.getByText('扩缩 Deployment')).toBeInTheDocument());
    expect(screen.getByText('回滚 Deployment')).toBeInTheDocument();
    expect(screen.getByText('2 个可用')).toBeInTheDocument();
  });

  it('marks high-risk actions that require approval with both tags', async () => {
    vi.mocked(fetchRecoveryActions).mockResolvedValueOnce({
      data: { actions: [mockActionRollback], total: 1 },
    } as never);

    render(
      <RecoveryActionsSection resourceId="deploy:foo" resourceType="Deployment" />,
      { wrapper: makeWrapper() },
    );

    await waitFor(() => expect(screen.getByText('回滚 Deployment')).toBeInTheDocument());
    // risk tag content (low / medium / high) is rendered as text inside <Tag>
    expect(screen.getByText('high')).toBeInTheDocument();
    expect(screen.getByText('审批')).toBeInTheDocument();
  });

  it('opens dry-run modal when 预演 button is clicked', async () => {
    vi.mocked(fetchRecoveryActions).mockResolvedValueOnce({
      data: { actions: [mockActionScale], total: 1 },
    } as never);

    const user = userEvent.setup();
    render(
      <RecoveryActionsSection
        resourceId="deploy:cce-prod-01:order:order-api"
        resourceType="Deployment"
      />,
      { wrapper: makeWrapper() },
    );

    await waitFor(() => expect(screen.getByText('扩缩 Deployment')).toBeInTheDocument());
    await user.click(screen.getByRole('button', { name: /预演/ }));

    // Modal opens — its content includes 「输入参数 (JSON)」 divider label
    await waitFor(() =>
      expect(screen.getByText('输入参数 (JSON)')).toBeInTheDocument(),
    );
  });
});
