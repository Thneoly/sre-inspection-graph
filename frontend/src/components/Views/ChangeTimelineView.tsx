import { useState, useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import {
  Card,
  Drawer,
  Empty,
  Input,
  Radio,
  Space,
  Spin,
  Tag,
  Timeline,
  Typography,
  Tree,
  Descriptions,
  Checkbox,
} from 'antd';
import { FieldTimeOutlined } from '@ant-design/icons';
import {
  fetchChangeEventTimeline,
  fetchChangeEventImpact,
} from '../../api/client';
import type {
  ChangeEvent,
  ChangeSeverity,
  ChangeType,
} from '../../api/client';

const { Text, Title } = Typography;

const severityColor: Record<ChangeSeverity, string> = {
  low: 'green',
  medium: 'gold',
  high: 'red',
};

const changeTypeLabel: Record<ChangeType, string> = {
  configmap_updated: 'ConfigMap',
  secret_rotated: 'Secret',
  deployment_rolled: 'Deployment',
  image_pushed: 'Image',
};

const ALL_TYPES: ChangeType[] = [
  'configmap_updated',
  'secret_rotated',
  'deployment_rolled',
  'image_pushed',
];

type RangePreset = '1h' | '6h' | '24h' | '7d';

const RANGE_SECONDS: Record<RangePreset, number> = {
  '1h': 3600,
  '6h': 6 * 3600,
  '24h': 24 * 3600,
  '7d': 7 * 24 * 3600,
};

function isoNow(): string {
  return new Date().toISOString();
}

function isoMinusSeconds(s: number): string {
  return new Date(Date.now() - s * 1000).toISOString();
}

function fullTimestamp(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString();
}

/**
 * 应用级 ChangeEvent 时间线 — PRD-002 Sprint 2。
 *
 * - 顶部:Application 选择器(自由输入,默认 app:order)+ 时间范围 1h/6h/24h/7d
 *        + 类型多选筛选
 * - 主区:antd Timeline,前 200 条事件按时间倒序
 * - 右抽屉:点击事件后展示详情 + 影响树(Tree)
 * - 顶右:by_type 聚合 Tag chips
 */
export default function ChangeTimelineView() {
  const [applicationId, setApplicationId] = useState('app:order');
  const [range, setRange] = useState<RangePreset>('24h');
  const [enabledTypes, setEnabledTypes] = useState<ChangeType[]>([...ALL_TYPES]);
  const [selected, setSelected] = useState<ChangeEvent | null>(null);

  // 时间范围 → since/until ISO
  const since = useMemo(() => isoMinusSeconds(RANGE_SECONDS[range]), [range]);
  const until = useMemo(() => isoNow(), [range]); // 同步 range 重新计算 now
  void until;  // 占位:until 默认就是 now,可不传

  const { data, isLoading, error } = useQuery({
    queryKey: ['change-timeline', applicationId, since],
    queryFn: () =>
      fetchChangeEventTimeline(applicationId, since).then((r) => r.data),
    enabled: !!applicationId,
    refetchInterval: 5000,
  });

  // 应用类型筛选 — 后端不支持 type filter on timeline,前端兜
  const filteredEvents = useMemo(() => {
    const all = data?.events ?? [];
    const allow = new Set(enabledTypes);
    const filtered = all.filter((ev) => allow.has(ev.change_type));
    return filtered.slice(0, 200); // 渲染上限
  }, [data, enabledTypes]);

  return (
    <div style={{ padding: 16, height: '100%', overflow: 'auto' }}>
      <Title level={4} style={{ marginBottom: 16 }}>
        <FieldTimeOutlined /> 变更时间线 — 应用级
      </Title>

      {/* 工具条 */}
      <Card size="small" style={{ marginBottom: 12 }}>
        <Space wrap size="middle">
          <Space>
            <Text>应用 ID</Text>
            <Input
              value={applicationId}
              onChange={(e) => setApplicationId(e.target.value)}
              placeholder="app:order"
              style={{ width: 220 }}
              allowClear
            />
          </Space>
          <Space>
            <Text>时间范围</Text>
            <Radio.Group value={range} onChange={(e) => setRange(e.target.value)} size="small">
              <Radio.Button value="1h">1h</Radio.Button>
              <Radio.Button value="6h">6h</Radio.Button>
              <Radio.Button value="24h">24h</Radio.Button>
              <Radio.Button value="7d">7d</Radio.Button>
            </Radio.Group>
          </Space>
          <Space>
            <Text>类型</Text>
            <Checkbox.Group
              value={enabledTypes}
              onChange={(vals) => setEnabledTypes(vals as ChangeType[])}
              options={ALL_TYPES.map((t) => ({ label: changeTypeLabel[t], value: t }))}
            />
          </Space>
        </Space>
      </Card>

      {/* by_type 聚合 */}
      {data && (
        <Card size="small" style={{ marginBottom: 12 }}>
          <Space wrap>
            <Text type="secondary">本应用范围内总计</Text>
            <Tag color="blue">{data.total} 个事件</Tag>
            <Tag color="default">{data.resources_in_scope} 个资源</Tag>
            {Object.entries(data.by_type).map(([type, count]) => (
              <Tag key={type} color="cyan">
                {changeTypeLabel[type as ChangeType] ?? type}: {count}
              </Tag>
            ))}
          </Space>
        </Card>
      )}

      {/* 主时间线 */}
      <Card title={`时间线(显示 ${filteredEvents.length} / ${data?.events.length ?? 0})`} size="small">
        {isLoading ? (
          <Spin />
        ) : error ? (
          <Text type="danger">查询失败</Text>
        ) : filteredEvents.length === 0 ? (
          <Empty description="所选范围内无变更" />
        ) : (
          <Timeline
            mode="left"
            items={filteredEvents.map((ev) => ({
              color: severityColor[ev.severity_estimate],
              label: <Text type="secondary" style={{ fontSize: 12 }}>{fullTimestamp(ev.changed_at)}</Text>,
              children: (
                <a onClick={() => setSelected(ev)}>
                  <Space size={4}>
                    <Text strong>{changeTypeLabel[ev.change_type] ?? ev.change_type}</Text>
                    <Tag color={severityColor[ev.severity_estimate]}>
                      {ev.severity_estimate}
                    </Tag>
                    {ev.changed_by && <Text type="secondary">{ev.changed_by}</Text>}
                    <Text type="secondary">on {ev.target_resource_id}</Text>
                  </Space>
                  {ev.description && (
                    <div style={{ fontSize: 12, color: '#888' }}>{ev.description}</div>
                  )}
                </a>
              ),
            }))}
          />
        )}
      </Card>

      {/* 详情抽屉 */}
      <Drawer
        open={!!selected}
        onClose={() => setSelected(null)}
        title={selected?.change_type}
        width={520}
      >
        {selected && <DetailPanel event={selected} />}
      </Drawer>
    </div>
  );
}

// ============================================================
// 抽屉内容 — 基本字段 + 影响树
// ============================================================

function DetailPanel({ event }: { event: ChangeEvent }) {
  const { data: impact, isLoading: impactLoading } = useQuery({
    queryKey: ['change-impact', event.change_event_id],
    queryFn: () => fetchChangeEventImpact(event.change_event_id).then((r) => r.data),
  });

  return (
    <Space direction="vertical" style={{ width: '100%' }}>
      <Descriptions column={1} size="small" bordered>
        <Descriptions.Item label="ID">{event.change_event_id}</Descriptions.Item>
        <Descriptions.Item label="时间">{fullTimestamp(event.changed_at)}</Descriptions.Item>
        <Descriptions.Item label="类型">
          {changeTypeLabel[event.change_type] ?? event.change_type}
        </Descriptions.Item>
        <Descriptions.Item label="目标">{event.target_resource_id}</Descriptions.Item>
        <Descriptions.Item label="操作人">{event.changed_by || '-'}</Descriptions.Item>
        <Descriptions.Item label="来源">{event.source}</Descriptions.Item>
        <Descriptions.Item label="风险">
          <Tag color={severityColor[event.severity_estimate]}>{event.severity_estimate}</Tag>
        </Descriptions.Item>
        <Descriptions.Item label="描述">{event.description || '-'}</Descriptions.Item>
        {Object.keys(event.diff_summary).length > 0 && (
          <Descriptions.Item label="变更摘要">
            <pre style={{ margin: 0, fontSize: 11 }}>
              {JSON.stringify(event.diff_summary, null, 2)}
            </pre>
          </Descriptions.Item>
        )}
      </Descriptions>

      <Card size="small" title={`影响范围${impact ? ` (${impact.affected_count})` : ''}`}>
        {impactLoading ? (
          <Spin size="small" />
        ) : !impact || impact.affected.length === 0 ? (
          <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="无下游资源受影响" />
        ) : (
          <Tree
            treeData={impact.affected.map((r) => ({
              key: r.resource_id,
              title: (
                <span>
                  <Tag>{r.resource_type || 'Resource'}</Tag>
                  {r.resource_name || r.resource_id}
                  <Text type="secondary" style={{ marginLeft: 8, fontSize: 11 }}>
                    距离 {r.distance}
                  </Text>
                </span>
              ),
            }))}
            selectable={false}
          />
        )}
      </Card>
    </Space>
  );
}
