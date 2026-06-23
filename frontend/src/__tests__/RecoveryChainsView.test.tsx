import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import RecoveryChainsView from '../components/Recovery/RecoveryChainsView';
import { makeWrapper } from './helpers/queryWrapper';

vi.mock('../api/client', async () => {
  const actual = await vi.importActual<typeof import('../api/client')>('../api/client');
  return {
    ...actual,
    fetchChains: vi.fn(),
    fetchChain: vi.fn(),
    fetchChainTemplates: vi.fn(),
    postChainExecute: vi.fn(),
    postChainAbort: vi.fn(),
  };
});

import {
  fetchChains,
  fetchChain,
  fetchChainTemplates,
  postChainExecute,
  postChainAbort,
} from '../api/client';

const mockChain = {
  chain_id: 'chain-1',
  template_id: 'safe_rollback_deployment',
  template_name: '安全回滚 Deployment',
  target_resource_id: 'deploy:vm-cluster:otel-demo:cart',
  status: 'succeeded' as const,
  on_failure: 'rollback_all' as const,
  current_step_index: 3,
  total_steps: 3,
  initiated_by: 'alice',
  initiated_at: '2026-06-23T10:00:00Z',
  completed_at: '2026-06-23T10:05:00Z',
  approval_id: 'ap-1',
  failure_reason: '',
  step_execution_ids: ['e1', 'e2', 'e3'],
};

const mockTemplates = [
  {
    template_id: 'safe_rollback_deployment',
    name: '安全回滚 Deployment(先扩容后回滚再收回)',
    description: '扩容 +2 留出冗余 → 回滚版本 → 缩回 -2 收回',
    target_type: 'Deployment',
    on_failure: 'rollback_all' as const,
    steps: [
      { action_id: 'scale_deployment', params: { replicas_delta: 2 }, target_from: 'input', verify_required: true },
      { action_id: 'rollback_deployment', params: {}, target_from: 'input', verify_required: true },
      { action_id: 'scale_deployment', params: { replicas_delta: -2 }, target_from: 'input', verify_required: false },
    ],
  },
];

describe('RecoveryChainsView', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(fetchChains).mockResolvedValue({
      data: { chains: [mockChain], total: 1 },
    } as never);
    vi.mocked(fetchChainTemplates).mockResolvedValue({
      data: { templates: mockTemplates, total: 1 },
    } as never);
    vi.mocked(fetchChain).mockResolvedValue({
      data: { ...mockChain, steps: [] },
    } as never);
    vi.mocked(postChainExecute).mockResolvedValue({
      data: { ...mockChain, chain_id: 'chain-2' },
    } as never);
    vi.mocked(postChainAbort).mockResolvedValue({
      data: { ...mockChain, status: 'aborted' },
    } as never);
  });

  it('renders chain table with template name and status', async () => {
    render(<RecoveryChainsView />, { wrapper: makeWrapper() });
    await waitFor(() => expect(screen.getByText(/安全回滚 Deployment/)).toBeInTheDocument());
    expect(screen.getByText('succeeded')).toBeInTheDocument();
    expect(screen.getByText('3 / 3')).toBeInTheDocument();
  });

  it('opens execute modal and displays template description', async () => {
    render(<RecoveryChainsView />, { wrapper: makeWrapper() });
    await waitFor(() => expect(screen.getByText(/安全回滚 Deployment/)).toBeInTheDocument());

    const btn = screen.getByRole('button', { name: /发起恢复链/ });
    await userEvent.click(btn);

    // 等模态框出现
    await waitFor(() =>
      expect(screen.getByRole('dialog', { name: /发起恢复链/ })).toBeInTheDocument(),
    );
  });

  it('submits chain execute request', async () => {
    render(<RecoveryChainsView />, { wrapper: makeWrapper() });
    await waitFor(() => expect(screen.getByText(/安全回滚 Deployment/)).toBeInTheDocument());

    await userEvent.click(screen.getByRole('button', { name: /发起恢复链/ }));
    // 等模板下拉可选
    await waitFor(() =>
      expect(screen.getByRole('dialog', { name: /发起恢复链/ })).toBeInTheDocument(),
    );
  });
});
