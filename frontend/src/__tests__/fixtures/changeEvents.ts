/**
 * Test fixtures for ChangeEvent components — PRD-002 Sprint 2.
 *
 * 字段镜像后端 backend/app/changes/event_service.py:_serialize_event() —
 * 与契约保持一致。
 */
import type {
  ChangeEvent,
  ChangeEventListResponse,
  ChangeEventTimelineResponse,
  ChangeEventImpactResponse,
  ChangeRecoverySuggestionResponse,
} from '../../api/client';

export const mockEventLow: ChangeEvent = {
  change_event_id: 'ce-aaa11122233',
  change_type: 'configmap_updated',
  target_resource_id: 'cm:order:order-config',
  target_resource_type: 'ConfigMap',
  changed_at: '2026-06-19T03:55:30Z',
  changed_by: 'alice@e2e',
  source: 'manual',
  description: '池大小 20 → 50',
  diff_summary: { max_pool_size: { old: 20, new: 50 } },
  related_commit: '',
  related_pr: '',
  severity_estimate: 'low',
  propagated_to: [],
  propagated_count: 0,
  commit_sha: '',
  pipeline_url: '',
  git_repo: '',
  cluster_id: '',
  yaml_diff: '',
};

export const mockEventMedium: ChangeEvent = {
  change_event_id: 'ce-bbb44455566',
  change_type: 'secret_rotated',
  target_resource_id: 'secret:order:order-secret',
  target_resource_type: 'Secret',
  changed_at: '2026-06-19T02:30:00Z',
  changed_by: 'ci-bot',
  source: 'gitops',
  description: 'rotate db password',
  diff_summary: {},
  related_commit: 'abc123',
  related_pr: '',
  severity_estimate: 'medium',
  propagated_to: ['pod:order:order-api-1', 'pod:order:order-api-2', 'pod:order:order-api-3', 'pod:order:order-api-4', 'pod:order:order-api-5'],
  propagated_count: 5,
  commit_sha: 'abc123def456',
  pipeline_url: '',
  git_repo: 'https://github.com/acme/order-api',
  cluster_id: '',
  yaml_diff: '',
};

export const mockEventHigh: ChangeEvent = {
  change_event_id: 'ce-ccc77788899',
  change_type: 'deployment_rolled',
  target_resource_id: 'deploy:order:order-api',
  target_resource_type: 'Deployment',
  changed_at: '2026-06-19T01:00:00Z',
  changed_by: 'argo-cd',
  source: 'argo_cd',
  description: 'rollout v1.2.4',
  diff_summary: { image: { old: 'order-api:1.2.3', new: 'order-api:1.2.4' } },
  related_commit: 'def456',
  related_pr: 'PR-123',
  severity_estimate: 'high',
  propagated_to: Array.from({ length: 12 }, (_, i) => `pod:order:p-${i}`),
  propagated_count: 12,
  commit_sha: 'def456789abc',
  pipeline_url: 'https://ci.example.com/run/42',
  git_repo: 'https://github.com/acme/order-api',
  cluster_id: 'vm-cluster',
  yaml_diff: '--- order-api.old\n+++ order-api.new\n@@ -1 +1 @@\n-image: order-api:1.2.3\n+image: order-api:1.2.4',
};

export const mockEventListResponse: ChangeEventListResponse = {
  events: [mockEventLow, mockEventMedium, mockEventHigh],
  total: 3,
};

export const mockTimelineResponse: ChangeEventTimelineResponse = {
  application_id: 'app:order',
  since: '2026-06-18T00:00:00Z',
  until: null,
  resources_in_scope: 8,
  events: [mockEventLow, mockEventMedium, mockEventHigh],
  total: 3,
  by_type: {
    configmap_updated: 1,
    secret_rotated: 1,
    deployment_rolled: 1,
  },
};

export const mockImpactResponse: ChangeEventImpactResponse = {
  change_event_id: 'ce-bbb44455566',
  target_resource_id: 'secret:order:order-secret',
  target_resource_type: 'Secret',
  affected: [
    { resource_id: 'pod:order:order-api-1', resource_type: 'Pod', resource_name: 'order-api-1', path: ['secret:order:order-secret', 'pod:order:order-api-1'], distance: 1 },
    { resource_id: 'pod:order:order-api-2', resource_type: 'Pod', resource_name: 'order-api-2', path: ['secret:order:order-secret', 'pod:order:order-api-2'], distance: 1 },
  ],
  affected_count: 2,
  severity_estimate: 'medium',
};

export const mockEmptyListResponse: ChangeEventListResponse = {
  events: [],
  total: 0,
};

export const mockEmptyTimelineResponse: ChangeEventTimelineResponse = {
  application_id: 'app:order',
  since: null,
  until: null,
  resources_in_scope: 0,
  events: [],
  total: 0,
  by_type: {},
};

// PRD-002 Phase 2 — 变更 → 恢复动作推荐(集成 PRD-001)
// 镜像后端 event_service.get_recovery_suggestion() 返回结构

export const mockSuggestionDirect: ChangeRecoverySuggestionResponse = {
  change_event_id: 'ce-ccc77788899',
  change_type: 'deployment_rolled',
  target_resource_id: 'deploy:order:order-api',
  target_resource_type: 'Deployment',
  suggestions: [
    {
      action_id: 'rollback_deployment',
      action_name: '回滚 Deployment',
      rationale: '新版本异常 → 回滚到上一 revision',
      confidence: 0.9,
      risk_level: 'high',
      requires_approval: true,
      target_type: 'Deployment',
      resolved_target_resource_id: 'deploy:order:order-api',
      resolved_target_type: 'Deployment',
      target_match: 'direct',
    },
  ],
  total: 1,
};

export const mockSuggestionUnresolved: ChangeRecoverySuggestionResponse = {
  change_event_id: 'ce-ccc77788899',
  change_type: 'image_pushed',
  target_resource_id: 'img:order:1.2.4',
  target_resource_type: 'ContainerImage',
  suggestions: [
    {
      action_id: 'rollback_deployment',
      action_name: '回滚 Deployment',
      rationale: '高危镜像 → 回滚到合规版本',
      confidence: 0.75,
      risk_level: 'high',
      requires_approval: true,
      target_type: 'Deployment',
      resolved_target_resource_id: null,
      resolved_target_type: '',
      target_match: 'unresolved',
    },
  ],
  total: 1,
};
