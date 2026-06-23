import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  Button,
  Card,
  Descriptions,
  Drawer,
  Empty,
  Form,
  Input,
  Modal,
  Select,
  Space,
  Table,
  Tag,
  Typography,
  Timeline,
  Tooltip,
  message,
  Popconfirm,
} from 'antd';
import { ApiOutlined, PlayCircleOutlined, StopOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import {
  fetchChainTemplates,
  fetchChains,
  fetchChain,
  postChainExecute,
  postChainAbort,
  type ChainStatus,
  type ChainOnFailure,
  type ChainTemplate,
  type RecoveryChain,
} from '../../api/client';

const { Text, Paragraph } = Typography;

const chainStatusColor: Record<ChainStatus, string> = {
  pending: 'default',
  awaiting_approval: 'orange',
  executing: 'processing',
  succeeded: 'success',
  partial: 'warning',
  failed: 'error',
  rolled_back: 'magenta',
  aborted: 'default',
};

const onFailureLabel: Record<ChainOnFailure, string> = {
  stop: 'stop · 失败即停',
  rollback_all: 'rollback_all · 整链回滚',
  continue: 'continue · 失败继续',
};

export default function RecoveryChainsView() {
  const qc = useQueryClient();
  const [selectedChainId, setSelectedChainId] = useState<string | null>(null);
  const [executeModalOpen, setExecuteModalOpen] = useState(false);

  const { data: chainsResp, isLoading } = useQuery({
    queryKey: ['recovery-chains'],
    queryFn: () => fetchChains().then((r) => r.data),
    refetchInterval: 5000,
  });

  const { data: tplResp } = useQuery({
    queryKey: ['recovery-chain-templates'],
    queryFn: () => fetchChainTemplates().then((r) => r.data),
  });

  const { data: chainDetail } = useQuery({
    queryKey: ['recovery-chain', selectedChainId],
    queryFn: () => fetchChain(selectedChainId!).then((r) => r.data),
    enabled: !!selectedChainId,
    refetchInterval: selectedChainId ? 3000 : false,
  });

  const abortMutation = useMutation({
    mutationFn: ({ chainId, reason }: { chainId: string; reason: string }) =>
      postChainAbort(chainId, reason),
    onSuccess: () => {
      message.success('链已中止');
      qc.invalidateQueries({ queryKey: ['recovery-chains'] });
      if (selectedChainId) {
        qc.invalidateQueries({ queryKey: ['recovery-chain', selectedChainId] });
      }
    },
    onError: (err: { response?: { data?: { detail?: string } }; message: string }) => {
      message.error(`中止失败:${err.response?.data?.detail || err.message}`);
    },
  });

  const columns: ColumnsType<RecoveryChain> = [
    {
      title: '发起时间',
      dataIndex: 'initiated_at',
      width: 170,
      render: (v: string) => (v ? new Date(v).toLocaleString('zh-CN') : '-'),
    },
    {
      title: '模板',
      dataIndex: 'template_name',
      render: (n: string, row) => (
        <Space size={4} direction="vertical" style={{ gap: 0 }}>
          <Text strong style={{ fontSize: 12 }}>{n || row.template_id}</Text>
          <Text type="secondary" style={{ fontSize: 10 }}>{row.template_id}</Text>
        </Space>
      ),
    },
    {
      title: '目标',
      dataIndex: 'target_resource_id',
      ellipsis: true,
      render: (rid: string) => <Text style={{ fontSize: 11, fontFamily: 'monospace' }}>{rid}</Text>,
    },
    {
      title: '状态',
      dataIndex: 'status',
      width: 130,
      render: (s: ChainStatus) => <Tag color={chainStatusColor[s]}>{s}</Tag>,
    },
    {
      title: '进度',
      dataIndex: 'current_step_index',
      width: 90,
      render: (idx: number, row) => `${idx} / ${row.total_steps}`,
    },
    {
      title: '失败策略',
      dataIndex: 'on_failure',
      width: 150,
      render: (s: ChainOnFailure) => <Tag>{s}</Tag>,
    },
    {
      title: '操作',
      width: 120,
      render: (_v, row) => {
        const canAbort = ['pending', 'awaiting_approval', 'executing'].includes(row.status);
        return (
          <Space size={4}>
            <Button size="small" onClick={(e) => { e.stopPropagation(); setSelectedChainId(row.chain_id); }}>
              详情
            </Button>
            {canAbort && (
              <Popconfirm
                title="中止此链?"
                description="已执行步骤不会回滚"
                onConfirm={(e) => {
                  e?.stopPropagation();
                  abortMutation.mutate({ chainId: row.chain_id, reason: 'aborted via UI' });
                }}
                onCancel={(e) => e?.stopPropagation()}
              >
                <Button size="small" icon={<StopOutlined />} danger onClick={(e) => e.stopPropagation()}>
                  中止
                </Button>
              </Popconfirm>
            )}
          </Space>
        );
      },
    },
  ];

  const chains = chainsResp?.chains ?? [];

  return (
    <div style={{ padding: 16, height: '100%', overflow: 'auto' }}>
      <Card
        title={
          <Space>
            <ApiOutlined />
            <Text strong>恢复链编排</Text>
            <Tag>{chainsResp?.total ?? 0} 条</Tag>
          </Space>
        }
        extra={
          <Button
            type="primary"
            icon={<PlayCircleOutlined />}
            onClick={() => setExecuteModalOpen(true)}
          >
            发起恢复链
          </Button>
        }
      >
        {chains.length === 0 && !isLoading ? (
          <Empty description="尚无恢复链发起记录" />
        ) : (
          <Table
            rowKey="chain_id"
            columns={columns}
            dataSource={chains}
            loading={isLoading}
            size="small"
            pagination={{ pageSize: 20, showSizeChanger: false }}
            onRow={(row) => ({
              onClick: () => setSelectedChainId(row.chain_id),
              style: { cursor: 'pointer' },
            })}
          />
        )}
      </Card>

      <Drawer
        title="恢复链详情"
        open={!!selectedChainId}
        onClose={() => setSelectedChainId(null)}
        width={700}
      >
        {chainDetail && (
          <>
            <Descriptions column={1} size="small" bordered>
              <Descriptions.Item label="chain_id">
                <Text code copyable style={{ fontSize: 11 }}>{chainDetail.chain_id}</Text>
              </Descriptions.Item>
              <Descriptions.Item label="模板">
                {chainDetail.template_name} ({chainDetail.template_id})
              </Descriptions.Item>
              <Descriptions.Item label="目标">
                <Text code style={{ fontSize: 11 }}>{chainDetail.target_resource_id}</Text>
              </Descriptions.Item>
              <Descriptions.Item label="状态">
                <Tag color={chainStatusColor[chainDetail.status]}>{chainDetail.status}</Tag>
              </Descriptions.Item>
              <Descriptions.Item label="失败策略">
                {onFailureLabel[chainDetail.on_failure]}
              </Descriptions.Item>
              <Descriptions.Item label="进度">
                {chainDetail.current_step_index} / {chainDetail.total_steps}
              </Descriptions.Item>
              <Descriptions.Item label="发起人">{chainDetail.initiated_by}</Descriptions.Item>
              <Descriptions.Item label="发起时间">{chainDetail.initiated_at}</Descriptions.Item>
              <Descriptions.Item label="完成时间">{chainDetail.completed_at || '-'}</Descriptions.Item>
              {chainDetail.approval_id && (
                <Descriptions.Item label="审批 ID">
                  <Text code style={{ fontSize: 11 }}>{chainDetail.approval_id}</Text>
                </Descriptions.Item>
              )}
              {chainDetail.failure_reason && (
                <Descriptions.Item label="失败原因">
                  <Text type="warning" style={{ fontSize: 12 }}>{chainDetail.failure_reason}</Text>
                </Descriptions.Item>
              )}
            </Descriptions>

            <Card title="步骤时间线" size="small" style={{ marginTop: 16 }}>
              {chainDetail.steps && chainDetail.steps.length > 0 ? (
                <Timeline
                  items={chainDetail.steps.map((step) => ({
                    color:
                      step.status === 'succeeded' ? 'green'
                      : step.status === 'rolled_back' ? 'magenta'
                      : step.status === 'failed' ? 'red'
                      : 'gray',
                    children: (
                      <Space direction="vertical" size={0}>
                        <Text strong>
                          step {step.chain_step_index}:{step.action_name || step.action_id}
                        </Text>
                        <Space size={4}>
                          <Tag>{step.status}</Tag>
                          {step.verify_status && (
                            <Tooltip title={JSON.stringify(step.verify_result || {})}>
                              <Tag color={
                                step.verify_status === 'passed' ? 'success'
                                : step.verify_status === 'failed' ? 'error'
                                : 'default'
                              }>
                                verify: {step.verify_status}
                              </Tag>
                            </Tooltip>
                          )}
                        </Space>
                        <Text type="secondary" style={{ fontSize: 11 }}>
                          {step.completed_at || step.initiated_at}
                        </Text>
                      </Space>
                    ),
                  }))}
                />
              ) : (
                <Empty description="尚无步骤执行" image={Empty.PRESENTED_IMAGE_SIMPLE} />
              )}
            </Card>
          </>
        )}
      </Drawer>

      <ExecuteChainModal
        open={executeModalOpen}
        onClose={() => setExecuteModalOpen(false)}
        templates={tplResp?.templates ?? []}
        onSuccess={() => {
          setExecuteModalOpen(false);
          qc.invalidateQueries({ queryKey: ['recovery-chains'] });
        }}
      />
    </div>
  );
}

interface ExecuteModalProps {
  open: boolean;
  onClose: () => void;
  templates: ChainTemplate[];
  onSuccess: () => void;
}

function ExecuteChainModal({ open, onClose, templates, onSuccess }: ExecuteModalProps) {
  const [form] = Form.useForm();
  const selectedTemplateId = Form.useWatch('template_id', form);
  const selectedTemplate = templates.find((t) => t.template_id === selectedTemplateId);

  const mutation = useMutation({
    mutationFn: postChainExecute,
    onSuccess: (resp) => {
      const chain = resp.data;
      if (chain.status === 'awaiting_approval') {
        message.info(`链已发起,等待审批(${chain.approval_id?.slice(0, 8)})`);
      } else {
        message.success(`链执行 ${chain.status}`);
      }
      form.resetFields();
      onSuccess();
    },
    onError: (err: { response?: { data?: { detail?: string } }; message: string }) => {
      message.error(`发起失败:${err.response?.data?.detail || err.message}`);
    },
  });

  return (
    <Modal
      title="发起恢复链"
      open={open}
      onCancel={onClose}
      onOk={() => {
        form.validateFields().then((values) => mutation.mutate(values));
      }}
      okText="发起"
      confirmLoading={mutation.isPending}
    >
      <Form form={form} layout="vertical">
        <Form.Item
          label="链模板"
          name="template_id"
          rules={[{ required: true, message: '请选择模板' }]}
        >
          <Select
            placeholder="选择 chain template"
            options={templates.map((t) => ({
              value: t.template_id,
              label: `${t.name} (${t.target_type})`,
            }))}
          />
        </Form.Item>

        {selectedTemplate && (
          <Card size="small" style={{ marginBottom: 16, background: '#fafafa' }}>
            <Paragraph style={{ fontSize: 12, marginBottom: 8 }}>
              {selectedTemplate.description}
            </Paragraph>
            <Text type="secondary" style={{ fontSize: 11 }}>
              默认 on_failure: <Tag>{selectedTemplate.on_failure}</Tag> · target_type:{' '}
              <Tag>{selectedTemplate.target_type}</Tag>
            </Text>
            <Card title="步骤" size="small" style={{ marginTop: 8 }} bodyStyle={{ padding: 8 }}>
              {selectedTemplate.steps.map((s, i) => (
                <Text key={i} style={{ fontSize: 11, display: 'block' }}>
                  {i + 1}. <Tag>{s.action_id}</Tag>
                  {s.verify_required && <Tag color="cyan">verify</Tag>}
                  {Object.keys(s.params).length > 0 && (
                    <Text code style={{ fontSize: 10 }}>{JSON.stringify(s.params)}</Text>
                  )}
                </Text>
              ))}
            </Card>
          </Card>
        )}

        <Form.Item
          label="目标资源 ID"
          name="target_resource_id"
          rules={[{ required: true, message: '请填写目标资源 ID' }]}
        >
          <Input placeholder="例: deploy:vm-cluster:otel-demo:cart" />
        </Form.Item>

        <Form.Item label="发起人" name="initiated_by" initialValue="web-ui">
          <Input />
        </Form.Item>

        <Form.Item label="申请理由" name="request_reason">
          <Input.TextArea rows={2} placeholder="(可选)" />
        </Form.Item>

        <Form.Item label="覆盖 on_failure(可选)" name="on_failure_override">
          <Select
            allowClear
            placeholder="沿用模板默认"
            options={[
              { value: 'stop', label: 'stop · 失败即停' },
              { value: 'rollback_all', label: 'rollback_all · 整链回滚' },
              { value: 'continue', label: 'continue · 失败继续' },
            ]}
          />
        </Form.Item>
      </Form>
    </Modal>
  );
}
