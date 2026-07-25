import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  Card, Table, Tag, Button, Modal, Form, Input, Select, message, Space,
} from "antd";
import {
  listAlerts, recordAlert, resolveAlert, getGraph,
  type AlertEvent, type AlertSeverity,
} from "../../api/client";

const SEV_COLOR: Record<string, string> = { critical: "red", warning: "orange" };

/** 告警列表 + 录入 + resolve(对齐 reference alert 视图,精简)。 */
export default function AlertsView() {
  const qc = useQueryClient();
  const { data: alerts } = useQuery({ queryKey: ["alerts"], queryFn: () => listAlerts({ limit: 200 }), refetchInterval: 10000 });
  const [open, setOpen] = useState(false);
  const resolveM = useMutation({
    mutationFn: (id: string) => resolveAlert(id),
    onSuccess: () => { message.success("已恢复"); qc.invalidateQueries({ queryKey: ["alerts"] }); },
    onError: (e) => message.error(String(e)),
  });

  const columns = [
    { title: "fired_at", dataIndex: "fired_at", width: 180 },
    { title: "severity", dataIndex: "severity", width: 90, render: (s: string) => <Tag color={SEV_COLOR[s] ?? "default"}>{s}</Tag> },
    { title: "status", dataIndex: "status", width: 90, render: (s: string) => <Tag color={s === "resolved" ? "green" : "default"}>{s}</Tag> },
    { title: "resource", dataIndex: "resource_ref", ellipsis: true, render: (t: string) => <code>{t}</code> },
    { title: "name", dataIndex: "alert_name" },
    { title: "metric", render: (_: unknown, a: AlertEvent) => `${a.metric_name}=${a.metric_value}` },
    { title: "ops", width: 100, render: (_: unknown, a: AlertEvent) => a.status === "firing" ? <Button size="small" loading={resolveM.isPending} onClick={() => resolveM.mutate(a.alert_event_id)}>resolve</Button> : null },
  ];

  return (
    <Card title="告警" extra={<Button type="primary" onClick={() => setOpen(true)}>录入告警</Button>} style={{ marginTop: 16 }}>
      <Table rowKey="alert_event_id" size="small" dataSource={alerts} columns={columns} />
      <RecordAlertModal open={open} onClose={() => setOpen(false)} onCreated={() => { setOpen(false); qc.invalidateQueries({ queryKey: ["alerts"] }); }} />
    </Card>
  );
}

function RecordAlertModal({ open, onClose, onCreated }: { open: boolean; onClose: () => void; onCreated: () => void }) {
  const [form] = Form.useForm();
  // 拓扑节点供 resource_ref 录入时搜索选择(避免手敲 resource_id 拼错 → 关联失败)
  const { data: graph } = useQuery({ queryKey: ["graph"], queryFn: getGraph });
  const nodeOptions = (graph?.nodes ?? []).map((n) => ({ value: n.id, label: `${n.type} · ${n.label}` }));
  const m = useMutation({
    mutationFn: (v: Record<string, unknown>) => recordAlert({
      alert_name: v.alert_name as string,
      resource_ref: v.resource_ref as string,
      severity: (v.severity as string) || undefined,
      rule_id: (v.rule_id as string) || undefined,
      metric_name: (v.metric_name as string) || undefined,
      metric_value: (v.metric_value as number) ?? 0,
      summary: (v.summary as string) || undefined,
      description: (v.description as string) || undefined,
      cluster_id: (v.cluster_id as string) || undefined,
    }),
    onSuccess: () => { message.success("已录入"); onCreated(); },
    onError: (e) => message.error(String(e)),
  });
  return (
    <Modal open={open} title="录入告警" onCancel={onClose} confirmLoading={m.isPending} onOk={() => form.submit()}>
      <Form form={form} layout="vertical" onFinish={(v) => m.mutate(v)} initialValues={{ severity: "critical" }}>
        <Form.Item name="alert_name" label="alert_name" rules={[{ required: true }]}><Input /></Form.Item>
        <Form.Item name="resource_ref" label="resource_ref" rules={[{ required: true }]}>
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
        <Space wrap>
          <Form.Item name="severity" label="severity"><Select style={{ width: 140 }} options={(["critical", "warning"] as AlertSeverity[]).map((s) => ({ value: s, label: s }))} /></Form.Item>
          <Form.Item name="metric_name" label="metric_name"><Input /></Form.Item>
          <Form.Item name="metric_value" label="metric_value"><Input type="number" /></Form.Item>
        </Space>
        <Form.Item name="rule_id" label="rule_id"><Input /></Form.Item>
        <Form.Item name="summary" label="summary"><Input /></Form.Item>
        <Form.Item name="description" label="description"><Input.TextArea rows={2} /></Form.Item>
        <Form.Item name="cluster_id" label="cluster_id"><Input /></Form.Item>
      </Form>
    </Modal>
  );
}
