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
  Tag,
  Typography,
} from 'antd';
import { FileTextOutlined, DownloadOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import {
  ALL_REPORT_MODULES,
  downloadReport,
  fetchReports,
  postReportGenerate,
  type ReportModule,
  type ReportStatus,
  type ReportTask,
  type ReportTemplate,
} from '../../api/client';
import { downloadBlob } from '../../utils/download';

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
 * 报告中心 — PRD-003 Sprint 1。
 *
 * - Table 列历史报告(模板/应用/状态/进度/创建时间/下载),generating 行 3s 自动刷新
 * - 「生成新报告」按钮 → Modal 表单(模板/应用/时间范围/模块多选)
 * - 下载(completed 行)→ blob → .md 文件
 */
export default function ReportsView() {
  const [modalOpen, setModalOpen] = useState(false);
  const queryClient = useQueryClient();
  const [form] = Form.useForm();

  const { data, isLoading } = useQuery({
    queryKey: ['reports'],
    queryFn: () => fetchReports().then((r) => r.data),
    refetchInterval: 3000, // 让 generating 行自动刷新到 completed
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
      render: (t: string) => (t === 'application_health' ? '应用健康报告' : t),
    },
    {
      title: '应用',
      key: 'application_id',
      render: (_: unknown, r: ReportTask) => r.scope.application_id || r.scope.cluster_id || '-',
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
      .then((vals: {
        template_id: ReportTemplate;
        application_id: string;
        range: RangePreset;
        modules: ReportModule[];
      }) => {
        generateMutation.mutate({
          template_id: vals.template_id,
          scope: {
            application_id: vals.application_id,
            time_range_start: isoMinusSeconds(RANGE_SECONDS[vals.range]),
          },
          format: 'markdown',
          modules: vals.modules,
        });
      })
      .catch(() => {});
  };

  return (
    <div style={{ padding: 16, height: '100%', overflow: 'auto' }}>
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
              options={[{ value: 'application_health', label: '应用健康报告' }]}
            />
          </Form.Item>
          <Form.Item
            name="application_id"
            label="应用 ID"
            rules={[{ required: true, message: '请输入应用 ID' }]}
          >
            <Input placeholder="app:order" />
          </Form.Item>
          <Form.Item name="range" label="时间范围">
            <Radio.Group>
              <Radio.Button value="1d">近 1 天</Radio.Button>
              <Radio.Button value="7d">近 7 天</Radio.Button>
              <Radio.Button value="30d">近 30 天</Radio.Button>
            </Radio.Group>
          </Form.Item>
          <Form.Item name="modules" label="启用模块">
            <Checkbox.Group
              options={ALL_REPORT_MODULES.map((m) => ({ label: moduleLabel[m], value: m }))}
            />
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}
