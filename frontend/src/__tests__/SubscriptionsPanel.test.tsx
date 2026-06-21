import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import SubscriptionsPanel from '../components/Views/SubscriptionsPanel';
import { makeWrapper } from './helpers/queryWrapper';
import {
  mockSubscriptionWeekly,
  mockSubscriptionFailed,
  mockSubscriptionList,
} from './fixtures/reports';

vi.mock('../api/client', async () => {
  const actual = await vi.importActual<typeof import('../api/client')>('../api/client');
  return {
    ...actual,
    fetchSubscriptions: vi.fn(),
    postSubscription: vi.fn(),
    patchSubscription: vi.fn(),
    deleteSubscription: vi.fn(),
    postTriggerSubscription: vi.fn(),
  };
});

import {
  fetchSubscriptions,
  postSubscription,
  deleteSubscription,
  postTriggerSubscription,
} from '../api/client';

describe('SubscriptionsPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders subscription rows with cron + recipients', async () => {
    vi.mocked(fetchSubscriptions).mockResolvedValue({ data: mockSubscriptionList } as never);

    render(<SubscriptionsPanel />, { wrapper: makeWrapper() });

    // 3 行订阅 → cron 与 sre 邮箱可能重复,用 getAllByText
    await waitFor(() => expect(screen.getAllByText('0 9 * * 1').length).toBeGreaterThan(0));
    expect(screen.getByText('0 9 1 * *')).toBeInTheDocument();
    expect(screen.getAllByText('sre@example.com').length).toBeGreaterThan(0);
    expect(screen.getByText('ops@example.com')).toBeInTheDocument();
    expect(screen.getByText('上次成功')).toBeInTheDocument();
  });

  it('shows last_error for failed subscription', async () => {
    vi.mocked(fetchSubscriptions).mockResolvedValue({
      data: { subscriptions: [mockSubscriptionFailed], total: 1 },
    } as never);

    render(<SubscriptionsPanel />, { wrapper: makeWrapper() });

    await waitFor(() => expect(screen.getByText('上次失败')).toBeInTheDocument());
    expect(screen.getByText('SMTPException: refused')).toBeInTheDocument();
  });

  it('shows empty state when no subscriptions', async () => {
    vi.mocked(fetchSubscriptions).mockResolvedValue({
      data: { subscriptions: [], total: 0 },
    } as never);

    render(<SubscriptionsPanel />, { wrapper: makeWrapper() });

    await waitFor(() =>
      expect(screen.getByText('暂无订阅,点击右上角「新建订阅」')).toBeInTheDocument(),
    );
  });

  it('opens create modal and submits form with parsed recipients', async () => {
    vi.mocked(fetchSubscriptions).mockResolvedValue({
      data: { subscriptions: [], total: 0 },
    } as never);
    vi.mocked(postSubscription).mockResolvedValue({ data: mockSubscriptionWeekly } as never);

    render(<SubscriptionsPanel />, { wrapper: makeWrapper() });

    const openBtn = await screen.findByRole('button', { name: /新建订阅/ });
    await userEvent.click(openBtn);

    // 填收件人(application_id 默认无值 — 我们手填)
    const inputs = await screen.findAllByRole('textbox');
    // application_id input(第一个 textbox)
    await userEvent.type(inputs[0], 'app:order');
    // recipients input(末尾那个,逗号分隔)
    const recipientsInput = inputs[inputs.length - 1];
    await userEvent.type(recipientsInput, 'a@x.com, b@x.com');

    // 「创 建」ok 按钮
    const okBtn = await screen.findByRole('button', { name: /^创\s?建$/ });
    await userEvent.click(okBtn);

    await waitFor(() => expect(postSubscription).toHaveBeenCalled());
    const arg = vi.mocked(postSubscription).mock.calls[0][0];
    expect(arg.template_id).toBe('application_health');
    expect(arg.recipients).toEqual(['a@x.com', 'b@x.com']);
    expect(arg.scope.application_id).toBe('app:order');
  });

  it('trigger button calls postTriggerSubscription', async () => {
    vi.mocked(fetchSubscriptions).mockResolvedValue({
      data: { subscriptions: [mockSubscriptionWeekly], total: 1 },
    } as never);
    vi.mocked(postTriggerSubscription).mockResolvedValue({ data: mockSubscriptionWeekly } as never);

    render(<SubscriptionsPanel />, { wrapper: makeWrapper() });

    await waitFor(() => expect(screen.getByText('0 9 * * 1')).toBeInTheDocument());

    const triggerBtn = screen.getByRole('button', { name: /立即运行/ });
    await userEvent.click(triggerBtn);

    await waitFor(() => expect(postTriggerSubscription).toHaveBeenCalled());
    expect(vi.mocked(postTriggerSubscription).mock.calls[0][0]).toBe('sub-weekly-001');
  });

  it('delete button calls deleteSubscription after confirm', async () => {
    vi.mocked(fetchSubscriptions).mockResolvedValue({
      data: { subscriptions: [mockSubscriptionWeekly], total: 1 },
    } as never);
    vi.mocked(deleteSubscription).mockResolvedValue({ data: undefined } as never);

    render(<SubscriptionsPanel />, { wrapper: makeWrapper() });

    await waitFor(() => expect(screen.getAllByText('0 9 * * 1').length).toBeGreaterThan(0));

    // 行操作的「删除」按钮(Popconfirm 触发器)
    const deleteBtns = screen.getAllByRole('button', { name: /删除/ });
    await userEvent.click(deleteBtns[0]);

    // 等 Popconfirm 内容出现(确认问题)
    await waitFor(() =>
      expect(screen.getByText('确定删除此订阅?')).toBeInTheDocument(),
    );

    // antd 2字中文 Button 文本会被插空 → "删 除"。row 触发器是 type=link、有 icon,
    // Popconfirm 的 ok 是 type=primary 仅文本。两个都是 button + 含 /删\s?除/。
    // 取最后一个 = Popconfirm 的 ok。
    const confirmBtns = screen.getAllByRole('button', { name: /^删\s?除$/ });
    await userEvent.click(confirmBtns[confirmBtns.length - 1]);

    await waitFor(() => expect(deleteSubscription).toHaveBeenCalled());
    expect(vi.mocked(deleteSubscription).mock.calls[0][0]).toBe('sub-weekly-001');
  });
});
