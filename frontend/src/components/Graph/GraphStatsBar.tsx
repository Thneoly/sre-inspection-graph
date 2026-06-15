import { Space, Tag } from 'antd';
import type { GraphSummary } from '../../api/client';

interface GraphStatsBarProps {
  summary: GraphSummary | undefined;
}

export default function GraphStatsBar({ summary }: GraphStatsBarProps) {
  if (!summary) return null;
  return (
    <div style={{ padding: '2px 12px', borderTop: '1px solid #e8e8e8', background: '#fafafa' }}>
      <Space size="middle">
        <span>节点 <strong>{summary.total_nodes}</strong></span>
        <span>边 <strong>{summary.total_edges}</strong></span>
        {summary.risk_counts.high > 0 && (
          <Tag color="error">高危 {summary.risk_counts.high}</Tag>
        )}
        {summary.risk_counts.medium > 0 && (
          <Tag color="warning">中危 {summary.risk_counts.medium}</Tag>
        )}
        <Tag color="success">正常 {summary.risk_counts.low}</Tag>
      </Space>
    </div>
  );
}
