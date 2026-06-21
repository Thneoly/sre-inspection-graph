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
  Radio,
  Select,
  Space,
  Table,
  Tabs,
  Tag,
  Typography,
} from 'antd';
import { FileTextOutlined, DownloadOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import {
  ALL_REPORT_MODULES,
  CLUSTER_REPORT_MODULES,
  INCIDENT_REPORT_MODULES,
  downloadReport,
  fetchReports,
  modulesForTemplate,
  postReportGenerate,
  type ReportModule,
  type ReportStatus,
  type ReportTask,
  type ReportTemplate,
} from '../../api/client';
import { downloadBlob } from '../../utils/download';
import SubscriptionsPanel from './SubscriptionsPanel';

const { Text } = Typography;

const statusColor: Record<ReportStatus, string> = {
  pending: 'orange',
  generating: 'processing',
  completed: 'success',
  failed: 'error',
};

const statusLabel: Record<ReportStatus, string> = {
  pending: '排队中',
  generating: '生成中',
  completed: '已完成',
  failed: '失败',
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

const templateLabel: Record<ReportTemplate, string> = {
  application_health: '应用健康报告',
  cluster_overview: '集群/总览',
  incident_report: '事件报告',
};

type RangePreset = '1d' | '7d' | '30d';
const RANGE_SECONDS: Record<RangePreset, number> = {
  '1d': 86400,
  '7d': 7 * 86400,
  '30d': 30 * 86400,
};

function isoMinusSeconds(s: number): string {
  return new Date(Date.now() - s * 1000).toISOString();
}

function fullTimestamp(iso: string): string {
  if (!iso) return '-';
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
}

/**
 * 报告中心 — PRD-003 Sprint 1 + Sprint 2。
 *
 * - Tab「报告列表」:Table + 生成新报告 Modal(动态 scope 字段)
 * - Tab「订阅管理」:SubscriptionsPanel
 */
export default function ReportsView() {
  return (
    <div style={{ padding: 16, height: '100%', overflow: 'auto' }}>
      <Tabs
        defaultActiveKey="list"
        items={[
          { key: 'list', label: '报告列表', children: <ReportsListPanel /> },
          { key: 'subs', label: '订阅管理', children: <SubscriptionsPanel /> },
        ]}
      />
    </div>
  );
}

interface FormValues {
  template_id: ReportTemplate;
  application_id?: string;
  cluster_id?: string;
  fault_id?: string;
  change_event_id?: string;
  range: RangePreset;
  modules: ReportModule[];
}

function ReportsListPanel() {
  const [modalOpen, setModalOpen] = useState(false);
  const queryClient = useQueryClient();
  const [form] = Form.useForm<FormValues>();
  const templateWatch = Form.useWatch('template_id', form) ?? 'application_health';

  const { data, isLoading } = useQuery({
    queryKey: ['reports'],
    queryFn: () => fetchReports().then((r) => r.data),
    refetchInterval: 3000,
  });

  const generateMutation = useMutation({
    mutationFn: postReportGenerate,
    onSuccess: () => {
      message.success('已提交生成,完成后可下载');
      queryClient.invalidateQueries({ queryKey: ['reports'] });
      setModalOpen(false);
      form.resetFields();
    },
    onError: (err: { response?: { data?: { detail?: string } }; message: string }) => {
      message.error(`生成失败:${err.response?.data?.detail || err.message}`);
    },
  });

  const downloadMutation = useMutation({
    mutationFn: downloadReport,
    onSuccess: (resp, reportId) => {
      downloadBlob(resp.data, `${reportId}.md`);
    },
    onError: (err: { response?: { data?: { detail?: string } }; message: string }) => {
      message.error(`下载失败:${err.response?.data?.detail || err.message}`);
    },
  });

  const handleDownload = (record: ReportTask) => {
    if (record.status !== 'completed') {
      message.warning('报告尚未生成完成');
      return;
    }
    downloadMutation.mutate(record.report_id);
  };

  const columns: ColumnsType<ReportTask> = [
    {
      title: '报告 ID',
      dataIndex: 'report_id',
      key: 'report_id',
      render: (id: string) => <Text code>{id.slice(0, 16)}</Text>,
    },
    {
      title: '模板',
      dataIndex: 'template_id',
      key: 'template_id',
      render: (t: ReportTemplate) => templateLabel[t] || t,
    },
    {
      title: '范围',
      key: 'scope',
      render: (_: unknown, r: ReportTask) =>
        r.scope.application_id ||
        r.scope.cluster_id ||
        r.scope.fault_id ||
        r.scope.change_event_id ||
        '总览',
    },
    {
      title: '状态',
      dataIndex: 'status',
      key: 'status',
      render: (s: ReportStatus) => <Tag color={statusColor[s]}>{statusLabel[s]}</Tag>,
    },
    {
      title: '进度',
      key: 'progress',
      render: (_: unknown, r: ReportTask) =>
        r.status === 'failed' ? (
          <Text type="danger" style={{ fontSize: 12 }}>{r.error_message || '失败'}</Text>
        ) : (
          <Text type="secondary">{r.progress}%</Text>
        ),
    },
    {
      title: '创建时间',
      dataIndex: 'created_at',
      key: 'created_at',
      render: fullTimestamp,
    },
    {
      title: '操作',
      key: 'action',
      render: (_: unknown, r: ReportTask) => (
        <Button
          type="link"
          size="small"
          icon={<DownloadOutlined />}
          disabled={r.status !== 'completed'}
          loading={downloadMutation.isPending}
          onClick={() => handleDownload(r)}
        >
          下载
        </Button>
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
        if (vals.range) {
          scope.time_range_start = isoMinusSeconds(RANGE_SECONDS[vals.range]);
        }

        generateMutation.mutate({
          template_id: vals.template_id,
          scope,
          format: 'markdown',
          modules: vals.modules,
        });
      })
      .catch(() => {});
  };

  return (
    <>
      <Card
        title={
          <Space>
            <FileTextOutlined />
            <Text strong>报告中心</Text>
          </Space>
        }
        extra={
          <Button type="primary" icon={<FileTextOutlined />} onClick={() => setModalOpen(true)}>
            生成新报告
          </Button>
        }
      >
        {isLoading ? (
          <Text type="secondary">加载中...</Text>
        ) : !data || data.reports.length === 0 ? (
          <Empty description="暂无报告,点击右上角「生成新报告」" />
        ) : (
          <Table
            rowKey="report_id"
            columns={columns}
            dataSource={data.reports}
            pagination={{ pageSize: 10 }}
            size="small"
          />
        )}
      </Card>

      <Modal
        title="生成新报告"
        open={modalOpen}
        onCancel={() => setModalOpen(false)}
        onOk={handleSubmit}
        confirmLoading={generateMutation.isPending}
        okText="生成"
        cancelText="取消"
        width={640}
      >
        <Form
          form={form}
          layout="vertical"
          initialValues={{
            template_id: 'application_health' as ReportTemplate,
            application_id: 'app:order',
            range: '7d' as RangePreset,
            modules: [...ALL_REPORT_MODULES],
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

          <Form.Item name="range" label="时间范围">
            <Radio.Group>
              <Radio.Button value="1d">近 1 天</Radio.Button>
              <Radio.Button value="7d">近 7 天</Radio.Button>
              <Radio.Button value="30d">近 30 天</Radio.Button>
            </Radio.Group>
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
        </Form>
      </Modal>
    </>
  );
}
