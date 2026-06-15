import type { ReactNode } from 'react';
import { Space } from 'antd';

interface GraphToolbarProps {
  children: ReactNode;
}

export default function GraphToolbar({ children }: GraphToolbarProps) {
  return (
    <div style={{ padding: '4px 12px', borderBottom: '1px solid #f0f0f0', background: '#fafafa' }}>
      <Space size="middle" wrap>
        {children}
      </Space>
    </div>
  );
}
