import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  Table,
  Tag,
  Typography,
  Space,
  Card,
  Drawer,
  Descriptions,
  Empty,
  Button,
  Input,
  message,
  Select,
} from 'antd';
import { CheckOutlined, CloseOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import {
  fetchApprovals,
  postApprovalApprove,
  postApprovalReject,
  type ApprovalRequest,
  type ApprovalStatus,
} from '../../api/client';

const { Text } = Typography;

const statusColor: Record<ApprovalStatus, string> = {
  pending: 'orange',
  approved: 'success',
  rejected: 'error',
  expired: 'default',
};

const STATUS_OPTIONS: { value: ApprovalStatus; label: string }[] = [
  { value: 'pending', label: '待审批' },
  { value: 'approved', label: '已批准' },
  { value: 'rejected', label: '已驳回' },
  { value: 'expired', label: '已过期' },
];

function formatRemaining(expiryAt: string): string {
  if (!expiryAt) return '-';
  const expiry = new Date(expiryAt).getTime();
  const now = Date.now();
  if (Number.isNaN(expiry)) return '-';
  const diffMs = expiry - now;
  if (diffMs <= 0) return '已过期';
  const hours = Math.floor(diffMs / 3_600_000);
  const minutes = Math.floor((diffMs % 3_600_000) / 60_000);
  if (hours >= 24) return `${Math.floor(hours / 24)}d ${hours % 24}h`;
  return `${hours}h ${minutes}m`;
}

export default function ApprovalsView() {
  const [statusFilter, setStatusFilter] = useState<ApprovalStatus | undefined>('pending');
  const [selected, setSelected] = useState<ApprovalRequest | null>(null);
  const [comment, setComment] = useState('');
  const [approverId, setApproverId] = useState('web-user');
  const queryClient = useQueryClient();

  const { data, isLoading } = useQuery({
    queryKey: ['recovery-approvals', statusFilter],
    queryFn: () => fetchApprovals({ status: statusFilter }).then((r) => r.data),
    refetchInterval: 5000,
  });

  const approvals = data?.approvals ?? [];

  const approveMutation = useMutation({
    mutationFn: postApprovalApprove,
    onSuccess: (resp) => {
      const exec = resp.data.execution;
      if (exec.status === 'succeeded') {
        message.success(`审批通过,执行成功 (${exec.execution_id.slice(0, 8)})`);
      } else if (exec.status === 'failed') {
        message.warning(`审批通过但执行失败:${JSON.stringify(exec.result?.error || exec.result)}`);
      } else {
        message.info(`审批已通过,execution 状态:${exec.status}`);
      }
      queryClient.invalidateQueries({ queryKey: ['recovery-approvals'] });
      queryClient.invalidateQueries({ queryKey: ['recovery-executions'] });
      handleClose();
    },
    onError: (err: { response?: { data?: { detail?: string } }; message: string }) => {
      message.error(`审批失败:${err.response?.data?.detail || err.message}`);
    },
  });

  const rejectMutation = useMutation({
    mutationFn: postApprovalReject,
    onSuccess: () => {
      message.success('已驳回');
      queryClient.invalidateQueries({ queryKey: ['recovery-approvals'] });
      queryClient.invalidateQueries({ queryKey: ['recovery-executions'] });
      handleClose();
    },
    onError: (err: { response?: { data?: { detail?: string } }; message: string }) => {
      message.error(`驳回失败:${err.response?.data?.detail || err.message}`);
    },
  });

  const handleClose = () => {
    setSelected(null);
    setComment('');
  };

  const handleApprove = () => {
    if (!selected || !approverId.trim()) {
      message.warning('请填写审批人 ID');
      return;
    }
    approveMutation.mutate({
      approval_id: selected.approval_id,
      approver_id: approverId.trim(),
      comment,
    });
  };

  const handleReject = () => {
    if (!selected || !approverId.trim()) {
      message.warning('请填写审批人 ID');
      return;
    }
    if (!comment.trim()) {
      message.warning('驳回需要填写审批意见');
      return;
    }
    rejectMutation.mutate({
      approval_id: selected.approval_id,
      approver_id: approverId.trim(),
      comment,
    });
  };

  const columns: ColumnsType<ApprovalRequest> = [
    {
      title: '申请时间',
      dataIndex: 'requested_at',
      key: 'requested_at',
      width: 170,
      render: (v: string) => (v ? new Date(v).toLocaleString('zh-CN') : '-'),
    },
    {
      title: '状态',
      dataIndex: 'approval_status',
      key: 'approval_status',
      width: 100,
      render: (s: ApprovalStatus) => <Tag color={statusColor[s]}>{s}</Tag>,
    },
    {
      title: '动作',
      key: 'action',
      width: 200,
      render: (_, row) =>
        row.execution_summary ? (
          <Space size={4} direction="vertical" style={{ gap: 0 }}>
            <Text strong style={{ fontSize: 12 }}>
              {row.execution_summary.action_name}
            </Text>
            <Text type="secondary" style={{ fontSize: 10 }}>
              {row.execution_summary.action_id}
            </Text>
          </Space>
        ) : (
          <Text type="secondary">-</Text>
        ),
    },
    {
      title: '目标资源',
      key: 'target',
      ellipsis: true,
      render: (_, row) =>
        row.execution_summary ? (
          <Space size={4} direction="vertical" style={{ gap: 0 }}>
            <Text style={{ fontSize: 11, fontFamily: 'monospace' }}>
              {row.execution_summary.target_resource_id}
            </Text>
            <Tag>{row.execution_summary.target_resource_type}</Tag>
          </Space>
        ) : (
          <Text type="secondary">-</Text>
        ),
    },
    {
      title: '申请人',
      dataIndex: 'requested_by',
      key: 'requested_by',
      width: 120,
    },
    {
      title: '负责团队',
      dataIndex: 'approver_team',
      key: 'approver_team',
      width: 130,
      render: (t: string) => <Tag color="blue">{t}</Tag>,
    },
    {
      title: '剩余时间',
      dataIndex: 'expiry_at',
      key: 'expiry_at',
      width: 100,
      render: (v: string, row) =>
        row.approval_status === 'pending' ? (
          <Text style={{ fontSize: 12 }}>{formatRemaining(v)}</Text>
        ) : (
          <Text type="secondary">-</Text>
        ),
    },
  ];

  return (
    <div style={{ padding: 16, height: '100%', overflow: 'auto' }}>
      <Card
        title={
          <Space>
            <Text strong>审批中心</Text>
            <Tag>{data?.total ?? 0} 条</Tag>
          </Space>
        }
        extra={
          <Select
            allowClear
            style={{ width: 140 }}
            placeholder="状态过滤"
            options={STATUS_OPTIONS}
            value={statusFilter}
            onChange={setStatusFilter}
            size="small"
          />
        }
      >
        {approvals.length === 0 && !isLoading ? (
          <Empty description="暂无审批请求" />
        ) : (
          <Table
            rowKey="approval_id"
            columns={columns}
            dataSource={approvals}
            loading={isLoading}
            size="small"
            pagination={{ pageSize: 20, showSizeChanger: false }}
            onRow={(row) => ({
              onClick: () => setSelected(row),
              style: { cursor: 'pointer' },
            })}
          />
        )}
      </Card>

      <Drawer
        title="审批详情"
        open={!!selected}
        onClose={handleClose}
        width={600}
        footer={
          selected?.approval_status === 'pending' ? (
            <Space>
              <Button
                type="primary"
                icon={<CheckOutlined />}
                onClick={handleApprove}
                loading={approveMutation.isPending}
              >
                批准并执行
              </Button>
              <Button
                danger
                icon={<CloseOutlined />}
                onClick={handleReject}
                loading={rejectMutation.isPending}
              >
                驳回
              </Button>
            </Space>
          ) : null
        }
      >
        {selected && (
          <>
            <Descriptions column={1} size="small" bordered>
              <Descriptions.Item label="approval_id">
                <Text copyable code style={{ fontSize: 11 }}>
                  {selected.approval_id}
                </Text>
              </Descriptions.Item>
              <Descriptions.Item label="状态">
                <Tag color={statusColor[selected.approval_status]}>
                  {selected.approval_status}
                </Tag>
              </Descriptions.Item>
              {selected.execution_summary && (
                <>
                  <Descriptions.Item label="动作">
                    {selected.execution_summary.action_name} (
                    {selected.execution_summary.action_id})
                  </Descriptions.Item>
                  <Descriptions.Item label="目标">
                    <Text code style={{ fontSize: 11 }}>
                      {selected.execution_summary.target_resource_id}
                    </Text>
                  </Descriptions.Item>
                  <Descriptions.Item label="目标类型">
                    <Tag>{selected.execution_summary.target_resource_type}</Tag>
                  </Descriptions.Item>
                  {selected.execution_summary.dry_run_summary && (
                    <Descriptions.Item label="影响范围">
                      影响 {selected.execution_summary.dry_run_summary.affected_count} 资源 / SLA{' '}
                      {selected.execution_summary.dry_run_summary.estimated_sla_impact}
                    </Descriptions.Item>
                  )}
                </>
              )}
              <Descriptions.Item label="申请人">{selected.requested_by}</Descriptions.Item>
              <Descriptions.Item label="申请时间">{selected.requested_at}</Descriptions.Item>
              <Descriptions.Item label="申请理由">
                {selected.request_reason || '-'}
              </Descriptions.Item>
              <Descriptions.Item label="负责团队">
                <Tag color="blue">{selected.approver_team}</Tag>
              </Descriptions.Item>
              <Descriptions.Item label="过期时间">{selected.expiry_at}</Descriptions.Item>
              {selected.approver_id && (
                <Descriptions.Item label="审批人">{selected.approver_id}</Descriptions.Item>
              )}
              {selected.approval_comment && (
                <Descriptions.Item label="审批意见">
                  {selected.approval_comment}
                </Descriptions.Item>
              )}
            </Descriptions>

            {selected.approval_status === 'pending' && (
              <Card title="审批操作" size="small" style={{ marginTop: 16 }}>
                <Space direction="vertical" size={8} style={{ width: '100%' }}>
                  <Input
                    placeholder="审批人 ID"
                    value={approverId}
                    onChange={(e) => setApproverId(e.target.value)}
                    size="small"
                  />
                  <Input.TextArea
                    placeholder="审批意见(驳回必填)"
                    value={comment}
                    onChange={(e) => setComment(e.target.value)}
                    rows={3}
                  />
                </Space>
              </Card>
            )}
          </>
        )}
      </Drawer>
    </div>
  );
}
