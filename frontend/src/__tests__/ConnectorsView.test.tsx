import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import ConnectorsView from '../components/Views/ConnectorsView';
import { makeWrapper } from './helpers/queryWrapper';

vi.mock('../api/client', async () => {
  const actual = await vi.importActual<typeof import('../api/client')>('../api/client');
  return {
    ...actual,
    fetchConnectors: vi.fn(),
    syncConnectorNow: vi.fn(),
  };
});

import { fetchConnectors, syncConnectorNow } from '../api/client';

const mockConnectors = {
  connectors: [
    {
      name: 'k8s',
      running: true,
      sync_interval_seconds: 30,
      last_sync_at: '2026-06-23T03:00:00Z',
      last_result: {
        nodes_added: 2, nodes_updated: 1, nodes_removed: 0,
        edges_added: 5, edges_updated: 0, edges_removed: 0,
        metrics_added: 0, events_added: 0, duration_ms: 120, notes: ['ok'],
      },
      last_error_message: '',
      error_count_24h: 0,
      sync_count: 10,
    },
    {
      name: 'prometheus',
      running: true,
      sync_interval_seconds: 30,
      last_sync_at: '2026-06-23T03:00:05Z',
      last_result: {
        nodes_added: 0, nodes_updated: 3, nodes_removed: 0,
        edges_added: 0, edges_updated: 0, edges_removed: 0,
        metrics_added: 14, events_added: 2, duration_ms: 80, notes: [],
      },
      last_error_message: 'query timeout',
      error_count_24h: 2,
      sync_count: 8,
    },
    {
      name: 'k8s_watch',
      running: true,
      sync_interval_seconds: 30,
      last_sync_at: null,
      last_result: null,
      last_error_message: '',
      error_count_24h: 0,
      sync_count: 0,
      mode: 'watch',
      cluster_id: 'vm-cluster',
      namespace: 'otel-demo',
      watched_kinds: ['ConfigMap', 'Secret', 'Deployment'],
      snapshot_sizes: { ConfigMap: 5, Secret: 3, Deployment: 14 },
    },
  ],
  total: 3,
};

describe('ConnectorsView', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(fetchConnectors).mockResolvedValue({ data: mockConnectors } as never);
    vi.mocked(syncConnectorNow).mockResolvedValue({
      data: {
        connector: 'k8s',
        result: {
          nodes_added: 1, nodes_updated: 0, nodes_removed: 0,
          edges_added: 0, edges_updated: 0, edges_removed: 0,
          metrics_added: 0, events_added: 0, duration_ms: 50, notes: [],
        },
      },
    } as never);
  });

  it('renders connector table with name labels and status tags', async () => {
    render(<ConnectorsView />, { wrapper: makeWrapper() });
    await waitFor(() => expect(screen.getByText('K8s 拓扑')).toBeInTheDocument());
    expect(screen.getByText('Prometheus 指标')).toBeInTheDocument();
    expect(screen.getByText('K8s Watch(实时)')).toBeInTheDocument();
    // 运行中 tag
    const runningTags = screen.getAllByText('运行中');
    expect(runningTags.length).toBe(3);
  });

  it('shows error count tag when connector has 24h errors', async () => {
    render(<ConnectorsView />, { wrapper: makeWrapper() });
    await waitFor(() => expect(screen.getByText('Prometheus 指标')).toBeInTheDocument());
    // prometheus 2 个错误 → red tag "2"
    const errorTag = screen.getByText('2');
    expect(errorTag).toBeInTheDocument();
  });

  it('disables sync button for watch-mode connector', async () => {
    render(<ConnectorsView />, { wrapper: makeWrapper() });
    await waitFor(() => expect(screen.getByText('K8s Watch(实时)')).toBeInTheDocument());
    // 3 个「立即同步」按钮,顺序 = dataSource:k8s / prometheus / k8s_watch
    const syncButtons = screen.getAllByRole('button', { name: /立即同步/ });
    expect(syncButtons.length).toBe(3);
    // watch 模式(最后一个)disabled
    expect(syncButtons[2]).toBeDisabled();
    // 前两个可点
    expect(syncButtons[0]).not.toBeDisabled();
    expect(syncButtons[1]).not.toBeDisabled();
  });

  it('triggers sync-now on click for non-watch connector', async () => {
    render(<ConnectorsView />, { wrapper: makeWrapper() });
    await waitFor(() => expect(screen.getByText('K8s 拓扑')).toBeInTheDocument());
    const syncButtons = screen.getAllByRole('button', { name: /立即同步/ });
    // 点前两个非 disabled 的(k8s / prometheus),至少触发一次 sync-now
    await userEvent.click(syncButtons[0]);
    await userEvent.click(syncButtons[1]);
    await waitFor(
      () => expect(syncConnectorNow).toHaveBeenCalled(),
      { timeout: 3000 },
    );
    // 被调的参数应是 k8s 或 prometheus(非 watch)
    const calledArgs = syncConnectorNow.mock.calls.map((c) => c[0]);
    expect(calledArgs.some((a) => a === 'k8s' || a === 'prometheus')).toBe(true);
    expect(calledArgs).not.toContain('k8s_watch');
  });
});
