/**
 * Phase 3.6 - Tauri invoke 封装 + TS 类型(对齐 Rust command DTO)。
 *
 * 移植自 reference `frontend/src/api/client.ts`,把 axios HTTP 调用换成
 * `@tauri-apps/api/core` 的 `invoke`(三层契约 B 层 - webview ↔ Rust JSON IPC)。
 *
 * 约定:
 * - **请求参数**:invoke 第二参用 camelCase 键(Tauri v2 自动转 snake_case Rust 形参);
 *   结构体入参(record_change_event / record_alert 的 `req`)用 snake_case 键对齐
 *   Rust struct serde 字段名。
 * - **响应字段**:snake_case(Rust serde 默认,与 engine 类型一致)。
 */
import { invoke } from "@tauri-apps/api/core";
// 图相关类型复用 TopologyView(它已是 engine_core::GraphResponse 的 serde 镜像,
// 含 `type` rename)。避免重复定义漂移。
import type { FactDto, GraphResponse } from "../views/TopologyView";
export type { FactDto, GraphResponse } from "../views/TopologyView";

// ===== 通用 =====

export async function getAppVersion(): Promise<string> {
  return invoke<string>("get_app_version");
}

export interface ConnectorInfo {
  name: string;
  version: string;
  kind: string;
  sync_interval_seconds: number;
  capabilities: string[];
}
export interface ConnectorStatusDto {
  name: string;
  fact_count: number;
  errors: string[];
}
export interface ChangeSummaryDto {
  nodes_upserted: number;
  nodes_removed: number;
  edges_upserted: number;
  edges_removed: number;
}
export interface SyncSummaryDto {
  facts: FactDto[];
  per_connector: ConnectorStatusDto[];
  total_errors: number;
  total_duration_ms: number;
  changes: ChangeSummaryDto;
}
export async function listConnectors(): Promise<ConnectorInfo[]> {
  return invoke<ConnectorInfo[]>("list_connectors");
}
export async function syncAllNow(configJson = "{}"): Promise<SyncSummaryDto> {
  return invoke<SyncSummaryDto>("sync_all_now", { configJson });
}
export async function getTopology(): Promise<FactDto[]> {
  return invoke<FactDto[]>("get_topology");
}
export async function getGraph(): Promise<GraphResponse> {
  return invoke<GraphResponse>("get_graph");
}
export interface ProxyStatusDto {
  running: boolean;
  port: number;
  api_base: string;
  pid: number | null;
  message: string;
}
export async function startKubectlProxy(port = 8001): Promise<ProxyStatusDto> {
  return invoke<ProxyStatusDto>("start_kubectl_proxy", { port });
}
export async function stopKubectlProxy(): Promise<ProxyStatusDto> {
  return invoke<ProxyStatusDto>("stop_kubectl_proxy");
}
export async function proxyStatus(): Promise<ProxyStatusDto> {
  return invoke<ProxyStatusDto>("proxy_status");
}

// ===== Recovery (PRD-001) =====

export type RiskLevel = "low" | "medium" | "high";
export type RecoveryStatus =
  | "pending" | "dry_run_ok" | "awaiting_approval" | "approved" | "rejected"
  | "executing" | "succeeded" | "failed" | "rolled_back";
export type VerifyStatus = "not_run" | "passed" | "failed" | "skipped" | "not_supported" | "timeout" | "error";
export type ChainStatus =
  | "pending" | "awaiting_approval" | "executing" | "succeeded"
  | "partial" | "failed" | "rolled_back" | "aborted";
export type OnFailureStrategy = "stop" | "rollback_all" | "continue";

export interface ParamSpec {
  name: string;
  kind: "boolean" | "integer" | "string";
  required: boolean;
}
export interface ActionDef {
  action_id: string;
  name: string;
  category: string;
  target_type: string;
  risk_level: RiskLevel;
  requires_approval: boolean;
  rollback_action_id: string | null;
  description: string;
  input_schema: ParamSpec[];
}
export interface AffectedResource {
  resource_id: string;
  type: string;
  name: string;
  impact_severity: "minimal" | "low" | "medium" | "high";
  via_relations: string[];
  notes: string[];
}
export interface DryRunResult {
  action_id: string;
  action_name: string | null;
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
  rollback_input_params: unknown | null;
  risk_level: RiskLevel | null;
  requires_approval: boolean | null;
}
export interface ActionSuggestion {
  action: ActionDef;
  rationale: string;
  confidence: number;
}
export interface RecoveryExecution {
  execution_id: string;
  action_id: string;
  target_resource_id: string;
  target_resource_type: string;
  finding_id: string | null;
  input_params: Record<string, unknown>;
  dry_run_result: DryRunResult;
  status: RecoveryStatus;
  initiated_by: string;
  request_reason: string;
  initiated_at: string;
  executed_at: string;
  completed_at: string;
  result: Record<string, unknown>;
  rollback_execution_id: string | null;
  reverses_execution_id: string | null;
  cluster_id: string;
  verify_status: VerifyStatus;
  verify_result: Record<string, unknown>;
  verified_at: string;
  chain_id: string;
  chain_step_index: number;
  approval_comment: string;
  approved_at: string;
}
export interface ChainStepDto {
  action_id: string;
  params: Record<string, unknown>;
  verify_required: boolean;
}
export interface ChainTemplateDto {
  template_id: string;
  name: string;
  description: string;
  target_type: string;
  on_failure: OnFailureStrategy;
  steps: ChainStepDto[];
}
export interface RecoveryChain {
  chain_id: string;
  template_id: string;
  target_resource_id: string;
  status: ChainStatus;
  on_failure: OnFailureStrategy;
  step_executions: string[];
  current_step_index: number;
  total_steps: number;
  initiated_by: string;
  request_reason: string;
  initiated_at: string;
  completed_at: string;
  approval_id: string;
  failure_reason: string;
  template_name: string;
  approval_comment: string;
  approved_at: string;
}

export async function listRecoveryActions(opts?: {
  targetType?: string; category?: string; riskLevel?: RiskLevel;
}): Promise<ActionDef[]> {
  return invoke<ActionDef[]>("list_recovery_actions", {
    targetType: opts?.targetType, category: opts?.category, riskLevel: opts?.riskLevel,
  });
}
export async function getRecoveryAction(actionId: string): Promise<ActionDef> {
  return invoke<ActionDef>("get_recovery_action", { actionId });
}
export async function dryRunRecovery(actionId: string, targetResourceId: string, inputParams: Record<string, unknown>): Promise<DryRunResult> {
  return invoke<DryRunResult>("dry_run_recovery", { actionId, targetResourceId, inputParams });
}
export async function recoverySuggestionsForRule(ruleId: string): Promise<ActionSuggestion[]> {
  return invoke<ActionSuggestion[]>("recovery_suggestions_for_rule", { ruleId });
}
export async function executeRecovery(opts: {
  actionId: string; targetResourceId: string; inputParams: Record<string, unknown>;
  initiatedBy?: string; requestReason?: string; findingId?: string;
}): Promise<RecoveryExecution> {
  return invoke<RecoveryExecution>("execute_recovery", opts);
}
export async function listRecoveryExecutions(opts?: {
  status?: RecoveryStatus; actionId?: string; targetResourceId?: string; limit?: number;
}): Promise<RecoveryExecution[]> {
  return invoke<RecoveryExecution[]>("list_recovery_executions", opts ?? {});
}
export async function getRecoveryExecution(executionId: string): Promise<RecoveryExecution> {
  return invoke<RecoveryExecution>("get_recovery_execution", { executionId });
}
export async function confirmRecoveryExecution(executionId: string, approvalComment?: string): Promise<RecoveryExecution> {
  return invoke<RecoveryExecution>("confirm_recovery_execution", { executionId, approvalComment });
}
export async function cancelRecoveryExecution(executionId: string): Promise<RecoveryExecution> {
  return invoke<RecoveryExecution>("cancel_recovery_execution", { executionId });
}
export async function rollbackRecoveryExecution(executionId: string, initiatedBy?: string, reason?: string): Promise<RecoveryExecution> {
  return invoke<RecoveryExecution>("rollback_recovery_execution", { executionId, initiatedBy, reason });
}
export async function reverifyRecoveryExecution(executionId: string): Promise<RecoveryExecution> {
  return invoke<RecoveryExecution>("reverify_recovery_execution", { executionId });
}
export async function listChainTemplates(): Promise<ChainTemplateDto[]> {
  return invoke<ChainTemplateDto[]>("list_chain_templates");
}
export async function getChainTemplate(templateId: string): Promise<ChainTemplateDto> {
  return invoke<ChainTemplateDto>("get_chain_template", { templateId });
}
export async function executeChain(opts: {
  templateId: string; targetResourceId: string; initiatedBy?: string;
  requestReason?: string; onFailureOverride?: OnFailureStrategy;
}): Promise<RecoveryChain> {
  return invoke<RecoveryChain>("execute_chain", opts);
}
export async function confirmChain(chainId: string, approvalComment?: string): Promise<RecoveryChain> {
  return invoke<RecoveryChain>("confirm_chain", { chainId, approvalComment });
}
export async function cancelChain(chainId: string): Promise<RecoveryChain> {
  return invoke<RecoveryChain>("cancel_chain", { chainId });
}
export async function abortChain(chainId: string, reason?: string): Promise<RecoveryChain> {
  return invoke<RecoveryChain>("abort_chain", { chainId, reason });
}
export async function listRecoveryChains(opts?: { status?: ChainStatus; limit?: number }): Promise<RecoveryChain[]> {
  return invoke<RecoveryChain[]>("list_recovery_chains", opts ?? {});
}
export async function getRecoveryChain(chainId: string): Promise<RecoveryChain> {
  return invoke<RecoveryChain>("get_recovery_chain", { chainId });
}

// ===== Change events (PRD-002) =====

export type ChangeType = "configmap_updated" | "secret_rotated" | "deployment_rolled" | "image_pushed";
export type Source = "k8s_api" | "argo_cd" | "gitops" | "manual" | "unknown" | "flagd";
export type Severity = "low" | "medium" | "high";

export interface ChangeEvent {
  change_event_id: string;
  change_type: ChangeType;
  target_resource_id: string;
  target_resource_type: string;
  changed_at: string;
  changed_by: string;
  source: Source;
  description: string;
  diff_summary: Record<string, unknown>;
  related_commit: string;
  related_pr: string;
  severity_estimate: Severity;
  propagated_to: string[];
  commit_sha: string;
  pipeline_url: string;
  git_repo: string;
  cluster_id: string;
  yaml_diff: string;
  propagated_count?: number;
}
export interface ChangeEventInput {
  change_type: string;
  target_resource_id: string;
  changed_by?: string;
  source?: string;
  description?: string;
  diff_summary?: Record<string, unknown>;
  related_commit?: string;
  related_pr?: string;
  changed_at?: string;
  commit_sha?: string;
  pipeline_url?: string;
  git_repo?: string;
  cluster_id?: string;
  yaml_diff?: string;
}
export interface CorrelatedChange {
  [k: string]: unknown;
  match_type: "direct" | "propagated";
  propagation_distance: number;
}
export interface CorrelatedResult {
  target_resource_id: string;
  window_start: string;
  window_end: string;
  now: string;
  include_propagated: boolean;
  changes: CorrelatedChange[];
  total: number;
}
export interface FrequentTarget {
  target_resource_id: string;
  count: number;
  event_ids: string[];
  is_frequent: boolean;
}
export interface FrequentChangesResponse {
  frequent: FrequentTarget[];
  window_seconds: number;
  threshold: number;
}
export interface ChangeEventImpactResponse {
  change_event_id: string;
  target_resource_id: string;
  target_resource_type: string;
  affected: string[];
  affected_count: number;
  severity_estimate: Severity;
}
export type TargetMatch = "direct" | "propagated" | "unresolved";
export interface RecoverySuggestion {
  action_id: string;
  action_name: string;
  rationale: string;
  confidence: number;
  risk_level: RiskLevel;
  requires_approval: boolean;
  target_type: string;
  resolved_target_resource_id: string | null;
  resolved_target_type: string;
  target_match: TargetMatch;
}
export interface RecoverySuggestionResult {
  change_event_id: string;
  change_type: ChangeType;
  target_resource_id: string;
  target_resource_type: string;
  suggestions: RecoverySuggestion[];
  total: number;
}
export interface CorrelateAlertsResult {
  change_event_id: string;
  changed_at: string;
  window_start: string;
  window_end: string;
  affected_resource_ids: string[];
  alerts: AlertEvent[];
  total: number;
  neo4j_available: boolean;
}

export async function recordChangeEvent(req: ChangeEventInput): Promise<ChangeEvent> {
  return invoke<ChangeEvent>("record_change_event", { req });
}
export async function listChangeEvents(opts?: {
  changeType?: ChangeType; targetResourceId?: string; source?: Source;
  since?: string; until?: string; limit?: number;
}): Promise<ChangeEvent[]> {
  return invoke<ChangeEvent[]>("list_change_events", opts ?? {});
}
export async function getChangeEvent(changeEventId: string): Promise<ChangeEvent> {
  return invoke<ChangeEvent>("get_change_event", { changeEventId });
}
export async function correlatedChanges(opts: {
  targetResourceId: string; window?: number; since?: string; until?: string;
  includePropagated?: boolean;
}): Promise<CorrelatedResult> {
  return invoke<CorrelatedResult>("correlated_changes", opts);
}
export async function frequentChanges(opts?: { window?: number; threshold?: number }): Promise<FrequentChangesResponse> {
  return invoke<FrequentChangesResponse>("frequent_changes", opts ?? {});
}
export async function changeEventImpact(changeEventId: string): Promise<ChangeEventImpactResponse> {
  return invoke<ChangeEventImpactResponse>("change_event_impact", { changeEventId });
}
export async function changeEventRecoverySuggestion(changeEventId: string): Promise<RecoverySuggestionResult> {
  return invoke<RecoverySuggestionResult>("change_event_recovery_suggestion", { changeEventId });
}
export async function changeEventAlerts(changeEventId: string, window?: number): Promise<CorrelateAlertsResult> {
  return invoke<CorrelateAlertsResult>("change_event_alerts", { changeEventId, window });
}

// ===== Alerts =====

export type AlertSeverity = "warning" | "critical";
export type AlertStatus = "firing" | "resolved";
export interface AlertEvent {
  alert_event_id: string;
  alert_name: string;
  severity: AlertSeverity;
  status: AlertStatus;
  fired_at: string;
  resource_ref: string;
  rule_id: string;
  metric_name: string;
  metric_value: number;
  summary: string;
  description: string;
  cluster_id: string;
  resolved_at: string;
}
export interface AlertInput {
  alert_name: string;
  alert_event_id?: string;
  resource_ref: string;
  severity?: string;
  status?: string;
  fired_at?: string;
  rule_id?: string;
  metric_name?: string;
  metric_value?: number;
  summary?: string;
  description?: string;
  cluster_id?: string;
}
export interface CorrelateChangesForAlertResult {
  alert_event_id: string;
  resource_ref: string;
  fired_at: string;
  changes: ChangeEvent[];
  total: number;
  neo4j_available: boolean;
}

export async function recordAlert(req: AlertInput): Promise<AlertEvent> {
  return invoke<AlertEvent>("record_alert", { req });
}
export async function listAlerts(opts?: {
  resourceRef?: string; severity?: AlertSeverity; status?: AlertStatus;
  since?: string; until?: string; limit?: number;
}): Promise<AlertEvent[]> {
  return invoke<AlertEvent[]>("list_alerts", opts ?? {});
}
export async function getAlert(alertEventId: string): Promise<AlertEvent> {
  return invoke<AlertEvent>("get_alert", { alertEventId });
}
export async function resolveAlert(alertEventId: string): Promise<AlertEvent> {
  return invoke<AlertEvent>("resolve_alert", { alertEventId });
}
export async function correlateChangesForAlert(opts: {
  alertEventId: string; window?: number; resourceRef?: string;
}): Promise<CorrelateChangesForAlertResult> {
  return invoke<CorrelateChangesForAlertResult>("correlate_changes_for_alert", opts);
}

// ===== Phase 4.1/4.3 - reports / subscriptions (PRD-003) =====

export type ReportTemplate = "application_health" | "cluster_overview" | "incident_report";
export type ReportStatus = "pending" | "generating" | "completed" | "failed";
export type SubscriptionStatus = "never" | "ok" | "failed";

export interface ReportScope {
  application_id?: string | null;
  cluster_id?: string | null;
  change_event_id?: string | null;
  fault_id?: string | null;
  time_range_start?: string | null;
  time_range_end?: string | null;
}
export type TriggerSource = "manual_cmd" | "scheduled" | "trigger_now";
export interface ReportTask {
  report_id: string;
  template_id: ReportTemplate;
  scope: ReportScope;
  modules: string[];
  format: string;
  status: ReportStatus;
  progress: number;
  current_step: string;
  error_message: string | null;
  markdown: string | null;
  created_at: string;
  completed_at: string | null;
  trigger_source: TriggerSource;
}
export interface ReportSubscription {
  subscription_id: string;
  template_id: ReportTemplate;
  scope: ReportScope;
  modules: string[];
  cron: string;
  recipients: string[];
  enabled: boolean;
  created_at: string;
  last_run_at: string;
  last_status: SubscriptionStatus;
  last_error: string;
  last_report_id: string;
}
export interface SentEmail {
  recipients: string[];
  subject: string;
  body: string;
  attachment_filename: string;
  attachment_content: string;
}

export async function generateReport(opts: {
  templateId: string;
  applicationId?: string;
  clusterId?: string;
  changeEventId?: string;
  faultId?: string;
  modules?: string[];
}): Promise<ReportTask> {
  return invoke<ReportTask>("generate_report_cmd", opts);
}
export async function listReports(opts?: {
  templateId?: string;
  applicationId?: string;
}): Promise<ReportTask[]> {
  return invoke<ReportTask[]>("list_reports", opts ?? {});
}
export async function getReport(reportId: string): Promise<ReportTask> {
  return invoke<ReportTask>("get_report", { reportId });
}
export async function clearReports(): Promise<number> {
  return invoke<number>("clear_reports");
}

export async function createSubscription(opts: {
  templateId: string;
  applicationId?: string;
  clusterId?: string;
  changeEventId?: string;
  faultId?: string;
  modules?: string[];
  cron: string;
  recipients: string[];
  enabled?: boolean;
}): Promise<ReportSubscription> {
  return invoke<ReportSubscription>("create_subscription", opts);
}
export async function listSubscriptions(opts?: {
  templateId?: string;
}): Promise<ReportSubscription[]> {
  return invoke<ReportSubscription[]>("list_subscriptions", opts ?? {});
}
export async function getSubscription(subscriptionId: string): Promise<ReportSubscription> {
  return invoke<ReportSubscription>("get_subscription", { subscriptionId });
}
export async function updateSubscription(opts: {
  subscriptionId: string;
  cron?: string;
  recipients?: string[];
  enabled?: boolean;
  modules?: string[];
}): Promise<ReportSubscription> {
  return invoke<ReportSubscription>("update_subscription", opts);
}
export async function deleteSubscription(subscriptionId: string): Promise<boolean> {
  return invoke<boolean>("delete_subscription", { subscriptionId });
}
export async function triggerSubscriptionNow(subscriptionId: string): Promise<ReportTask> {
  return invoke<ReportTask>("trigger_subscription_now", { subscriptionId });
}
export async function listSentEmails(): Promise<SentEmail[]> {
  return invoke<SentEmail[]>("list_sent_emails");
}
