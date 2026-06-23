import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
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
  Button,
  Modal,
  message,
} from 'antd';
import { RollbackOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import {
  fetchRecoveryExecutions,
  postExecutionRollback,
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

const verifyColor: Record<string, string> = {
  passed: 'success',
  failed: 'error',
  skipped: 'default',
  not_supported: 'default',
  error: 'red',
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
  const queryClient = useQueryClient();

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

  const rollbackMutation = useMutation({
    mutationFn: postExecutionRollback,
    onSuccess: (resp) => {
      const rb = resp.data;
      if (rb.status === 'succeeded') {
        message.success(`回滚成功 (${rb.execution_id.slice(0, 8)})`);
      } else {
        message.error(
          `回滚失败: ${typeof rb.result?.error === 'string' ? rb.result.error : '未知错误'}`,
        );
      }
      queryClient.invalidateQueries({ queryKey: ['recovery-executions'] });
    },
    onError: (err: { response?: { data?: { detail?: string } }; message: string }) => {
      const detail = err.response?.data?.detail || err.message;
      message.error(`回滚被拒: ${detail}`);
    },
  });

  const handleRollback = (row: RecoveryExecution) => {
    Modal.confirm({
      title: '确认回滚此执行?',
      content: (
        <Space direction="vertical" size={4}>
          <Typography.Text>
            动作:<Typography.Text strong>{row.action_name}</Typography.Text>
          </Typography.Text>
          <Typography.Text>
            目标:<Typography.Text code>{row.target_resource_id}</Typography.Text>
          </Typography.Text>
          <Typography.Text type="warning" style={{ fontSize: 12 }}>
            将创建反向 execution,直接执行(不再二次审批)。
          </Typography.Text>
        </Space>
      ),
      okText: '确认回滚',
      okButtonProps: { danger: true },
      cancelText: '取消',
      onOk: () =>
        rollbackMutation.mutateAsync({
          execution_id: row.execution_id,
          initiated_by: 'web-ui',
          reason: `manual rollback of ${row.execution_id.slice(0, 8)}`,
        }),
    });
  };

  const canRollback = (row: RecoveryExecution): boolean =>
    row.status === 'succeeded' &&
    !row.rollback_execution_id &&
    !row.reverses_execution_id &&    // 不允许回滚一个回滚
    !!row.dry_run_summary?.rollback_action_id;

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
          <Space size={4}>
            <Tag>{row.target_resource_type}</Tag>
            {row.cluster_id && <Tag color="geekblue" style={{ fontSize: 10 }}>{row.cluster_id}</Tag>}
          </Space>
        </Space>
      ),
    },
    {
      title: '验证',
      dataIndex: 'verify_status',
      key: 'verify_status',
      width: 100,
      render: (s: string | undefined) =>
        s ? <Tag color={verifyColor[s] || 'default'}>{s}</Tag> : <Text type="secondary">-</Text>,
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
    {
      title: '操作',
      key: 'actions',
      width: 100,
      render: (_, row) =>
        canRollback(row) ? (
          <Button
            size="small"
            icon={<RollbackOutlined />}
            onClick={(e) => {
              e.stopPropagation();
              handleRollback(row);
            }}
            loading={rollbackMutation.isPending}
          >
            回滚
          </Button>
        ) : (
          <Text type="secondary" style={{ fontSize: 11 }}>-</Text>
        ),
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
              {selectedExecution.cluster_id && (
                <Descriptions.Item label="所属集群">
                  <Tag color="geekblue">{selectedExecution.cluster_id}</Tag>
                </Descriptions.Item>
              )}
              {selectedExecution.verify_status && (
                <Descriptions.Item label="验证状态">
                  <Tag color={verifyColor[selectedExecution.verify_status] || 'default'}>
                    {selectedExecution.verify_status}
                  </Tag>
                  {selectedExecution.verified_at && (
                    <Text type="secondary" style={{ marginLeft: 8, fontSize: 11 }}>
                      {selectedExecution.verified_at}
                    </Text>
                  )}
                </Descriptions.Item>
              )}
              {selectedExecution.chain_id && (
                <Descriptions.Item label="所属链">
                  <Text code style={{ fontSize: 11 }}>
                    {selectedExecution.chain_id.slice(0, 8)} (step {selectedExecution.chain_step_index})
                  </Text>
                </Descriptions.Item>
              )}
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

            {selectedExecution.verify_result && Object.keys(selectedExecution.verify_result).length > 0 && (
              <Card title="验证结果" size="small" style={{ marginTop: 16 }}>
                <pre style={{ fontSize: 11, background: '#f5f5f5', padding: 8, margin: 0, borderRadius: 4 }}>
                  {JSON.stringify(selectedExecution.verify_result, null, 2)}
                </pre>
              </Card>
            )}
          </>
        )}
      </Drawer>
    </div>
  );
}
