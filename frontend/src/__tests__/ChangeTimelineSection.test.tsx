import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import ChangeTimelineSection from '../components/Graph/ChangeTimelineSection';
import { makeWrapper } from './helpers/queryWrapper';
import {
  mockEventLow,
  mockEventMedium,
  mockEventHigh,
  mockEmptyListResponse,
} from './fixtures/changeEvents';

vi.mock('../api/client', async () => {
  const actual = await vi.importActual<typeof import('../api/client')>('../api/client');
  return {
    ...actual,
    fetchChangeEvents: vi.fn(),
  };
});

import { fetchChangeEvents } from '../api/client';

describe('ChangeTimelineSection', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders empty state when resource has no recent changes', async () => {
    vi.mocked(fetchChangeEvents).mockResolvedValueOnce({
      data: mockEmptyListResponse,
    } as never);

    render(<ChangeTimelineSection resourceId="cm:order:nothing" />, {
      wrapper: makeWrapper(),
    });

    await waitFor(() => expect(screen.getByText('近期无变更')).toBeInTheDocument());
  });

  it('renders timeline items with severity tags + change types', async () => {
    vi.mocked(fetchChangeEvents).mockResolvedValueOnce({
      data: { events: [mockEventLow, mockEventMedium, mockEventHigh], total: 3 },
    } as never);

    render(<ChangeTimelineSection resourceId="cm:order:order-config" />, {
      wrapper: makeWrapper(),
    });

    // 三种 severity tag 都得展示
    await waitFor(() => expect(screen.getByText('low')).toBeInTheDocument());
    expect(screen.getByText('medium')).toBeInTheDocument();
    expect(screen.getByText('high')).toBeInTheDocument();

    // 类型标签的中文化(ConfigMap 更新 / Secret 轮换 / Deployment 滚动)
    expect(screen.getByText('ConfigMap 更新')).toBeInTheDocument();
    expect(screen.getByText('Secret 轮换')).toBeInTheDocument();
    expect(screen.getByText('Deployment 滚动')).toBeInTheDocument();

    // 操作人显示
    expect(screen.getByText('by alice@e2e')).toBeInTheDocument();
    expect(screen.getByText('by ci-bot')).toBeInTheDocument();
    expect(screen.getByText('by argo-cd')).toBeInTheDocument();
  });

  it('shows propagated count for events with downstream impact', async () => {
    vi.mocked(fetchChangeEvents).mockResolvedValueOnce({
      data: { events: [mockEventMedium, mockEventHigh], total: 2 },
    } as never);

    render(<ChangeTimelineSection resourceId="secret:order:order-secret" />, {
      wrapper: makeWrapper(),
    });

    await waitFor(() => expect(screen.getByText('影响 5 个下游资源')).toBeInTheDocument());
    expect(screen.getByText('影响 12 个下游资源')).toBeInTheDocument();
  });
});
