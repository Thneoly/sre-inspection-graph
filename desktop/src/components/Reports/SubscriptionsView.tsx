import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  Table, Tag, Button, Drawer, Descriptions, Modal, Select, Input, Switch, Space, message, Card,
} from "antd";
import {
  listSubscriptions, createSubscription, updateSubscription, deleteSubscription,
  triggerSubscriptionNow,
  type ReportSubscription, type ReportTemplate, type SubscriptionStatus,
} from "../../api/client";

const STATUS_COLOR: Record<SubscriptionStatus, string> = {
  never: "default", ok: "green", failed: "red",
};

const TEMPLATES: { value: ReportTemplate; label: string }[] = [
  { value: "application_health", label: "application_health" },
  { value: "cluster_overview", label: "cluster_overview" },
  { value: "incident_report", label: "incident_report" },
];

/**
 * 报告订阅列表 + Drawer 详情 + 新建/启停/触发/删除(Phase 4.3)。
 *
 * cron 5-field(例 `0 9 * * 1` = 每周一 9:00);调度器 60s tick 检查触发。
 * 「触发」= trigger_subscription_now(立即跑一次,与调度同入口)。
 */
export default function SubscriptionsView() {
  const qc = useQueryClient();
  const { data: subs } = useQuery({
    queryKey: ["report-subscriptions"],
    queryFn: () => listSubscriptions(),
  });
  const [selected, setSelected] = useState<ReportSubscription | null>(null);
  const [createOpen, setCreateOpen] = useState(false);

  const invalidate = () => qc.invalidateQueries({ queryKey: ["report-subscriptions"] });

  const toggleM = useMutation({
    mutationFn: (s: ReportSubscription) => updateSubscription({ subscriptionId: s.subscription_id, enabled: !s.enabled }),
    onSuccess: () => { message.success("已切换"); invalidate(); },
    onError: (e) => message.error(String(e)),
  });
  const triggerM = useMutation({
    mutationFn: (id: string) => triggerSubscriptionNow(id),
    onSuccess: (t) => { message.success(`已触发: ${t.report_id.slice(0, 16)}`); invalidate(); qc.invalidateQueries({ queryKey: ["reports"] }); },
    onError: (e) => message.error(String(e)),
  });
  const deleteM = useMutation({
    mutationFn: (id: string) => deleteSubscription(id),
    onSuccess: () => { message.success("已删除"); invalidate(); setSelected(null); },
    onError: (e) => message.error(String(e)),
  });

  const columns = [
    { title: "template", dataIndex: "template_id", width: 160, render: (t: string) => <code>{t}</code> },
    { title: "cron", dataIndex: "cron", width: 120, render: (c: string) => <code>{c}</code> },
    { title: "recipients", dataIndex: "recipients", ellipsis: true, render: (r: string[]) => r.join(", ") },
    { title: "enabled", dataIndex: "enabled", width: 80, render: (e: boolean) => (e ? <Tag color="green">on</Tag> : <Tag>off</Tag>) },
    { title: "last", dataIndex: "last_status", width: 90, render: (s: SubscriptionStatus) => <Tag color={STATUS_COLOR[s]}>{s}</Tag> },
    { title: "ops", width: 230, render: (_: unknown, s: ReportSubscription) => (
      <Space>
        <Button size="small" loading={toggleM.isPending} onClick={(ev) => { ev.stopPropagation(); toggleM.mutate(s); }}>{s.enabled ? "停用" : "启用"}</Button>
        <Button size="small" type="primary" loading={triggerM.isPending} onClick={(ev) => { ev.stopPropagation(); triggerM.mutate(s.subscription_id); }}>触发</Button>
        <Button size="small" danger loading={deleteM.isPending} onClick={(ev) => { ev.stopPropagation(); Modal.confirm({ title: "删除订阅?", content: s.subscription_id, onOk: () => deleteM.mutate(s.subscription_id) }); }}>删除</Button>
      </Space>
    )},
  ];

  return (
    <Card title="报告订阅" extra={<Button type="primary" onClick={() => setCreateOpen(true)}>新建订阅</Button>}>
      <Table
        rowKey="subscription_id"
        size="small"
        dataSource={subs}
        columns={columns}
        onRow={(s) => ({ onClick: () => setSelected(s), style: { cursor: "pointer" } })}
      />
      <Drawer open={!!selected} onClose={() => setSelected(null)} width={560} title={selected ? `subscription ${selected.subscription_id.slice(0, 16)}` : ""}>
        {selected && (
          <Descriptions size="small" column={1} bordered items={[
            { key: "id", label: "subscription_id", children: <code>{selected.subscription_id}</code> },
            { key: "tpl", label: "template", children: <code>{selected.template_id}</code> },
            { key: "cron", label: "cron", children: <code>{selected.cron}</code> },
            { key: "rec", label: "recipients", children: selected.recipients.join(", ") },
            { key: "en", label: "enabled", children: selected.enabled ? "是" : "否" },
            { key: "mod", label: "modules", children: selected.modules.length ? selected.modules.join(",") : "(全)" },
            { key: "sc", label: "scope", children: JSON.stringify(selected.scope) },
            { key: "lr", label: "last_run_at", children: selected.last_run_at || "-" },
            { key: "ls", label: "last_status", children: <Tag color={STATUS_COLOR[selected.last_status]}>{selected.last_status}</Tag> },
            { key: "le", label: "last_error", children: selected.last_error || "-" },
            { key: "lrid", label: "last_report_id", children: selected.last_report_id ? <code>{selected.last_report_id.slice(0, 16)}</code> : "-" },
            { key: "ca", label: "created_at", children: selected.created_at },
          ]} />
        )}
      </Drawer>

      <CreateSubscriptionModal open={createOpen} onClose={() => setCreateOpen(false)} onDone={() => invalidate()} />
    </Card>
  );
}

function CreateSubscriptionModal({ open, onClose, onDone }: {
  open: boolean; onClose: () => void; onDone: () => void;
}) {
  const [template, setTemplate] = useState<ReportTemplate>("application_health");
  const [applicationId, setApplicationId] = useState("");
  const [clusterId, setClusterId] = useState("");
  const [changeEventId, setChangeEventId] = useState("");
  const [cron, setCron] = useState("0 9 * * 1");
  const [recipients, setRecipients] = useState("");
  const [modules, setModules] = useState("");
  const [enabled, setEnabled] = useState(true);

  const createM = useMutation({
    mutationFn: () => createSubscription({
      templateId: template,
      applicationId: applicationId || undefined,
      clusterId: clusterId || undefined,
      changeEventId: changeEventId || undefined,
      cron,
      recipients: recipients.split(",").map((s) => s.trim()).filter(Boolean),
      modules: modules ? modules.split(",").map((s) => s.trim()).filter(Boolean) : undefined,
      enabled,
    }),
    onSuccess: (s) => { message.success(`已创建: ${s.subscription_id.slice(0, 16)}`); onDone(); onClose(); },
    onError: (e) => message.error(String(e)),
  });

  return (
    <Modal
      open={open}
      title="新建订阅"
      onCancel={onClose}
      footer={[
        <Button key="c" onClick={onClose}>取消</Button>,
        <Button key="g" type="primary" loading={createM.isPending} onClick={() => createM.mutate()}>创建</Button>,
      ]}
    >
      <div style={{ marginBottom: 6 }}>模板:</div>
      <Select style={{ width: "100%" }} value={template} onChange={setTemplate} options={TEMPLATES} />
      <div style={{ marginTop: 12, marginBottom: 6 }}>scope(按模板选填):</div>
      <Input value={applicationId} onChange={(e) => setApplicationId(e.target.value)} placeholder="application_id" style={{ marginBottom: 6 }} />
      <Input value={clusterId} onChange={(e) => setClusterId(e.target.value)} placeholder="cluster_id" style={{ marginBottom: 6 }} />
      <Input value={changeEventId} onChange={(e) => setChangeEventId(e.target.value)} placeholder="change_event_id (incident_report)" style={{ marginBottom: 6 }} />
      <div style={{ marginTop: 6, marginBottom: 6 }}>cron(5-field,例 `0 9 * * 1`):</div>
      <Input value={cron} onChange={(e) => setCron(e.target.value)} placeholder="0 9 * * 1" style={{ marginBottom: 6 }} />
      <div style={{ marginBottom: 6 }}>recipients(逗号分隔邮箱):</div>
      <Input value={recipients} onChange={(e) => setRecipients(e.target.value)} placeholder="ops@example.com, sre@example.com" style={{ marginBottom: 6 }} />
      <div style={{ marginBottom: 6 }}>modules(逗号分隔,空 = 全模块):</div>
      <Input value={modules} onChange={(e) => setModules(e.target.value)} placeholder="health_score,risk_list" style={{ marginBottom: 6 }} />
      <Space style={{ marginTop: 4 }}>
        <Switch checked={enabled} onChange={setEnabled} /> <span>启用</span>
      </Space>
    </Modal>
  );
}
