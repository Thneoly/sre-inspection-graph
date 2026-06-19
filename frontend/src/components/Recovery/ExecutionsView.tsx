import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import {
  Table,
  Tag,
  Typography,
  Space,
  Select,
  Card,
  Drawer,
  Descriptions,
  Empty,
} from 'antd';
import type { ColumnsType } from 'antd/es/table';
import {
  fetchRecoveryExecutions,
  type RecoveryExecution,
  type ExecutionStatus,
} from '../../api/client';

const { Text } = Typography;

const statusColor: Record<ExecutionStatus, string> = {
  pending: 'default',
  dry_run_ok: 'cyan',
  awaiting_approval: 'orange',
  approved: 'blue',
  rejected: 'red',
  executing: 'processing',
  succeeded: 'success',
  failed: 'error',
  rolled_back: 'magenta',
};

const STATUS_OPTIONS: { value: ExecutionStatus; label: string }[] = [
  { value: 'succeeded', label: '已成功' },
  { value: 'failed', label: '已失败' },
  { value: 'awaiting_approval', label: '待审批' },
  { value: 'rejected', label: '已驳回' },
  { value: 'executing', label: '执行中' },
  { value: 'rolled_back', label: '已回滚' },
];

export default function ExecutionsView() {
  const [statusFilter, setStatusFilter] = useState<ExecutionStatus | undefined>();
  const [actionFilter, setActionFilter] = useState<string | undefined>();
  const [selectedExecution, setSelectedExecution] = useState<RecoveryExecution | null>(null);

  const { data, isLoading } = useQuery({
    queryKey: ['recovery-executions', statusFilter, actionFilter],
    queryFn: () =>
      fetchRecoveryExecutions({
        status: statusFilter,
        action_id: actionFilter,
        limit: 100,
      }).then((r) => r.data),
    refetchInterval: 5000,    // 5s 自动刷新,捕获新执行
  });

  const executions = data?.executions || [];

  const columns: ColumnsType<RecoveryExecution> = [
    {
      title: '时间',
      dataIndex: 'initiated_at',
      key: 'initiated_at',
      width: 180,
      render: (v: string) => (v ? new Date(v).toLocaleString('zh-CN') : '-'),
    },
    {
      title: '状态',
      dataIndex: 'status',
      key: 'status',
      width: 110,
      render: (s: ExecutionStatus) => <Tag color={statusColor[s]}>{s}</Tag>,
    },
    {
      title: '动作',
      dataIndex: 'action_name',
      key: 'action_name',
      width: 180,
      render: (name: string, row) => (
        <Space size={4} direction="vertical" style={{ gap: 0 }}>
          <Text strong style={{ fontSize: 12 }}>{name}</Text>
          <Text type="secondary" style={{ fontSize: 10 }}>{row.action_id}</Text>
        </Space>
      ),
    },
    {
      title: '目标资源',
      dataIndex: 'target_resource_id',
      key: 'target_resource_id',
      ellipsis: true,
      render: (rid: string, row) => (
        <Space size={4} direction="vertical" style={{ gap: 0 }}>
          <Text style={{ fontSize: 11, fontFamily: 'monospace' }}>{rid}</Text>
          <Tag>{row.target_resource_type}</Tag>
        </Space>
      ),
    },
    {
      title: '发起人',
      dataIndex: 'initiated_by',
      key: 'initiated_by',
      width: 140,
    },
    {
      title: '理由',
      dataIndex: 'request_reason',
      key: 'request_reason',
      ellipsis: true,
      render: (r: string) => r ? <Text style={{ fontSize: 12 }}>{r}</Text> : <Text type="secondary">-</Text>,
    },
  ];

  return (
    <div style={{ padding: 16, height: '100%', overflow: 'auto' }}>
      <Card
        title={
          <Space>
            <Text strong>恢复动作审计历史</Text>
            <Tag>{data?.total ?? 0} 条</Tag>
          </Space>
        }
        extra={
          <Space>
            <Select
              allowClear
              style={{ width: 140 }}
              placeholder="状态过滤"
              options={STATUS_OPTIONS}
              value={statusFilter}
              onChange={setStatusFilter}
              size="small"
            />
            <Select
              allowClear
              style={{ width: 180 }}
              placeholder="动作过滤"
              options={[
                { value: 'scale_deployment', label: 'scale_deployment' },
                { value: 'kill_query', label: 'kill_query' },
                { value: 'restart_service', label: 'restart_service' },
                { value: 'restart_pod', label: 'restart_pod' },
                { value: 'rollback_deployment', label: 'rollback_deployment' },
                { value: 'refresh_secret', label: 'refresh_secret' },
                { value: 'drain_node', label: 'drain_node' },
                { value: 'clear_cache', label: 'clear_cache' },
              ]}
              value={actionFilter}
              onChange={setActionFilter}
              size="small"
            />
          </Space>
        }
      >
        {executions.length === 0 && !isLoading ? (
          <Empty description="还没有恢复动作执行记录" />
        ) : (
          <Table
            rowKey="execution_id"
            columns={columns}
            dataSource={executions}
            loading={isLoading}
            size="small"
            pagination={{ pageSize: 20, showSizeChanger: false }}
            onRow={(row) => ({
              onClick: () => setSelectedExecution(row),
              style: { cursor: 'pointer' },
            })}
          />
        )}
      </Card>

      <Drawer
        title="执行详情"
        open={!!selectedExecution}
        onClose={() => setSelectedExecution(null)}
        width={600}
      >
        {selectedExecution && (
          <>
            <Descriptions column={1} size="small" bordered>
              <Descriptions.Item label="execution_id">
                <Text copyable code style={{ fontSize: 11 }}>
                  {selectedExecution.execution_id}
                </Text>
              </Descriptions.Item>
              <Descriptions.Item label="状态">
                <Tag color={statusColor[selectedExecution.status]}>
                  {selectedExecution.status}
                </Tag>
              </Descriptions.Item>
              <Descriptions.Item label="动作">
                {selectedExecution.action_name} ({selectedExecution.action_id})
              </Descriptions.Item>
              <Descriptions.Item label="目标">
                <Text code style={{ fontSize: 11 }}>{selectedExecution.target_resource_id}</Text>
              </Descriptions.Item>
              <Descriptions.Item label="目标类型">
                <Tag>{selectedExecution.target_resource_type}</Tag>
              </Descriptions.Item>
              <Descriptions.Item label="发起人">{selectedExecution.initiated_by}</Descriptions.Item>
              <Descriptions.Item label="申请理由">
                {selectedExecution.request_reason || '-'}
              </Descriptions.Item>
              <Descriptions.Item label="发起时间">
                {selectedExecution.initiated_at}
              </Descriptions.Item>
              <Descriptions.Item label="完成时间">
                {selectedExecution.completed_at || '-'}
              </Descriptions.Item>
              {selectedExecution.finding_id && (
                <Descriptions.Item label="触发 Finding">
                  <Text code style={{ fontSize: 11 }}>{selectedExecution.finding_id}</Text>
                </Descriptions.Item>
              )}
              {selectedExecution.dry_run_summary && (
                <Descriptions.Item label="Dry-run">
                  影响 {selectedExecution.dry_run_summary.affected_count} 资源 / SLA {selectedExecution.dry_run_summary.estimated_sla_impact}
                </Descriptions.Item>
              )}
            </Descriptions>

            <Card title="输入参数" size="small" style={{ marginTop: 16 }}>
              <pre style={{ fontSize: 11, background: '#f5f5f5', padding: 8, margin: 0, borderRadius: 4 }}>
                {JSON.stringify(selectedExecution.input_params, null, 2)}
              </pre>
            </Card>

            <Card title="执行结果" size="small" style={{ marginTop: 16 }}>
              <pre style={{ fontSize: 11, background: '#f5f5f5', padding: 8, margin: 0, borderRadius: 4 }}>
                {JSON.stringify(selectedExecution.result, null, 2)}
              </pre>
            </Card>
          </>
        )}
      </Drawer>
    </div>
  );
}
