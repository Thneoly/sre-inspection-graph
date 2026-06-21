import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import ChangeTimelineView from '../components/Views/ChangeTimelineView';
import { makeWrapper } from './helpers/queryWrapper';
import {
  mockTimelineResponse,
  mockEmptyTimelineResponse,
  mockImpactResponse,
  mockSuggestionDirect,
} from './fixtures/changeEvents';

vi.mock('../api/client', async () => {
  const actual = await vi.importActual<typeof import('../api/client')>('../api/client');
  return {
    ...actual,
    fetchChangeEventTimeline: vi.fn(),
    fetchChangeEventImpact: vi.fn(),
    fetchChangeEventRecoverySuggestion: vi.fn(),
    postRecoveryExecute: vi.fn(),
  };
});

import {
  fetchChangeEventTimeline,
  fetchChangeEventImpact,
  fetchChangeEventRecoverySuggestion,
  postRecoveryExecute,
} from '../api/client';

describe('ChangeTimelineView', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // 默认:抽屉里的恢复建议返回 direct 可执行;各测试可覆盖
    vi.mocked(fetchChangeEventRecoverySuggestion).mockResolvedValue({
      data: mockSuggestionDirect,
    } as never);
    vi.mocked(postRecoveryExecute).mockResolvedValue({
      data: {
        execution_id: 'exec-suggest-1',
        action_id: 'rollback_deployment',
        action_name: '回滚 Deployment',
        target_resource_id: 'deploy:order:order-api',
        target_resource_type: 'Deployment',
        finding_id: null,
        input_params: {},
        status: 'awaiting_approval',
        initiated_by: 'change-timeline',
        request_reason: '',
        initiated_at: '',
        executed_at: '',
        completed_at: '',
        result: {},
        approval_id: 'appr-1',
        rollback_execution_id: null,
        reverses_execution_id: null,
        dry_run_summary: null,
      },
    } as never);
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

  it('shows recovery suggestion card in drawer and triggers execute on click', async () => {
    vi.mocked(fetchChangeEventTimeline).mockResolvedValue({
      data: mockTimelineResponse,
    } as never);
    vi.mocked(fetchChangeEventImpact).mockResolvedValue({
      data: mockImpactResponse,
    } as never);

    render(<ChangeTimelineView />, { wrapper: makeWrapper() });

    // 打开 Secret 事件抽屉
    await waitFor(() => expect(screen.getByText('rotate db password')).toBeInTheDocument());
    await userEvent.click(screen.getByText('rotate db password'));

    // 推荐恢复动作卡片渲染 — 含动作名 + 直接目标 tag + 置信度
    await waitFor(() => expect(screen.getByText('🚀 推荐恢复动作(从此变更直接调起)')).toBeInTheDocument());
    expect(screen.getByText('回滚 Deployment')).toBeInTheDocument();
    expect(screen.getByText('直接目标')).toBeInTheDocument();
    expect(screen.getByText('置信度 90%')).toBeInTheDocument();

    // 点击发起按钮 → 调用 postRecoveryExecute,带 resolved target + reason
    const btn = screen.getByRole('button', { name: /发起/ });
    await userEvent.click(btn);

    await waitFor(() => expect(postRecoveryExecute).toHaveBeenCalledTimes(1));
    const callArg = vi.mocked(postRecoveryExecute).mock.calls[0][0];
    expect(callArg.action_id).toBe('rollback_deployment');
    expect(callArg.target_resource_id).toBe('deploy:order:order-api');
    expect(callArg.initiated_by).toBe('change-timeline');
  });

  it('disables execute button when target is unresolved', async () => {
    vi.mocked(fetchChangeEventTimeline).mockResolvedValue({
      data: mockTimelineResponse,
    } as never);
    vi.mocked(fetchChangeEventRecoverySuggestion).mockResolvedValue({
      data: {
        ...mockSuggestionDirect,
        suggestions: [
          {
            ...mockSuggestionDirect.suggestions[0],
            target_match: 'unresolved',
            resolved_target_resource_id: null,
            resolved_target_type: '',
          },
        ],
      },
    } as never);

    render(<ChangeTimelineView />, { wrapper: makeWrapper() });

    await waitFor(() => expect(screen.getByText('rotate db password')).toBeInTheDocument());
    await userEvent.click(screen.getByText('rotate db password'));

    await waitFor(() => expect(screen.getByText('无可执行目标')).toBeInTheDocument());
    const btn = screen.getByRole('button', { name: /发起/ });
    expect(btn).toBeDisabled();

    await userEvent.click(btn);
    expect(postRecoveryExecute).not.toHaveBeenCalled();
  });
});
