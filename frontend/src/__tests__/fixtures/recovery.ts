/**
 * Test fixtures for Recovery components.
 *
 * Mirrors the Pydantic shapes returned by `backend/app/routers/recovery.py`
 * — keep these in sync when API contract changes.
 */
import type {
  RecoveryAction,
  RecoveryExecution,
  DryRunResult,
  ApprovalRequest,
} from '../../api/client';

export const mockActionScale: RecoveryAction = {
  action_id: 'scale_deployment',
  action_name: '扩缩 Deployment',
  action_category: 'capacity',
  target_resource_type: 'Deployment',
  risk_level: 'low',
  requires_approval: false,
  rollback_action_id: 'scale_deployment',
  estimated_duration_seconds: 30,
  description: '调整 Deployment 副本数。线性增量,通常用于应对流量峰值或释放资源。',
  input_schema: { replicas_delta: { type: 'integer' } },
  sla_impact_estimate: 'minimal',
  warnings: [],
};

export const mockActionRestartPod: RecoveryAction = {
  action_id: 'restart_pod',
  action_name: '重启 Pod',
  action_category: 'restart',
  target_resource_type: 'Pod',
  risk_level: 'medium',
  requires_approval: false,
  rollback_action_id: null,
  estimated_duration_seconds: 20,
  description: '触发单个 Pod 删除,由 ReplicaSet 重建。短暂中断该副本。',
  input_schema: {},
  sla_impact_estimate: 'low',
  warnings: ['会短暂丢失正在处理的请求'],
};

export const mockActionRollback: RecoveryAction = {
  action_id: 'rollback_deployment',
  action_name: '回滚 Deployment',
  action_category: 'rollback',
  target_resource_type: 'Deployment',
  risk_level: 'high',
  requires_approval: true,
  rollback_action_id: null,
  estimated_duration_seconds: 90,
  description: '回滚到上一版本。影响所有副本,可能导致请求失败。',
  input_schema: {},
  sla_impact_estimate: 'medium',
  warnings: ['滚动期间部分请求可能失败', '需提前通知业务方'],
};

export const mockDryRunValid: DryRunResult = {
  action_id: 'scale_deployment',
  action_name: '扩缩 Deployment',
  target_resource_id: 'deploy:cce-prod-01:order:order-api',
  target_resource_type: 'Deployment',
  target_resource_name: 'order-api',
  target_valid: true,
  validation_error: null,
  affected_resources: [
    {
      resource_id: 'pod:cce-prod-01:order:order-api-1',
      type: 'Pod',
      name: 'order-api-1',
      impact_severity: 'low',
      via_relations: ['CONTAINS'],
      notes: [],
    },
    {
      resource_id: 'pod:cce-prod-01:order:order-api-2',
      type: 'Pod',
      name: 'order-api-2',
      impact_severity: 'minimal',
      via_relations: ['CONTAINS'],
      notes: [],
    },
  ],
  affected_count: 2,
  estimated_duration_seconds: 30,
  estimated_sla_impact: 'minimal',
  warnings: [],
  rollback_action_id: 'scale_deployment',
  rollback_input_params: { replicas_delta: -2 },
  risk_level: 'low',
  requires_approval: false,
};

export const mockDryRunInvalid: DryRunResult = {
  action_id: 'scale_deployment',
  action_name: '扩缩 Deployment',
  target_resource_id: 'deploy:not-exist',
  target_resource_type: null,
  target_resource_name: null,
  target_valid: false,
  validation_error: '目标资源不存在',
  affected_resources: [],
  affected_count: 0,
  estimated_duration_seconds: 0,
  estimated_sla_impact: 'minimal',
  warnings: [],
  rollback_action_id: null,
  rollback_input_params: null,
  risk_level: null,
  requires_approval: null,
};

export const mockExecutionSucceeded: RecoveryExecution = {
  execution_id: 'exec-001',
  action_id: 'scale_deployment',
  action_name: '扩缩 Deployment',
  target_resource_id: 'deploy:cce-prod-01:order:order-api',
  target_resource_type: 'Deployment',
  finding_id: null,
  input_params: { replicas_delta: 2 },
  status: 'succeeded',
  initiated_by: 'alice@example.com',
  request_reason: '业务低峰期扩容',
  initiated_at: '2026-06-19T10:00:00Z',
  executed_at: '2026-06-19T10:00:01Z',
  completed_at: '2026-06-19T10:00:31Z',
  result: { previous_replicas: 3, new_replicas: 5 },
  approval_id: null,
  rollback_execution_id: null,
  reverses_execution_id: null,
  dry_run_summary: {
    affected_count: 5,
    estimated_sla_impact: 'minimal',
    rollback_action_id: 'scale_deployment',
  },
};

export const mockExecutionFailed: RecoveryExecution = {
  ...mockExecutionSucceeded,
  execution_id: 'exec-002',
  action_id: 'restart_service',
  action_name: '重启 Service',
  target_resource_id: 'svc:cce-prod-01:order:order-api',
  target_resource_type: 'Service',
  status: 'failed',
  result: { error: 'connection refused' },
  initiated_by: 'system',
  request_reason: '',
  dry_run_summary: {
    affected_count: 0,
    estimated_sla_impact: 'minimal',
    rollback_action_id: null,    // restart_service 无回滚
  },
};

export const mockExecutionAwaitingApproval: RecoveryExecution = {
  ...mockExecutionSucceeded,
  execution_id: 'exec-003',
  action_id: 'rollback_deployment',
  action_name: '回滚 Deployment',
  status: 'awaiting_approval',
  approval_id: 'approval-001',
  result: {},
  completed_at: '',
  executed_at: '',
};

export const mockApprovalPending: ApprovalRequest = {
  approval_id: 'approval-001',
  execution_id: 'exec-003',
  requested_by: 'alice@example.com',
  requested_at: '2026-06-19T10:00:00Z',
  request_reason: 'v1.2.3 上线后告警增多',
  approver_id: '',
  approver_team: '订单团队',
  approval_status: 'pending',
  approved_at: '',
  approval_comment: '',
  expiry_at: '2026-06-20T10:00:00Z',
  execution_summary: {
    action_id: 'rollback_deployment',
    action_name: '回滚 Deployment',
    target_resource_id: 'deploy:cce-prod-01:order:order-api',
    target_resource_type: 'Deployment',
    status: 'awaiting_approval',
    dry_run_summary: { affected_count: 5, estimated_sla_impact: 'medium' },
  },
};

export const mockApprovalApproved: ApprovalRequest = {
  ...mockApprovalPending,
  approval_id: 'approval-002',
  approval_status: 'approved',
  approver_id: 'bob@example.com',
  approved_at: '2026-06-19T10:05:00Z',
  approval_comment: '业务侧已知会',
};
