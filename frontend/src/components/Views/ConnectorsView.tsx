import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  Button,
  Card,
  Space,
  Spin,
  Table,
  Tag,
  Typography,
  message,
} from 'antd';
import { SyncOutlined, ApiOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import {
  fetchConnectors,
  syncConnectorNow,
  type ConnectorStatus,
} from '../../api/client';

const { Text, Title } = Typography;

const connectorLabel: Record<string, string> = {
  k8s: 'K8s 拓扑',
  prometheus: 'Prometheus 指标',
  jaeger: 'Jaeger 调用链',
  flagd: 'flagd 特性开关',
  k8s_events: 'K8s 事件',
  k8s_watch: 'K8s Watch(实时)',
};

function label(name: string) {
  return connectorLabel[name] ?? name;
}

export default function ConnectorsView() {
  const qc = useQueryClient();
  const { data, isLoading } = useQuery({
    queryKey: ['connectors-status'],
    queryFn: () => fetchConnectors().then((r) => r.data),
    refetchInterval: 5000,
  });

  const syncMutation = useMutation({
    mutationFn: syncConnectorNow,
    onSuccess: (resp, name) => {
      const r = resp.data.result;
      message.success(
        `${label(name)} 同步完成:${r.nodes_added} 节点 / ${r.metrics_added} 指标 / ${r.events_added} 事件 / ${r.duration_ms}ms`,
      );
      qc.invalidateQueries({ queryKey: ['connectors-status'] });
    },
    onError: (err: { response?: { data?: { detail?: string } }; message: string }) => {
      message.error(`同步失败:${err.response?.data?.detail || err.message}`);
    },
  });

  const columns: ColumnsType<ConnectorStatus> = [
    {
      title: 'Connector',
      dataIndex: 'name',
      render: (name: string, row) => (
        <Space direction="vertical" size={0}>
          <Text strong>
            <ApiOutlined /> {label(name)}
          </Text>
          {row.mode === 'watch' && (
            <Text type="secondary" style={{ fontSize: 11 }}>
              watch 模式 · {row.cluster_id} / {row.namespace}
            </Text>
          )}
        </Space>
      ),
    },
    {
      title: '状态',
      dataIndex: 'running',
      width: 90,
      render: (running: boolean) =>
        running ? <Tag color="success">运行中</Tag> : <Tag color="default">已停止</Tag>,
    },
    {
      title: '最近同步',
      dataIndex: 'last_sync_at',
      width: 170,
      render: (v: string | null) => (v ? <Text code>{v}</Text> : <Text type="secondary">-</Text>),
    },
    {
      title: '同步次数',
      dataIndex: 'sync_count',
      width: 90,
    },
    {
      title: '24h 错误',
      dataIndex: 'error_count_24h',
      width: 100,
      render: (n: number) =>
        n > 0 ? <Tag color="red">{n}</Tag> : <Tag color="green">0</Tag>,
    },
    {
      title: '最近产出',
      width: 220,
      render: (_v, row) => {
        const r = row.last_result;
        if (!r) return <Text type="secondary">-</Text>;
        const parts: string[] = [];
        if (r.nodes_added) parts.push(`+${r.nodes_added}节点`);
        if (r.nodes_updated) parts.push(`~${r.nodes_updated}更新`);
        if (r.metrics_added) parts.push(`+${r.metrics_added}指标`);
        if (r.events_added) parts.push(`+${r.events_added}事件`);
        return (
          <Space direction="vertical" size={0}>
            <Text style={{ fontSize: 11 }}>{parts.join(' / ') || '无变化'}</Text>
            <Text type="secondary" style={{ fontSize: 11 }}>{r.duration_ms}ms</Text>
          </Space>
        );
      },
    },
    {
      title: '最近错误',
      dataIndex: 'last_error_message',
      render: (msg: string) =>
        msg ? (
          <Text type="danger" style={{ fontSize: 11 }} ellipsis>
            {msg}
          </Text>
        ) : (
          <Text type="secondary">-</Text>
        ),
    },
    {
      title: '操作',
      width: 110,
      render: (_v, row) => (
        <Button
          size="small"
          icon={<SyncOutlined />}
          loading={syncMutation.isPending && syncMutation.variables === row.name}
          onClick={() => syncMutation.mutate(row.name)}
          disabled={row.mode === 'watch'}
        >
          立即同步
        </Button>
      ),
    },
  ];

  const connectors = data?.connectors ?? [];

  return (
    <div style={{ padding: 16, height: '100%', overflow: 'auto' }}>
      <Title level={4} style={{ marginBottom: 16 }}>
        <ApiOutlined /> Connector 健康检查
      </Title>
      <Text type="secondary" style={{ display: 'block', marginBottom: 12 }}>
        5 个数据源 connector 的运行状态、最近同步产出与错误计数。watch 模式 connector 走长连接,不支持手动触发(只读状态)。
      </Text>
      <Card size="small">
        {isLoading ? (
          <Spin />
        ) : (
          <Table
            rowKey="name"
            columns={columns}
            dataSource={connectors}
            pagination={false}
            size="small"
            expandable={{
              expandedRowRender: (row) => (
                <Space direction="vertical" size={0} style={{ fontSize: 12 }}>
                  {row.last_result?.notes?.map((n, i) => (
                    <Text key={i} type="secondary">
                      • {n}
                    </Text>
                  )) ?? <Text type="secondary">无 notes</Text>}
                  {row.watched_kinds && (
                    <Text type="secondary">监听类型:{row.watched_kinds.join(', ')}</Text>
                  )}
                  {row.snapshot_sizes && (
                    <Text type="secondary">
                      快照:{Object.entries(row.snapshot_sizes)
                        .map(([k, v]) => `${k}=${v}`)
                        .join(' / ')}
                    </Text>
                  )}
                </Space>
              ),
            }}
          />
        )}
      </Card>
    </div>
  );
}
