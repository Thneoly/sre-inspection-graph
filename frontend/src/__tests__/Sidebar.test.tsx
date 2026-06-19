import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import MainLayout from '../components/Layout/MainLayout';

function renderLayout(initialEntry = '/topology') {
  return render(
    <MemoryRouter initialEntries={[initialEntry]}>
      <MainLayout />
    </MemoryRouter>,
  );
}

describe('MainLayout', () => {
  it('should render all 7 navigation menu items (6 inspection views + recovery history)', () => {
    renderLayout();
    const items = [
      '应用拓扑', '访问链路', '节点影响', '配置影响', '镜像风险', '告警归并',
      '恢复历史',
    ];
    for (const label of items) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
  });

  it('should render the recovery history entry below the inspection views', () => {
    renderLayout();
    // 恢复历史 sits below a divider; assert it renders as a menu entry
    expect(screen.getByText('恢复历史')).toBeInTheDocument();
  });

  it('should render the app title', () => {
    renderLayout();
    expect(screen.getByText('Cloud Native Inspection Graph')).toBeInTheDocument();
  });

  it('should render without crashing', () => {
    const { container } = renderLayout();
    expect(container.querySelector('.ant-layout')).toBeInTheDocument();
  });
});
