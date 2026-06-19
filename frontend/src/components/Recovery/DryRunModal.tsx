import { useState } from 'react';
import { useMutation } from '@tanstack/react-query';
import {
  Modal,
  Button,
  Descriptions,
  Tag,
  Typography,
  Alert,
  Space,
  List,
  Empty,
  Input,
  message,
  Spin,
  Divider,
} from 'antd';
import {
  WarningOutlined,
  CheckCircleOutlined,
  PlayCircleOutlined,
  EyeOutlined,
} from '@ant-design/icons';
import {
  postRecoveryDryRun,
  postRecoveryExecute,
  type RecoveryAction,
  type DryRunResult,
  type RecoveryExecution,
  type RiskLevel,
  type AffectedResource,
} from '../../api/client';

const { Text, Paragraph } = Typography;

interface Props {
  open: boolean;
  action: RecoveryAction | null;
  targetResourceId: string;
  findingId?: string;
  onClose: () => void;
  onExecuted?: (execution: RecoveryExecution) => void;
}

const riskTagColor: Record<RiskLevel, string> = {
  low: 'success',
  medium: 'warning',
  high: 'error',
};

const severityColor: Record<AffectedResource['impact_severity'], string> = {
  minimal: 'default',
  low: 'blue',
  medium: 'orange',
  high: 'red',
};

export default function DryRunModal({
  open,
  action,
  targetResourceId,
  findingId,
  onClose,
  onExecuted,
}: Props) {
  const [inputParams, setInputParams] = useState<string>('{}');
  const [requestReason, setRequestReason] = useState<string>('');
  const [dryRunResult, setDryRunResult] = useState<DryRunResult | null>(null);

  // Dry-run mutation
  const dryRunMutation = useMutation({
    mutationFn: postRecoveryDryRun,
    onSuccess: (resp) => setDryRunResult(resp.data),
    onError: (err: Error) => message.error(`dry-run 失败: ${err.message}`),
  });

  // Execute mutation
  const executeMutation = useMutation({
    mutationFn: postRecoveryExecute,
    onSuccess: (resp) => {
      const exec = resp.data;
      if (exec.status === 'succeeded') {
        message.success(`执行成功 (${exec.execution_id.slice(0, 8)})`);
      } else if (exec.status === 'failed') {
        message.error(`执行失败: ${JSON.stringify(exec.result?.error || exec.result)}`);
      } else {
        message.info(`状态: ${exec.status}`);
      }
      onExecuted?.(exec);
      handleClose();
    },
    onError: (err: { response?: { data?: { detail?: string } }; message: string }) => {
      const detail = err.response?.data?.detail || err.message;
      message.error(`执行被拒: ${detail}`);
    },
  });

  const handleClose = () => {
    setDryRunResult(null);
    setInputParams('{}');
    setRequestReason('');
    onClose();
  };

  const handleDryRun = () => {
    if (!action) return;
    let parsedParams: Record<string, unknown>;
    try {
      parsedParams = inputParams.trim() ? JSON.parse(inputParams) : {};
    } catch {
      message.error('input_params 不是合法 JSON');
      return;
    }
    dryRunMutation.mutate({
      action_id: action.action_id,
      target_resource_id: targetResourceId,
      input_params: parsedParams,
      finding_id: findingId,
    });
  };

  const handleExecute = () => {
    if (!action || !dryRunResult || !dryRunResult.target_valid) return;
    let parsedParams: Record<string, unknown>;
    try {
      parsedParams = inputParams.trim() ? JSON.parse(inputParams) : {};
    } catch {
      message.error('input_params 不是合法 JSON');
      return;
    }
    executeMutation.mutate({
      action_id: action.action_id,
      target_resource_id: targetResourceId,
      input_params: parsedParams,
      finding_id: findingId,
      initiated_by: 'web-ui',
      request_reason: requestReason,
    });
  };

  const isHighRisk = action?.risk_level === 'high';
  const needsApproval = action?.requires_approval || false;
  const canExecute =
    !!dryRunResult && dryRunResult.target_valid && action?.risk_level === 'low' && !needsApproval;

  if (!action) return null;

  return (
    <Modal
      title={
        <Space>
          <Tag color={riskTagColor[action.risk_level]}>{action.risk_level}</Tag>
          <Text strong>{action.action_name}</Text>
          {needsApproval && <Tag color="orange">需审批</Tag>}
        </Space>
      }
      open={open}
      onCancel={handleClose}
      width={720}
      footer={[
        <Button key="cancel" onClick={handleClose}>
          取消
        </Button>,
        <Button
          key="dry-run"
          icon={<EyeOutlined />}
          onClick={handleDryRun}
          loading={dryRunMutation.isPending}
        >
          预演 (Dry-run)
        </Button>,
        <Button
          key="execute"
          type="primary"
          danger={isHighRisk}
          icon={<PlayCircleOutlined />}
          disabled={!canExecute}
          loading={executeMutation.isPending}
          onClick={handleExecute}
        >
          {needsApproval || isHighRisk ? '审批后执行 (Sprint 3)' : '执行'}
        </Button>,
      ]}
    >
      <Descriptions column={2} size="small" bordered style={{ marginBottom: 16 }}>
        <Descriptions.Item label="目标资源" span={2}>
          <Text copyable code style={{ fontSize: 11 }}>
            {targetResourceId}
          </Text>
        </Descriptions.Item>
        <Descriptions.Item label="动作类别">{action.action_category}</Descriptions.Item>
        <Descriptions.Item label="预计耗时">{action.estimated_duration_seconds}s</Descriptions.Item>
        <Descriptions.Item label="SLA 影响">{action.sla_impact_estimate}</Descriptions.Item>
        <Descriptions.Item label="可回滚">
          {action.rollback_action_id ? <Tag color="success">是</Tag> : <Tag>否</Tag>}
        </Descriptions.Item>
      </Descriptions>

      <Paragraph type="secondary" style={{ fontSize: 12 }}>
        {action.description}
      </Paragraph>

      {action.warnings.length > 0 && (
        <Alert
          type="warning"
          showIcon
          icon={<WarningOutlined />}
          message="动作警告"
          description={
            <ul style={{ margin: 0, paddingLeft: 18 }}>
              {action.warnings.map((w, i) => (
                <li key={i}>{w}</li>
              ))}
            </ul>
          }
          style={{ marginBottom: 16 }}
        />
      )}

      <Divider plain>
        <span style={{ fontSize: 12, color: '#888' }}>输入参数 (JSON)</span>
      </Divider>
      <Input.TextArea
        value={inputParams}
        onChange={(e) => setInputParams(e.target.value)}
        rows={3}
        placeholder='{"replicas_delta": 2}'
        style={{ marginBottom: 16, fontFamily: 'monospace', fontSize: 12 }}
      />

      {needsApproval && (
        <>
          <Divider plain>
            <span style={{ fontSize: 12, color: '#888' }}>申请理由(用于审计)</span>
          </Divider>
          <Input.TextArea
            value={requestReason}
            onChange={(e) => setRequestReason(e.target.value)}
            rows={2}
            placeholder="例如:业务低峰期扩容应对预期流量"
            style={{ marginBottom: 16 }}
          />
        </>
      )}

      <Divider plain>
        <span style={{ fontSize: 12, color: '#888' }}>预演结果</span>
      </Divider>
      {dryRunMutation.isPending && <Spin tip="计算影响范围..." />}
      {!dryRunResult && !dryRunMutation.isPending && (
        <Empty description="点击「预演」查看影响范围" />
      )}

      {dryRunResult && !dryRunResult.target_valid && (
        <Alert
          type="error"
          showIcon
          message="目标校验失败"
          description={dryRunResult.validation_error}
        />
      )}

      {dryRunResult && dryRunResult.target_valid && (
        <>
          <Space wrap style={{ marginBottom: 12 }}>
            <Tag icon={<CheckCircleOutlined />} color="success">
              影响 {dryRunResult.affected_count} 个资源
            </Tag>
            <Tag>持续 {dryRunResult.estimated_duration_seconds}s</Tag>
            <Tag color="blue">SLA {dryRunResult.estimated_sla_impact}</Tag>
            {dryRunResult.rollback_action_id && (
              <Tag color="cyan">
                回滚: {dryRunResult.rollback_action_id}
                {dryRunResult.rollback_input_params &&
                  ` (${JSON.stringify(dryRunResult.rollback_input_params)})`}
              </Tag>
            )}
          </Space>

          {dryRunResult.affected_resources.length > 0 ? (
            <List
              size="small"
              dataSource={dryRunResult.affected_resources.slice(0, 20)}
              renderItem={(r) => (
                <List.Item style={{ padding: '4px 0' }}>
                  <Space size="small" style={{ width: '100%' }}>
                    <Tag color={severityColor[r.impact_severity]}>{r.impact_severity}</Tag>
                    <Tag>{r.type}</Tag>
                    <Text style={{ fontSize: 12 }}>{r.name}</Text>
                    {r.via_relations.length > 0 && (
                      <Text type="secondary" style={{ fontSize: 11 }}>
                        via {r.via_relations.join(', ')}
                      </Text>
                    )}
                  </Space>
                </List.Item>
              )}
            />
          ) : (
            <Text type="secondary">无影响资源(动作仅影响目标自身)</Text>
          )}
          {dryRunResult.affected_resources.length > 20 && (
            <Text type="secondary" style={{ fontSize: 11 }}>
              ...还有 {dryRunResult.affected_resources.length - 20} 个未显示
            </Text>
          )}

          {(needsApproval || isHighRisk) && (
            <Alert
              type="info"
              showIcon
              style={{ marginTop: 16 }}
              message="此动作需审批 (Sprint 3 上线)"
              description="当前 Sprint 2 仅支持 low_risk + 不需审批的动作直接执行。"
            />
          )}
        </>
      )}
    </Modal>
  );
}
