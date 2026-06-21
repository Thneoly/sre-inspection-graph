import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  Button,
  Card,
  Checkbox,
  Empty,
  Form,
  Input,
  message,
  Modal,
  Popconfirm,
  Select,
  Space,
  Switch,
  Table,
  Tag,
  Typography,
} from 'antd';
import { CalendarOutlined, MailOutlined, PlayCircleOutlined, DeleteOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import {
  ALL_REPORT_MODULES,
  CLUSTER_REPORT_MODULES,
  INCIDENT_REPORT_MODULES,
  modulesForTemplate,
  fetchSubscriptions,
  postSubscription,
  patchSubscription,
  deleteSubscription,
  postTriggerSubscription,
  type ReportModule,
  type ReportSubscription,
  type ReportTemplate,
} from '../../api/client';

const { Text } = Typography;

const statusColor: Record<ReportSubscription['last_status'], string> = {
  never: 'default',
  ok: 'success',
  failed: 'error',
};
const statusLabel: Record<ReportSubscription['last_status'], string> = {
  never: '未运行',
  ok: '上次成功',
  failed: '上次失败',
};

const templateLabel: Record<ReportTemplate, string> = {
  application_health: '应用健康',
  cluster_overview: '集群总览',
  incident_report: '事件报告',
};

const moduleLabel: Record<ReportModule, string> = {
  health_score: '健康度评分',
  seven_views: '视图汇总',
  risk_list: '风险清单',
  recommended_actions: '推荐动作',
  historical_trends: '历史趋势',
  cluster_health: '集群健康',
  cluster_risk_top_n: '风险 Top-N',
  cluster_changes: '变更汇总',
  cluster_recoveries: '恢复汇总',
  incident_summary: '事件摘要',
  incident_timeline: '时间线',
  incident_recoveries: '恢复 & 推荐',
};

const CRON_PRESETS: Array<{ label: string; cron: string }> = [
  { label: '每周一 9 点', cron: '0 9 * * 1' },
  { label: '每日 9 点', cron: '0 9 * * *' },
  { label: '每月 1 号 9 点', cron: '0 9 1 * *' },
  { label: '每小时', cron: '0 * * * *' },
];

interface FormValues {
  template_id: ReportTemplate;
  application_id?: string;
  cluster_id?: string;
  fault_id?: string;
  change_event_id?: string;
  cron: string;
  recipients: string;  // comma-separated
  modules: ReportModule[];
  enabled: boolean;
}

/**
 * 订阅管理面板 — PRD-003 Sprint 2。
 *
 * - Table 列订阅(模板/scope/cron/收件人/状态/操作),3s 刷新
 * - 「新建订阅」Modal 表单(动态 scope 字段)
 * - 行操作:立即运行 / 暂停切换 / 删除(二次确认)
 */
export default function SubscriptionsPanel() {
  const [modalOpen, setModalOpen] = useState(false);
  const [form] = Form.useForm<FormValues>();
  const qc = useQueryClient();
  const templateWatch = Form.useWatch('template_id', form) ?? 'application_health';

  const { data, isLoading } = useQuery({
    queryKey: ['report-subscriptions'],
    queryFn: () => fetchSubscriptions().then((r) => r.data),
    refetchInterval: 5000,
  });

  const createMutation = useMutation({
    mutationFn: postSubscription,
    onSuccess: () => {
      message.success('订阅已创建');
      qc.invalidateQueries({ queryKey: ['report-subscriptions'] });
      setModalOpen(false);
      form.resetFields();
    },
    onError: (err: { response?: { data?: { detail?: string } }; message: string }) => {
      message.error(`创建失败:${err.response?.data?.detail || err.message}`);
    },
  });

  const triggerMutation = useMutation({
    mutationFn: postTriggerSubscription,
    onSuccess: () => {
      message.success('已立即触发');
      qc.invalidateQueries({ queryKey: ['report-subscriptions'] });
      qc.invalidateQueries({ queryKey: ['reports'] });
    },
    onError: (err: { response?: { data?: { detail?: string } }; message: string }) => {
      message.error(`触发失败:${err.response?.data?.detail || err.message}`);
    },
  });

  const toggleMutation = useMutation({
    mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) =>
      patchSubscription(id, { enabled }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['report-subscriptions'] });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: deleteSubscription,
    onSuccess: () => {
      message.success('订阅已删除');
      qc.invalidateQueries({ queryKey: ['report-subscriptions'] });
    },
  });

  const columns: ColumnsType<ReportSubscription> = [
    {
      title: '模板',
      dataIndex: 'template_id',
      key: 'template_id',
      render: (t: ReportTemplate) => <Tag>{templateLabel[t] || t}</Tag>,
    },
    {
      title: '范围',
      key: 'scope',
      render: (_: unknown, r: ReportSubscription) => {
        const s = r.scope;
        return s.application_id || s.cluster_id || s.fault_id || s.change_event_id || '总览';
      },
    },
    {
      title: 'Cron',
      dataIndex: 'cron',
      key: 'cron',
      render: (c: string) => <Text code><CalendarOutlined /> {c}</Text>,
    },
    {
      title: '收件人',
      dataIndex: 'recipients',
      key: 'recipients',
      render: (r: string[]) => (
        <Space size={4} wrap>
          {r.map((e) => (
            <Tag key={e} icon={<MailOutlined />}>{e}</Tag>
          ))}
        </Space>
      ),
    },
    {
      title: '最近运行',
      key: 'last_run',
      render: (_: unknown, r: ReportSubscription) => (
        <Space direction="vertical" size={2}>
          <Tag color={statusColor[r.last_status]}>{statusLabel[r.last_status]}</Tag>
          {r.last_run_at && (
            <Text type="secondary" style={{ fontSize: 12 }}>
              {new Date(r.last_run_at).toLocaleString()}
            </Text>
          )}
          {r.last_error && (
            <Text type="danger" style={{ fontSize: 12 }}>{r.last_error}</Text>
          )}
        </Space>
      ),
    },
    {
      title: '启用',
      dataIndex: 'enabled',
      key: 'enabled',
      render: (_: boolean, r: ReportSubscription) => (
        <Switch
          checked={r.enabled}
          loading={toggleMutation.isPending}
          onChange={(v) => toggleMutation.mutate({ id: r.subscription_id, enabled: v })}
        />
      ),
    },
    {
      title: '操作',
      key: 'action',
      render: (_: unknown, r: ReportSubscription) => (
        <Space size={4}>
          <Button
            type="link"
            size="small"
            icon={<PlayCircleOutlined />}
            loading={triggerMutation.isPending}
            onClick={() => triggerMutation.mutate(r.subscription_id)}
          >
            立即运行
          </Button>
          <Popconfirm
            title="确定删除此订阅?"
            onConfirm={() => deleteMutation.mutate(r.subscription_id)}
            okText="删除"
            cancelText="取消"
          >
            <Button type="link" size="small" danger icon={<DeleteOutlined />}>删除</Button>
          </Popconfirm>
        </Space>
      ),
    },
  ];

  const handleSubmit = () => {
    form
      .validateFields()
      .then((vals) => {
        const scope: Record<string, string> = {};
        if (vals.template_id === 'application_health' && vals.application_id) {
          scope.application_id = vals.application_id;
        }
        if (vals.template_id === 'cluster_overview' && vals.cluster_id) {
          scope.cluster_id = vals.cluster_id;
        }
        if (vals.template_id === 'incident_report') {
          if (vals.fault_id) scope.fault_id = vals.fault_id;
          if (vals.change_event_id) scope.change_event_id = vals.change_event_id;
        }
        const recipients = vals.recipients
          .split(',')
          .map((e) => e.trim())
          .filter(Boolean);

        createMutation.mutate({
          template_id: vals.template_id,
          scope,
          modules: vals.modules,
          cron: vals.cron,
          recipients,
          enabled: vals.enabled ?? true,
        });
      })
      .catch(() => {});
  };

  return (
    <Card
      title={<Text strong>订阅管理</Text>}
      extra={
        <Button type="primary" onClick={() => setModalOpen(true)}>
          新建订阅
        </Button>
      }
    >
      {isLoading ? (
        <Text type="secondary">加载中...</Text>
      ) : !data || data.subscriptions.length === 0 ? (
        <Empty description="暂无订阅,点击右上角「新建订阅」" />
      ) : (
        <Table
          rowKey="subscription_id"
          columns={columns}
          dataSource={data.subscriptions}
          pagination={{ pageSize: 10 }}
          size="small"
        />
      )}

      <Modal
        title="新建订阅"
        open={modalOpen}
        onCancel={() => setModalOpen(false)}
        onOk={handleSubmit}
        confirmLoading={createMutation.isPending}
        okText="创建"
        cancelText="取消"
        width={640}
      >
        <Form
          form={form}
          layout="vertical"
          initialValues={{
            template_id: 'application_health' as ReportTemplate,
            cron: '0 9 * * 1',
            modules: ALL_REPORT_MODULES,
            enabled: true,
            recipients: '',
          }}
        >
          <Form.Item name="template_id" label="模板" rules={[{ required: true }]}>
            <Select
              options={[
                { value: 'application_health', label: '应用健康报告' },
                { value: 'cluster_overview', label: '集群/全公司总览' },
                { value: 'incident_report', label: '事件报告' },
              ]}
              onChange={(v: ReportTemplate) => {
                form.setFieldValue('modules', modulesForTemplate(v));
              }}
            />
          </Form.Item>

          {templateWatch === 'application_health' && (
            <Form.Item
              name="application_id"
              label="应用 ID"
              rules={[{ required: true, message: '请输入应用 ID' }]}
            >
              <Input placeholder="app:order" />
            </Form.Item>
          )}
          {templateWatch === 'cluster_overview' && (
            <Form.Item name="cluster_id" label="集群 ID(可空=全公司)">
              <Input placeholder="vm-cluster" />
            </Form.Item>
          )}
          {templateWatch === 'incident_report' && (
            <>
              <Form.Item name="fault_id" label="故障 ID(fault_id / change_event_id 二选一)">
                <Input placeholder="flt-xxx" />
              </Form.Item>
              <Form.Item name="change_event_id" label="变更事件 ID">
                <Input placeholder="ce-xxx" />
              </Form.Item>
            </>
          )}

          <Form.Item label="Cron" required>
            <Space direction="vertical" style={{ width: '100%' }}>
              <Form.Item name="cron" noStyle rules={[{ required: true, message: '请输入 cron' }]}>
                <Input placeholder="0 9 * * 1" />
              </Form.Item>
              <Space size={4} wrap>
                {CRON_PRESETS.map((p) => (
                  <Button
                    key={p.cron}
                    size="small"
                    onClick={() => form.setFieldValue('cron', p.cron)}
                  >
                    {p.label}
                  </Button>
                ))}
              </Space>
            </Space>
          </Form.Item>

          <Form.Item
            name="recipients"
            label="收件人(逗号分隔多个)"
            rules={[{ required: true, message: '请输入至少一个邮箱' }]}
          >
            <Input placeholder="sre@example.com, ops@example.com" />
          </Form.Item>

          <Form.Item name="modules" label="启用模块">
            <Checkbox.Group
              options={
                templateWatch === 'cluster_overview'
                  ? CLUSTER_REPORT_MODULES.map((m) => ({ label: moduleLabel[m], value: m }))
                  : templateWatch === 'incident_report'
                    ? INCIDENT_REPORT_MODULES.map((m) => ({ label: moduleLabel[m], value: m }))
                    : ALL_REPORT_MODULES.map((m) => ({ label: moduleLabel[m], value: m }))
              }
            />
          </Form.Item>

          <Form.Item name="enabled" label="启用" valuePropName="checked">
            <Switch />
          </Form.Item>
        </Form>
      </Modal>
    </Card>
  );
}
