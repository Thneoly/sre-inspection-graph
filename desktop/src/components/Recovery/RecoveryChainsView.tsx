import { useState } from "react";
import { useQuery, useMutation, useQueries, useQueryClient } from "@tanstack/react-query";
import {
  Card, Table, Tag, Button, Drawer, Descriptions, Timeline, Modal, Form, Select, Input,
  message, Space, Popconfirm,
} from "antd";
import {
  listRecoveryChains, listChainTemplates, executeChain, confirmChain, cancelChain,
  abortChain, getRecoveryExecution,
  type RecoveryChain, type OnFailureStrategy,
} from "../../api/client";

const CHAIN_STATUS_COLOR: Record<string, string> = {
  succeeded: "green", failed: "red", awaiting_approval: "orange", executing: "blue",
  partial: "volcano", rolled_back: "purple", aborted: "default", pending: "default",
};

/**
 * 恢复链列表 + Drawer(step Timeline)+ 发起链(对齐 reference `RecoveryChainsView.tsx`)。
 * awaiting_approval 行显示「确认 / 取消」;executing 显示「中止」。
 */
export default function RecoveryChainsView() {
  const qc = useQueryClient();
  const { data: chains } = useQuery({
    queryKey: ["recovery-chains"],
    queryFn: () => listRecoveryChains({ limit: 100 }),
    refetchInterval: 5000,
  });
  const [selected, setSelected] = useState<RecoveryChain | null>(null);
  const [execOpen, setExecOpen] = useState(false);
  const invalidate = () => qc.invalidateQueries({ queryKey: ["recovery-chains"] });

  const confirmM = useMutation({ mutationFn: (id: string) => confirmChain(id),
    onSuccess: (c) => { message.success(`chain: ${c.status}`); invalidate(); setSelected(c); }, onError: (e) => message.error(String(e)) });
  const cancelM = useMutation({ mutationFn: (id: string) => cancelChain(id),
    onSuccess: (c) => { message.success(`chain: ${c.status}`); invalidate(); setSelected(c); }, onError: (e) => message.error(String(e)) });
  const abortM = useMutation({ mutationFn: (id: string) => abortChain(id),
    onSuccess: (c) => { message.success(`chain: ${c.status}`); invalidate(); setSelected(c); }, onError: (e) => message.error(String(e)) });

  const columns = [
    { title: "status", dataIndex: "status", width: 130, render: (s: string) => <Tag color={CHAIN_STATUS_COLOR[s] ?? "default"}>{s}</Tag> },
    { title: "template", dataIndex: "template_name" },
    { title: "target", dataIndex: "target_resource_id", ellipsis: true, render: (t: string) => <code>{t}</code> },
    { title: "progress", width: 90, render: (_: unknown, c: RecoveryChain) => `${c.current_step_index}/${c.total_steps}` },
    { title: "on_failure", dataIndex: "on_failure", width: 120 },
    { title: "ops", width: 200, render: (_: unknown, c: RecoveryChain) => (
      <Space>
        {c.status === "awaiting_approval" && <Button size="small" type="primary" loading={confirmM.isPending} onClick={(e) => { e.stopPropagation(); confirmM.mutate(c.chain_id); }}>确认</Button>}
        {c.status === "awaiting_approval" && <Button size="small" danger loading={cancelM.isPending} onClick={(e) => { e.stopPropagation(); cancelM.mutate(c.chain_id); }}>取消</Button>}
        {(c.status === "executing" || c.status === "awaiting_approval") && (
          <Popconfirm title="中止链?" onConfirm={() => abortM.mutate(c.chain_id)}>
            <Button size="small" loading={abortM.isPending} onClick={(e) => e.stopPropagation()}>中止</Button>
          </Popconfirm>
        )}
      </Space>
    )},
  ];

  return (
    <Card title="恢复链" extra={<Button type="primary" onClick={() => setExecOpen(true)}>发起恢复链</Button>} style={{ marginTop: 16 }}>
      <Table rowKey="chain_id" size="small" dataSource={chains} columns={columns} onRow={(c) => ({ onClick: () => setSelected(c), style: { cursor: "pointer" } })} />
      <Drawer open={!!selected} onClose={() => setSelected(null)} width={520} title={selected ? `chain ${selected.chain_id.slice(0, 8)}` : ""}>
        {selected && <ChainDetail chain={selected} />}
      </Drawer>
      <ExecuteChainModal open={execOpen} onClose={() => setExecOpen(false)} onCreated={() => { setExecOpen(false); invalidate(); }} />
    </Card>
  );
}

function ChainDetail({ chain }: { chain: RecoveryChain }) {
  const stepQs = useQueries({
    queries: chain.step_executions.map((id) => ({
      queryKey: ["recovery-exec", id],
      queryFn: () => getRecoveryExecution(id),
    })),
  });
  return (
    <>
      <Descriptions size="small" column={1} bordered items={[
        { key: "s", label: "status", children: <Tag color={CHAIN_STATUS_COLOR[chain.status] ?? "default"}>{chain.status}</Tag> },
        { key: "t", label: "template", children: chain.template_name },
        { key: "tg", label: "target", children: <code>{chain.target_resource_id}</code> },
        { key: "p", label: "progress", children: `${chain.current_step_index}/${chain.total_steps}` },
        { key: "of", label: "on_failure", children: chain.on_failure },
        { key: "by", label: "initiated_by", children: chain.initiated_by },
        { key: "fr", label: "failure_reason", children: chain.failure_reason || "-" },
      ]} />
      <div style={{ marginTop: 12 }}>Steps:</div>
      <Timeline style={{ marginTop: 8 }} items={stepQs.map((q, i) => {
        const e = q.data;
        const color = !e ? "gray" : e.status === "succeeded" ? "green" : e.status === "failed" ? "red" : e.status === "awaiting_approval" ? "orange" : "blue";
        return {
          color,
          children: <span>step {i}: {e ? (<><code>{e.action_id}</code> <Tag>{e.status}</Tag> <Tag>{e.verify_status}</Tag></>) : "loading..."}</span>,
        };
      })} />
    </>
  );
}

function ExecuteChainModal({
  open, onClose, onCreated,
}: {
  open: boolean;
  onClose: () => void;
  onCreated: () => void;
}) {
  const [form] = Form.useForm();
  const { data: templates } = useQuery({ queryKey: ["chain-templates"], queryFn: listChainTemplates });
  const m = useMutation({
    mutationFn: (v: { templateId: string; targetResourceId: string; initiatedBy?: string; requestReason?: string; onFailureOverride?: OnFailureStrategy }) => executeChain(v),
    onSuccess: (c) => { message.success(`chain: ${c.status}`); onCreated(); },
    onError: (e) => message.error(String(e)),
  });
  return (
    <Modal open={open} title="发起恢复链" onCancel={onClose} confirmLoading={m.isPending} onOk={() => form.submit()}>
      <Form form={form} layout="vertical" onFinish={(v) => m.mutate(v)}>
        <Form.Item name="templateId" label="模板" rules={[{ required: true }]}>
          <Select options={(templates ?? []).map((t) => ({ value: t.template_id, label: `${t.name} (${t.template_id})` }))} />
        </Form.Item>
        <Form.Item name="targetResourceId" label="目标资源 ID" rules={[{ required: true }]}>
          <Input placeholder="deploy:order-api" />
        </Form.Item>
        <Form.Item name="initiatedBy" label="发起人"><Input /></Form.Item>
        <Form.Item name="requestReason" label="理由"><Input /></Form.Item>
        <Form.Item name="onFailureOverride" label="on_failure 覆盖">
          <Select allowClear options={[{ value: "stop", label: "stop" }, { value: "rollback_all", label: "rollback_all" }, { value: "continue", label: "continue" }]} />
        </Form.Item>
      </Form>
    </Modal>
  );
}
