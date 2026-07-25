import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  Card, Table, Tag, Button, Drawer, Descriptions, Modal, Form, Select, Input, message,
  Alert, Space, Tabs, List,
} from "antd";
import {
  listChangeEvents, recordChangeEvent, frequentChanges, changeEventImpact,
  changeEventAlerts, changeEventRecoverySuggestion, executeRecovery, getGraph,
  type ChangeEvent, type ChangeType,
} from "../../api/client";

const SEV_COLOR: Record<string, string> = { high: "red", medium: "orange", low: "green" };
const PRE: React.CSSProperties = { background: "#0d1117", color: "#c9d1d9", padding: "0.5rem 0.75rem", borderRadius: 4, fontSize: "0.8rem", overflow: "auto", maxHeight: 200 };

/**
 * 变更事件列表 + Drawer(impact/alerts/suggestion)+ 记录表单 + 过频告警 banner。
 * 移植自 reference `ChangeTimelineView.tsx`(axios -> invoke,精简)。
 */
export default function ChangeTimelineView() {
  const qc = useQueryClient();
  const { data: events } = useQuery({ queryKey: ["change-events"], queryFn: () => listChangeEvents({ limit: 200 }), refetchInterval: 10000 });
  const { data: freq } = useQuery({ queryKey: ["frequent-changes"], queryFn: () => frequentChanges() });
  const [selected, setSelected] = useState<ChangeEvent | null>(null);
  const [recordOpen, setRecordOpen] = useState(false);
  const [typeFilter, setTypeFilter] = useState<ChangeType | undefined>();

  const filtered = typeFilter ? events?.filter((e) => e.change_type === typeFilter) : events;

  const columns = [
    { title: "changed_at", dataIndex: "changed_at", width: 180 },
    { title: "type", dataIndex: "change_type", width: 170, render: (t: string) => <Tag>{t}</Tag> },
    { title: "target", dataIndex: "target_resource_id", ellipsis: true, render: (t: string) => <code>{t}</code> },
    { title: "severity", dataIndex: "severity_estimate", width: 90, render: (s: string) => <Tag color={SEV_COLOR[s] ?? "green"}>{s}</Tag> },
    { title: "source", dataIndex: "source", width: 100 },
    { title: "prop", width: 70, render: (_: unknown, e: ChangeEvent) => e.propagated_to.length },
    { title: "description", dataIndex: "description", ellipsis: true },
  ];

  return (
    <Card
      title="变更事件"
      extra={
        <Space>
          <Select
            allowClear
            placeholder="filter type"
            style={{ width: 190 }}
            onChange={(v) => setTypeFilter(v as ChangeType | undefined)}
            options={["configmap_updated", "secret_rotated", "deployment_rolled", "image_pushed"].map((t) => ({ value: t, label: t }))}
          />
          <Button type="primary" onClick={() => setRecordOpen(true)}>记录变更</Button>
        </Space>
      }
    >
      {freq && freq.frequent.length > 0 && (
        <Alert
          style={{ marginBottom: 12 }}
          type="warning"
          showIcon
          message={`过频变更:${freq.frequent.length} 个 target 超阈值 ${freq.threshold}/${freq.window_seconds}s`}
          description={freq.frequent.map((f) => `${f.target_resource_id}(${f.count})`).join(", ")}
        />
      )}
      <Table rowKey="change_event_id" size="small" dataSource={filtered} columns={columns} onRow={(e) => ({ onClick: () => setSelected(e), style: { cursor: "pointer" } })} />
      <Drawer open={!!selected} onClose={() => setSelected(null)} width={620} title={selected ? `change ${selected.change_event_id}` : ""}>
        {selected && <ChangeDetail event={selected} />}
      </Drawer>
      <RecordChangeModal
        open={recordOpen}
        onClose={() => setRecordOpen(false)}
        onCreated={() => { setRecordOpen(false); qc.invalidateQueries({ queryKey: ["change-events"] }); qc.invalidateQueries({ queryKey: ["frequent-changes"] }); }}
      />
    </Card>
  );
}

function ChangeDetail({ event }: { event: ChangeEvent }) {
  const { data: impact } = useQuery({ queryKey: ["change-impact", event.change_event_id], queryFn: () => changeEventImpact(event.change_event_id) });
  const { data: alerts } = useQuery({ queryKey: ["change-alerts", event.change_event_id], queryFn: () => changeEventAlerts(event.change_event_id) });
  const { data: sug } = useQuery({ queryKey: ["change-suggestion", event.change_event_id], queryFn: () => changeEventRecoverySuggestion(event.change_event_id) });
  const qc = useQueryClient();
  const execM = useMutation({
    mutationFn: (v: { actionId: string; targetResourceId: string }) => executeRecovery({ actionId: v.actionId, targetResourceId: v.targetResourceId, inputParams: {} }),
    onSuccess: (e) => { message.success(`执行已创建:${e.status}`); qc.invalidateQueries({ queryKey: ["recovery-executions"] }); },
    onError: (e) => message.error(String(e)),
  });

  return (
    <Tabs
      items={[
        {
          key: "detail", label: "详情", children: (
            <>
              <Descriptions size="small" column={1} bordered items={[
                { key: "t", label: "type", children: <Tag>{event.change_type}</Tag> },
                { key: "tg", label: "target", children: <code>{event.target_resource_id}</code> },
                { key: "tt", label: "target_type", children: event.target_resource_type || "-" },
                { key: "at", label: "changed_at", children: event.changed_at },
                { key: "by", label: "changed_by", children: event.changed_by || "-" },
                { key: "src", label: "source", children: event.source },
                { key: "sev", label: "severity", children: <Tag color={SEV_COLOR[event.severity_estimate] ?? "green"}>{event.severity_estimate}</Tag> },
                { key: "cm", label: "commit", children: event.related_commit || "-" },
                { key: "pr", label: "pr", children: event.related_pr || "-" },
                { key: "cl", label: "cluster", children: event.cluster_id || "-" },
              ]} />
              {event.description && <p style={{ marginTop: 12 }}>{event.description}</p>}
              {event.yaml_diff && (<><div style={{ marginTop: 8 }}>yaml_diff:</div><pre style={PRE}>{event.yaml_diff}</pre></>)}
              {Object.keys(event.diff_summary ?? {}).length > 0 && (<><div style={{ marginTop: 8 }}>diff_summary:</div><pre style={PRE}>{JSON.stringify(event.diff_summary, null, 2)}</pre></>)}
            </>
          ),
        },
        {
          key: "impact", label: `影响 (${impact?.affected_count ?? 0})`, children: (
            <List size="small" dataSource={impact?.affected ?? []} locale={{ emptyText: "无" }} renderItem={(r) => <List.Item><code>{r}</code></List.Item>} />
          ),
        },
        {
          key: "alerts", label: `告警 (${alerts?.total ?? 0})`, children: (
            <List size="small" dataSource={alerts?.alerts ?? []} locale={{ emptyText: "无关联告警" }} renderItem={(a) => (
              <List.Item><Space><Tag color={a.severity === "critical" ? "red" : "orange"}>{a.severity}</Tag><Tag>{a.status}</Tag><code>{a.resource_ref}</code>{a.alert_name}</Space></List.Item>
            )} />
          ),
        },
        {
          key: "sug", label: `恢复建议 (${sug?.total ?? 0})`, children: (
            <List size="small" dataSource={sug?.suggestions ?? []} locale={{ emptyText: "无" }} renderItem={(s) => (
              <List.Item actions={s.target_match !== "unresolved" && s.resolved_target_resource_id ? [
                <Button size="small" type="primary" loading={execM.isPending} onClick={() => execM.mutate({ actionId: s.action_id, targetResourceId: s.resolved_target_resource_id! })}>
                  {s.requires_approval ? "请求审批" : "执行"}
                </Button>,
              ] : []}>
                <List.Item.Meta
                  title={<span>{s.action_name} <Tag>{s.target_match}</Tag> <Tag color={s.risk_level === "high" ? "red" : s.risk_level === "medium" ? "orange" : "green"}>{s.risk_level}</Tag></span>}
                  description={<span>{s.rationale} · target=<code>{s.resolved_target_resource_id ?? "unresolved"}</code> · confidence={s.confidence.toFixed(2)}</span>}
                />
              </List.Item>
            )} />
          ),
        },
      ]}
    />
  );
}

function RecordChangeModal({
  open, onClose, onCreated,
}: {
  open: boolean;
  onClose: () => void;
  onCreated: () => void;
}) {
  const [form] = Form.useForm();
  // 拓扑节点供 target_resource_id 录入时搜索选择(避免手敲 resource_id 拼错 → 关联失败)
  const { data: graph } = useQuery({ queryKey: ["graph"], queryFn: getGraph });
  const nodeOptions = (graph?.nodes ?? []).map((n) => ({ value: n.id, label: `${n.type} · ${n.label}` }));
  const m = useMutation({
    mutationFn: (v: Record<string, unknown>) => recordChangeEvent({
      change_type: v.change_type as string,
      target_resource_id: v.target_resource_id as string,
      changed_by: (v.changed_by as string) || undefined,
      source: (v.source as string) || undefined,
      description: (v.description as string) || undefined,
      related_commit: (v.related_commit as string) || undefined,
      commit_sha: (v.commit_sha as string) || undefined,
      cluster_id: (v.cluster_id as string) || undefined,
    }),
    onSuccess: () => { message.success("已记录"); onCreated(); },
    onError: (e) => message.error(String(e)),
  });
  return (
    <Modal open={open} title="记录变更事件" onCancel={onClose} confirmLoading={m.isPending} onOk={() => form.submit()}>
      <Form form={form} layout="vertical" onFinish={(v) => m.mutate(v)}>
        <Form.Item name="change_type" label="change_type" rules={[{ required: true }]}>
          <Select options={["configmap_updated", "secret_rotated", "deployment_rolled", "image_pushed"].map((t) => ({ value: t, label: t }))} />
        </Form.Item>
        <Form.Item name="target_resource_id" label="target_resource_id" rules={[{ required: true }]}>
          <Select
            showSearch
            allowClear
            placeholder="选已有资源(搜索 id / label)"
            options={nodeOptions}
            filterOption={(input, option) => {
              const q = input.toLowerCase();
              return (
                String(option?.value ?? "").toLowerCase().includes(q) ||
                String(option?.label ?? "").toLowerCase().includes(q)
              );
            }}
          />
        </Form.Item>
        <Form.Item name="changed_by" label="changed_by"><Input /></Form.Item>
        <Form.Item name="source" label="source">
          <Select options={["manual", "k8s_api", "argo_cd", "gitops", "flagd", "unknown"].map((s) => ({ value: s, label: s }))} />
        </Form.Item>
        <Form.Item name="description" label="description"><Input.TextArea rows={2} /></Form.Item>
        <Form.Item name="commit_sha" label="commit_sha"><Input /></Form.Item>
        <Form.Item name="related_commit" label="related_commit"><Input /></Form.Item>
        <Form.Item name="cluster_id" label="cluster_id"><Input /></Form.Item>
      </Form>
    </Modal>
  );
}
