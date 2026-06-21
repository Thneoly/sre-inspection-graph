import axios from 'axios';

const api = axios.create({
  baseURL: import.meta.env.VITE_API_BASE_URL || '/api/v1',
  timeout: 15000,
});

// Graph types
export interface GraphNode {
  id: string;
  label: string;
  type: string;
  properties: Record<string, unknown>;
}

export interface GraphEdge {
  id: string;
  source: string;
  target: string;
  type: string;
  properties: Record<string, unknown>;
}

export interface GraphSummary {
  total_nodes: number;
  total_edges: number;
  risk_counts: Record<string, number>;
  health_counts: Record<string, number>;
}

export interface GraphResponse {
  nodes: GraphNode[];
  edges: GraphEdge[];
  summary: GraphSummary;
}

export interface MetricSnapshotOut {
  id: string;
  metric_name: string;
  current_value: number;
  unit: string;
  fetched_at: string;
  is_stale: boolean;
  warning_breached: boolean;
  critical_breached: boolean;
  warning_threshold: number | null;
  critical_threshold: number | null;
}

export interface ResourceMetricsResponse {
  resource_id: string;
  metrics: MetricSnapshotOut[];
}

// API functions
export function fetchTopology(appCode: string, depth = 5) {
  return api.get<GraphResponse>(`/topology/app/${appCode}`, { params: { depth } });
}

export function fetchAccessLink(appCode: string) {
  return api.get<GraphResponse>(`/access-link/${appCode}`);
}

export function fetchNodeImpact(nodeId: string, depth = 4) {
  return api.get<GraphResponse>(`/node-impact/${encodeURIComponent(nodeId)}`, { params: { depth } });
}

export function fetchConfigImpact(resourceId: string) {
  return api.get<GraphResponse>(`/config-impact/${encodeURIComponent(resourceId)}`);
}

export function fetchImageRisk(imageId: string) {
  return api.get<GraphResponse>(`/image-risk/${encodeURIComponent(imageId)}`);
}

export function fetchAlertAggregation(severity?: string) {
  return api.get<GraphResponse>('/alert-aggregation', { params: { severity } });
}

export function fetchResourceMetrics(resourceId: string) {
  return api.get<ResourceMetricsResponse>(`/metrics/${encodeURIComponent(resourceId)}`);
}


// ============================================================
// Recovery Action types & API (PRD-001)
// ============================================================

export type RiskLevel = 'low' | 'medium' | 'high';
export type ExecutionStatus =
  | 'pending'
  | 'dry_run_ok'
  | 'awaiting_approval'
  | 'approved'
  | 'rejected'
  | 'executing'
  | 'succeeded'
  | 'failed'
  | 'rolled_back';

export interface RecoveryAction {
  action_id: string;
  action_name: string;
  action_category: string;
  target_resource_type: string;
  risk_level: RiskLevel;
  requires_approval: boolean;
  rollback_action_id: string | null;
  estimated_duration_seconds: number;
  description: string;
  input_schema: Record<string, unknown>;
  sla_impact_estimate: string;
  warnings: string[];
}

export interface RecoverySuggestion {
  action_id: string;
  action_name: string;
  rationale: string;
  confidence: number;
  risk_level: RiskLevel;
  requires_approval: boolean;
  target_resource_type: string;
}

export interface AffectedResource {
  resource_id: string;
  type: string;
  name: string;
  impact_severity: 'minimal' | 'low' | 'medium' | 'high';
  via_relations: string[];
  notes: string[];
}

export interface DryRunResult {
  action_id: string;
  action_name: string;
  target_resource_id: string;
  target_resource_type: string | null;
  target_resource_name: string | null;
  target_valid: boolean;
  validation_error: string | null;
  affected_resources: AffectedResource[];
  affected_count: number;
  estimated_duration_seconds: number;
  estimated_sla_impact: string;
  warnings: string[];
  rollback_action_id: string | null;
  rollback_input_params: Record<string, unknown> | null;
  risk_level: RiskLevel | null;
  requires_approval: boolean | null;
  finding_id?: string;
}

export interface RecoveryExecution {
  execution_id: string;
  action_id: string;
  action_name: string;
  target_resource_id: string;
  target_resource_type: string;
  finding_id: string | null;
  input_params: Record<string, unknown>;
  status: ExecutionStatus;
  initiated_by: string;
  request_reason: string;
  initiated_at: string;
  executed_at: string;
  completed_at: string;
  result: Record<string, unknown>;
  approval_id: string | null;
  rollback_execution_id: string | null;
  reverses_execution_id: string | null;
  dry_run_summary: {
    affected_count: number;
    estimated_sla_impact: string | null;
    rollback_action_id?: string | null;
  } | null;
}

export type ApprovalStatus = 'pending' | 'approved' | 'rejected' | 'expired';

export interface ApprovalRequest {
  approval_id: string;
  execution_id: string;
  requested_by: string;
  requested_at: string;
  request_reason: string;
  approver_id: string;
  approver_team: string;
  approval_status: ApprovalStatus;
  approved_at: string;
  approval_comment: string;
  expiry_at: string;
  execution_summary: {
    action_id: string;
    action_name: string;
    target_resource_id: string;
    target_resource_type: string;
    status: ExecutionStatus;
    dry_run_summary: { affected_count: number; estimated_sla_impact: string | null } | null;
  } | null;
}

// Endpoints

export function fetchRecoveryActions(params?: {
  target_type?: string;
  category?: string;
  risk_level?: RiskLevel;
}) {
  return api.get<{ actions: RecoveryAction[]; total: number }>('/recovery/actions', { params });
}

export function fetchRecoveryAction(actionId: string) {
  return api.get<RecoveryAction>(`/recovery/actions/${encodeURIComponent(actionId)}`);
}

export function fetchRecoverySuggestions(ruleId: string) {
  return api.get<{ rule_id: string; suggestions: RecoverySuggestion[]; total: number }>(
    '/recovery/suggestions',
    { params: { rule_id: ruleId } },
  );
}

export function postRecoveryDryRun(req: {
  action_id: string;
  target_resource_id: string;
  input_params?: Record<string, unknown>;
  finding_id?: string;
}) {
  return api.post<DryRunResult>('/recovery/dry-run', req);
}

export function postRecoveryExecute(req: {
  action_id: string;
  target_resource_id: string;
  input_params?: Record<string, unknown>;
  finding_id?: string;
  initiated_by?: string;
  request_reason?: string;
}) {
  return api.post<RecoveryExecution>('/recovery/execute', req);
}

export function fetchRecoveryExecutions(params?: {
  status?: ExecutionStatus;
  action_id?: string;
  target_resource_id?: string;
  limit?: number;
}) {
  return api.get<{ executions: RecoveryExecution[]; total: number }>('/recovery/executions', {
    params,
  });
}

export function fetchRecoveryExecution(executionId: string) {
  return api.get<RecoveryExecution>(
    `/recovery/executions/${encodeURIComponent(executionId)}`,
  );
}

// Sprint 3 — Approval flow + rollback

export function fetchApprovals(params?: { status?: ApprovalStatus }) {
  return api.get<{ approvals: ApprovalRequest[]; total: number }>('/recovery/approvals', {
    params,
  });
}

export function fetchApproval(approvalId: string) {
  return api.get<ApprovalRequest>(`/recovery/approvals/${encodeURIComponent(approvalId)}`);
}

export function postApprovalApprove(req: {
  approval_id: string;
  approver_id: string;
  comment?: string;
}) {
  return api.post<{ approval: ApprovalRequest; execution: RecoveryExecution }>(
    `/recovery/approvals/${encodeURIComponent(req.approval_id)}/approve`,
    { approver_id: req.approver_id, comment: req.comment ?? '' },
  );
}

export function postApprovalReject(req: {
  approval_id: string;
  approver_id: string;
  comment?: string;
}) {
  return api.post<{ approval: ApprovalRequest; execution: RecoveryExecution | null }>(
    `/recovery/approvals/${encodeURIComponent(req.approval_id)}/reject`,
    { approver_id: req.approver_id, comment: req.comment ?? '' },
  );
}

export function postExecutionRollback(req: {
  execution_id: string;
  initiated_by?: string;
  reason?: string;
}) {
  return api.post<RecoveryExecution>(
    `/recovery/executions/${encodeURIComponent(req.execution_id)}/rollback`,
    { initiated_by: req.initiated_by ?? 'web-ui', reason: req.reason ?? '' },
  );
}


// ============================================================
// PRD-002 — ChangeEvent
// ============================================================
// 字段对标后端 backend/app/changes/event_service.py:_serialize_event()
// API path 都是 /change-events/*(api 实例 baseURL = /api/v1)。

export type ChangeType =
  | 'configmap_updated'
  | 'secret_rotated'
  | 'deployment_rolled'
  | 'image_pushed';

export type ChangeSource =
  | 'k8s_api'
  | 'argo_cd'
  | 'gitops'
  | 'manual'
  | 'unknown'
  | 'flagd';

export type ChangeSeverity = 'low' | 'medium' | 'high';

export type CorrelationMatchType = 'direct' | 'propagated';

export interface ChangeEvent {
  change_event_id: string;
  change_type: ChangeType;
  target_resource_id: string;
  target_resource_type: string;
  changed_at: string;          // ISO8601
  changed_by: string;
  source: ChangeSource;
  description: string;
  diff_summary: Record<string, unknown>;
  related_commit: string;
  related_pr: string;
  severity_estimate: ChangeSeverity;
  propagated_to: string[];
  propagated_count: number;
}

export interface CorrelatedChange extends ChangeEvent {
  match_type: CorrelationMatchType;
  propagation_distance: number;
}

export interface ChangeEventListResponse {
  events: ChangeEvent[];
  total: number;
}

export interface ChangeEventTimelineResponse {
  application_id: string;
  since: string | null;
  until: string | null;
  resources_in_scope: number;
  events: ChangeEvent[];
  total: number;
  by_type: Record<string, number>;
}

export interface ChangeEventCorrelatedResponse {
  target_resource_id: string;
  window_start: string;
  window_end: string;
  now: string;
  include_propagated: boolean;
  changes: CorrelatedChange[];
  total: number;
}

export interface ChangeAffectedResource {
  resource_id: string;
  resource_type: string;
  resource_name: string;
  path: string[];
  distance: number;
}

export interface ChangeEventImpactResponse {
  change_event_id: string;
  target_resource_id: string;
  target_resource_type: string;
  affected: ChangeAffectedResource[];
  affected_count: number;
  severity_estimate: ChangeSeverity;
}


export function fetchChangeEvents(params?: {
  change_type?: ChangeType;
  target_resource_id?: string;
  source?: ChangeSource;
  since?: string;
  until?: string;
  limit?: number;
}) {
  return api.get<ChangeEventListResponse>('/change-events', { params });
}

export function fetchChangeEvent(changeEventId: string) {
  return api.get<ChangeEvent>(
    `/change-events/${encodeURIComponent(changeEventId)}`,
  );
}

export function fetchCorrelatedChanges(params: {
  target_resource_id: string;
  window?: number;
  since?: string;
  until?: string;
  include_propagated?: boolean;
}) {
  return api.get<ChangeEventCorrelatedResponse>(
    '/change-events/correlated',
    { params },
  );
}

export function fetchChangeEventTimeline(
  application_id: string,
  since?: string,
  until?: string,
) {
  return api.get<ChangeEventTimelineResponse>(
    '/change-events/timeline',
    { params: { application_id, since, until } },
  );
}

export function fetchChangeEventImpact(changeEventId: string) {
  return api.get<ChangeEventImpactResponse>(
    `/change-events/${encodeURIComponent(changeEventId)}/impact`,
  );
}

// PRD-002 Phase 2 — 变更 → 恢复动作推荐(集成 PRD-001)

export type ChangeRecoveryTargetMatch = 'direct' | 'propagated' | 'unresolved';

export interface ChangeRecoverySuggestionItem {
  action_id: string;
  action_name: string;
  rationale: string;
  confidence: number;
  risk_level: RiskLevel | null;
  requires_approval: boolean;
  target_type: string;
  resolved_target_resource_id: string | null;
  resolved_target_type: string;
  target_match: ChangeRecoveryTargetMatch;
}

export interface ChangeRecoverySuggestionResponse {
  change_event_id: string;
  change_type: ChangeType;
  target_resource_id: string;
  target_resource_type: string;
  suggestions: ChangeRecoverySuggestionItem[];
  total: number;
}

export function fetchChangeEventRecoverySuggestion(changeEventId: string) {
  return api.get<ChangeRecoverySuggestionResponse>(
    `/change-events/${encodeURIComponent(changeEventId)}/recovery-suggestion`,
  );
}


// ============================================================
// PRD-003 — 自检报告生成(Sprint 1: Markdown)
// ============================================================

export type ReportTemplate = 'application_health';
export type ReportStatus = 'pending' | 'generating' | 'completed' | 'failed';
export type ReportModule =
  | 'health_score'
  | 'seven_views'
  | 'risk_list'
  | 'recommended_actions'
  | 'historical_trends';

export interface ReportScope {
  application_id?: string;
  cluster_id?: string;
  time_range_start?: string;
  time_range_end?: string;
}

export interface ReportTask {
  report_id: string;
  template_id: ReportTemplate;
  scope: ReportScope;
  modules: ReportModule[];
  format: string;
  status: ReportStatus;
  progress: number;
  current_step: string;
  error_message: string | null;
  has_markdown: boolean;
  file_path: string | null;
  created_at: string;
  completed_at: string;
}

export interface ReportStatusResponse {
  report_id: string;
  status: ReportStatus;
  progress: number;
  current_step: string;
  error_message: string | null;
}

export const ALL_REPORT_MODULES: ReportModule[] = [
  'health_score',
  'seven_views',
  'risk_list',
  'recommended_actions',
  'historical_trends',
];

export function postReportGenerate(req: {
  template_id: ReportTemplate;
  scope: ReportScope;
  format?: string;
  modules?: ReportModule[];
}) {
  return api.post<{ report_id: string; status: ReportStatus; estimated_completion_seconds: number }>(
    '/reports/generate',
    req,
  );
}

export function fetchReportStatus(reportId: string) {
  return api.get<ReportStatusResponse>(`/reports/${encodeURIComponent(reportId)}/status`);
}

export function fetchReports(params?: {
  template_id?: ReportTemplate;
  application_id?: string;
  limit?: number;
}) {
  return api.get<{ reports: ReportTask[]; total: number; returned: number }>('/reports', {
    params,
  });
}

/** 下载报告 Markdown(blob)。全项目首个 blob 下载。 */
export function downloadReport(reportId: string) {
  return api.get<Blob>(`/reports/${encodeURIComponent(reportId)}/download`, {
    responseType: 'blob',
    params: { format: 'markdown' },
  });
}


export default api;
