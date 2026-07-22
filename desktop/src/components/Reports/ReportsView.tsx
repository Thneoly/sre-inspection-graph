import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  Table, Tag, Button, Drawer, Modal, Select, Input, message, Card, Space,
} from "antd";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  listReports, generateReport, getReport, clearReports,
  type ReportTask, type ReportTemplate,
} from "../../api/client";

const STATUS_COLOR: Record<string, string> = {
  completed: "green", generating: "blue", failed: "red", pending: "default",
};

// trigger_source -> Tag 配色 + 中文 label(区分手动 / 调度 / 立即触发)。
const TRIGGER_COLOR: Record<string, string> = {
  manual_cmd: "default", scheduled: "purple", trigger_now: "gold",
};
const TRIGGER_LABEL: Record<string, string> = {
  manual_cmd: "手动", scheduled: "调度", trigger_now: "立即触发",
};

const TEMPLATES: { value: ReportTemplate; label: string }[] = [
  { value: "application_health", label: "应用健康报告 (application_health)" },
  { value: "cluster_overview", label: "集群总览 (cluster_overview)" },
  { value: "incident_report", label: "事件报告 (incident_report)" },
];

/**
 * 报告列表 + Drawer(markdown 渲染)+ 生成 Modal(Phase 4.1/4.3)。
 *
 * 生成:选模板 + 填 scope(application_id / cluster_id / change_event_id / fault_id
 * 按模板选填)+ modules(逗号分隔,空 = 全模块)。
 */
export default function ReportsView() {
  const qc = useQueryClient();
  const { data: reports } = useQuery({
    queryKey: ["reports"],
    queryFn: () => listReports(),
  });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [genOpen, setGenOpen] = useState(false);

  const invalidate = () => qc.invalidateQueries({ queryKey: ["reports"] });

  const clearM = useMutation({
    mutationFn: () => clearReports(),
    onSuccess: (n) => { message.success(`已清空 ${n} 条报告`); setSelectedId(null); invalidate(); },
    onError: (e) => message.error(String(e)),
  });

  const { data: selected } = useQuery({
    queryKey: ["report", selectedId],
    queryFn: () => getReport(selectedId!),
    enabled: !!selectedId,
  });

  const columns = [
    { title: "report_id", dataIndex: "report_id", ellipsis: true, render: (s: string) => <code>{s.slice(0, 16)}</code> },
    { title: "template", dataIndex: "template_id", width: 160, render: (t: string) => <code>{t}</code> },
    { title: "status", dataIndex: "status", width: 110, render: (s: string) => <Tag color={STATUS_COLOR[s] ?? "default"}>{s}</Tag> },
    { title: "source", dataIndex: "trigger_source", width: 120, render: (s: string) => <Tag color={TRIGGER_COLOR[s] ?? "default"}>{TRIGGER_LABEL[s] ?? s}</Tag> },
    { title: "created", dataIndex: "created_at", ellipsis: true },
  ];

  return (
    <Card title="报告" extra={<Space>
      <Button onClick={() => Modal.confirm({ title: "清空所有报告历史?", content: "不可恢复", onOk: () => clearM.mutate() })} loading={clearM.isPending}>清空历史</Button>
      <Button type="primary" onClick={() => setGenOpen(true)}>生成报告</Button>
    </Space>}>
      <Table
        rowKey="report_id"
        size="small"
        dataSource={reports}
        columns={columns}
        onRow={(r) => ({ onClick: () => setSelectedId(r.report_id), style: { cursor: "pointer" } })}
      />
      <Drawer
        open={!!selectedId}
        onClose={() => setSelectedId(null)}
        width={720}
        title={selected ? `report ${selected.report_id.slice(0, 16)}` : ""}
      >
        {selected && (
          <>
            <Space wrap size="small" style={{ marginBottom: 8 }}>
              <Tag color={STATUS_COLOR[selected.status] ?? "default"}>{selected.status}</Tag>
              <Tag>{selected.template_id}</Tag>
              <Tag color={TRIGGER_COLOR[selected.trigger_source] ?? "default"}>{TRIGGER_LABEL[selected.trigger_source] ?? selected.trigger_source}</Tag>
              {selected.error_message && <Tag color="red">error</Tag>}
            </Space>
            {selected.error_message && (
              <div style={{ color: "red", marginBottom: 8 }}>{selected.error_message}</div>
            )}
            <div style={{ marginBottom: 8, color: "#666", fontSize: "0.85rem" }}>
              scope: {JSON.stringify(selected.scope)} · modules: {selected.modules.length ? selected.modules.join(",") : "(全)"} · created: {selected.created_at}
            </div>
            {selected.markdown ? (
              <div className="report-md" style={{ background: "#fafafa", padding: "1rem", borderRadius: 4, fontSize: "0.85rem" }}>
                <ReactMarkdown remarkPlugins={[remarkGfm]}>{selected.markdown}</ReactMarkdown>
              </div>
            ) : (
              <div style={{ color: "#999" }}>无 markdown(status={selected.status})</div>
            )}
          </>
        )}
      </Drawer>

      <GenerateModal open={genOpen} onClose={() => setGenOpen(false)} onDone={() => invalidate()} />
    </Card>
  );
}

function GenerateModal({ open, onClose, onDone }: {
  open: boolean; onClose: () => void; onDone: () => void;
}) {
  const [template, setTemplate] = useState<ReportTemplate>("application_health");
  const [applicationId, setApplicationId] = useState("");
  const [clusterId, setClusterId] = useState("");
  const [changeEventId, setChangeEventId] = useState("");
  const [faultId, setFaultId] = useState("");
  const [modules, setModules] = useState("");

  const genM = useMutation({
    mutationFn: () => generateReport({
      templateId: template,
      applicationId: applicationId || undefined,
      clusterId: clusterId || undefined,
      changeEventId: changeEventId || undefined,
      faultId: faultId || undefined,
      modules: modules ? modules.split(",").map((s) => s.trim()).filter(Boolean) : undefined,
    }),
    onSuccess: (t) => { message.success(`生成成功: ${t.report_id.slice(0, 16)} (${t.status})`); onDone(); onClose(); },
    onError: (e) => message.error(String(e)),
  });

  return (
    <Modal
      open={open}
      title="生成报告"
      onCancel={onClose}
      footer={[
        <Button key="c" onClick={onClose}>取消</Button>,
        <Button key="g" type="primary" loading={genM.isPending} onClick={() => genM.mutate()}>生成</Button>,
      ]}
    >
      <div style={{ marginBottom: 6 }}>模板:</div>
      <Select
        style={{ width: "100%" }}
        value={template}
        onChange={setTemplate}
        options={TEMPLATES}
      />
      <div style={{ marginTop: 12, marginBottom: 6, color: "#666", fontSize: "0.85rem" }}>
        scope(按模板选填:app_health=application_id / cluster_overview=cluster_id / incident=change_event_id):
      </div>
      <Input value={applicationId} onChange={(e) => setApplicationId(e.target.value)} placeholder="application_id (app:order)" style={{ marginBottom: 6 }} />
      <Input value={clusterId} onChange={(e) => setClusterId(e.target.value)} placeholder="cluster_id (otel-demo)" style={{ marginBottom: 6 }} />
      <Input value={changeEventId} onChange={(e) => setChangeEventId(e.target.value)} placeholder="change_event_id (ce-...)" style={{ marginBottom: 6 }} />
      <Input value={faultId} onChange={(e) => setFaultId(e.target.value)} placeholder="fault_id (Rust 不支持)" disabled style={{ marginBottom: 6 }} />
      <div style={{ marginTop: 6, marginBottom: 6 }}>modules(逗号分隔,空 = 全模块):</div>
      <Input value={modules} onChange={(e) => setModules(e.target.value)} placeholder="health_score,risk_list" />
    </Modal>
  );
}
