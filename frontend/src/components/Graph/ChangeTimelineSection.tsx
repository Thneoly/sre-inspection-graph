import { useQuery } from '@tanstack/react-query';
import { Empty, Spin, Tag, Timeline, Typography } from 'antd';
import { fetchChangeEvents } from '../../api/client';
import type { ChangeEvent, ChangeSeverity } from '../../api/client';

const { Text, Paragraph } = Typography;

interface Props {
  resourceId: string;
}

const severityColor: Record<ChangeSeverity, string> = {
  low: 'green',
  medium: 'gold',
  high: 'red',
};

const changeTypeLabel: Record<string, string> = {
  configmap_updated: 'ConfigMap 更新',
  secret_rotated: 'Secret 轮换',
  deployment_rolled: 'Deployment 滚动',
  image_pushed: '镜像推送',
};

/** 把 ISO8601 转成"MM/DD HH:mm"短格式,前端 timeline 显示用 */
function shortTimestamp(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const mm = String(d.getMonth() + 1).padStart(2, '0');
  const dd = String(d.getDate()).padStart(2, '0');
  const hh = String(d.getHours()).padStart(2, '0');
  const mi = String(d.getMinutes()).padStart(2, '0');
  return `${mm}/${dd} ${hh}:${mi}`;
}

/**
 * 给 NodeDetailPanel 用的"变更时间线"段。
 * 拿当前选中资源前 50 条 ChangeEvent,按 severity 颜色渲染 antd Timeline。
 */
export default function ChangeTimelineSection({ resourceId }: Props) {
  const { data, isLoading, error } = useQuery({
    queryKey: ['change-events', resourceId],
    queryFn: () =>
      fetchChangeEvents({ target_resource_id: resourceId, limit: 50 }).then((r) => r.data),
    enabled: !!resourceId,
  });

  if (isLoading) {
    return <Spin size="small" />;
  }
  if (error) {
    return <Text type="danger">变更事件查询失败</Text>;
  }

  const events = data?.events ?? [];
  if (events.length === 0) {
    return <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="近期无变更" />;
  }

  return (
    <Timeline
      mode="left"
      items={events.map((ev: ChangeEvent) => ({
        color: severityColor[ev.severity_estimate] ?? 'blue',
        label: <Text type="secondary" style={{ fontSize: 11 }}>{shortTimestamp(ev.changed_at)}</Text>,
        children: (
          <div style={{ fontSize: 12 }}>
            <Text strong>{changeTypeLabel[ev.change_type] ?? ev.change_type}</Text>
            <Tag color={severityColor[ev.severity_estimate]} style={{ marginLeft: 6 }}>
              {ev.severity_estimate}
            </Tag>
            {ev.changed_by && (
              <Text type="secondary" style={{ marginLeft: 6 }}>by {ev.changed_by}</Text>
            )}
            {ev.description && (
              <Paragraph style={{ margin: '4px 0 2px', fontSize: 12 }} ellipsis={{ rows: 2, expandable: true, symbol: '展开' }}>
                {ev.description}
              </Paragraph>
            )}
            {ev.propagated_count > 0 && (
              <Text type="secondary" style={{ fontSize: 11 }}>
                影响 {ev.propagated_count} 个下游资源
              </Text>
            )}
          </div>
        ),
      }))}
    />
  );
}
