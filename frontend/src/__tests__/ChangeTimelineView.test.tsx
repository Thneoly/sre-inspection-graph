import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import ChangeTimelineView from '../components/Views/ChangeTimelineView';
import { makeWrapper } from './helpers/queryWrapper';
import {
  mockTimelineResponse,
  mockEmptyTimelineResponse,
  mockImpactResponse,
} from './fixtures/changeEvents';

vi.mock('../api/client', async () => {
  const actual = await vi.importActual<typeof import('../api/client')>('../api/client');
  return {
    ...actual,
    fetchChangeEventTimeline: vi.fn(),
    fetchChangeEventImpact: vi.fn(),
  };
});

import {
  fetchChangeEventTimeline,
  fetchChangeEventImpact,
} from '../api/client';

describe('ChangeTimelineView', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders empty state when application has no changes in range', async () => {
    vi.mocked(fetchChangeEventTimeline).mockResolvedValue({
      data: mockEmptyTimelineResponse,
    } as never);

    render(<ChangeTimelineView />, { wrapper: makeWrapper() });

    await waitFor(() => expect(screen.getByText('所选范围内无变更')).toBeInTheDocument());
  });

  it('renders timeline events + by_type aggregation tags', async () => {
    vi.mocked(fetchChangeEventTimeline).mockResolvedValue({
      data: mockTimelineResponse,
    } as never);

    render(<ChangeTimelineView />, { wrapper: makeWrapper() });

    // 标题
    await waitFor(() => expect(screen.getByText(/变更时间线/)).toBeInTheDocument());

    // by_type 聚合 — 每种类型一个 Tag
    await waitFor(() => expect(screen.getByText('ConfigMap: 1')).toBeInTheDocument());
    expect(screen.getByText('Secret: 1')).toBeInTheDocument();
    expect(screen.getByText('Deployment: 1')).toBeInTheDocument();

    // 总数 + 资源数
    expect(screen.getByText('3 个事件')).toBeInTheDocument();
    expect(screen.getByText('8 个资源')).toBeInTheDocument();
  });

  it('switches range preset (24h → 7d) — refetches with new since', async () => {
    vi.mocked(fetchChangeEventTimeline).mockResolvedValue({
      data: mockTimelineResponse,
    } as never);

    render(<ChangeTimelineView />, { wrapper: makeWrapper() });

    await waitFor(() => expect(fetchChangeEventTimeline).toHaveBeenCalled());
    const initialCallCount = vi.mocked(fetchChangeEventTimeline).mock.calls.length;

    // 点击 7d 按钮 — antd RadioButton 隐藏 input 有 pointer-events:none,
    // 关掉检测,直接点 label 文本
    const user = userEvent.setup({ pointerEventsCheck: 0 });
    await user.click(screen.getByText('7d'));

    await waitFor(() => {
      expect(vi.mocked(fetchChangeEventTimeline).mock.calls.length).toBeGreaterThan(initialCallCount);
    });
  });

  it('opens detail drawer with impact tree when timeline item is clicked', async () => {
    vi.mocked(fetchChangeEventTimeline).mockResolvedValue({
      data: mockTimelineResponse,
    } as never);
    vi.mocked(fetchChangeEventImpact).mockResolvedValue({
      data: mockImpactResponse,
    } as never);

    render(<ChangeTimelineView />, { wrapper: makeWrapper() });

    // 点 Secret 类型的事件(rotate db password 描述,确保唯一)
    await waitFor(() => expect(screen.getByText('rotate db password')).toBeInTheDocument());
    await userEvent.click(screen.getByText('rotate db password'));

    // 抽屉里显示 ID
    await waitFor(() => expect(screen.getByText('ce-bbb44455566')).toBeInTheDocument());

    // 影响 2 个 Pod
    await waitFor(() => expect(screen.getByText('order-api-1')).toBeInTheDocument());
    expect(screen.getByText('order-api-2')).toBeInTheDocument();
  });

  it('filters events by change_type checkbox', async () => {
    vi.mocked(fetchChangeEventTimeline).mockResolvedValue({
      data: mockTimelineResponse,
    } as never);

    render(<ChangeTimelineView />, { wrapper: makeWrapper() });

    // 默认全选 — 3 个事件
    await waitFor(() => expect(screen.getByText('rotate db password')).toBeInTheDocument());
    expect(screen.getByText('rollout v1.2.4')).toBeInTheDocument();

    // 取消勾选 Deployment
    const deployCheckbox = screen.getByRole('checkbox', { name: 'Deployment' });
    await userEvent.click(deployCheckbox);

    // Deployment 类的事件应消失
    await waitFor(() =>
      expect(screen.queryByText('rollout v1.2.4')).not.toBeInTheDocument(),
    );
    // 其它类型仍在
    expect(screen.getByText('rotate db password')).toBeInTheDocument();
  });
});
