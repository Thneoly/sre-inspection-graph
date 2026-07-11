import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  Table, Tag, Button, Drawer, Descriptions, Space, Modal, Select, Input, message, Card,
} from "antd";
import {
  listRecoveryExecutions, confirmRecoveryExecution, cancelRecoveryExecution,
  rollbackRecoveryExecution, reverifyRecoveryExecution, listRecoveryActions,
  type RecoveryExecution, type ActionDef,
} from "../../api/client";
import DryRunModal from "./DryRunModal";

const STATUS_COLOR: Record<string, string> = {
  succeeded: "green", failed: "red", awaiting_approval: "orange",
  executing: "blue", rolled_back: "purple", rejected: "default",
  approved: "cyan", pending: "default", dry_run_ok: "default",
};

const PRE: React.CSSProperties = {
  background: "#0d1117", color: "#c9d1d9", padding: "0.5rem 0.75rem",
  borderRadius: 4, fontSize: "0.8rem", overflow: "auto", maxHeight: 240,
};

/**
 * 恢复执行列表 + Drawer 详情 + 行内操作(对齐 reference `ExecutionsView.tsx`)。
 *
 * **ApprovalsView 折叠进此处**(单机确认门):awaiting_approval 行直接显示
 * 「确认 / 取消」按钮(= reference approve / reject);succeeded 行显示「回滚 / 重验」。
 * 「新建执行」-> picker(target + action)-> DryRunModal(预演 + 执行/请求审批)。
 */
export default function ExecutionsView() {
  const qc = useQueryClient();
  const { data: execs } = useQuery({
    queryKey: ["recovery-executions"],
    queryFn: () => listRecoveryExecutions({ limit: 200 }),
    refetchInterval: 5000,
  });
  const [selected, setSelected] = useState<RecoveryExecution | null>(null);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [dryAction, setDryAction] = useState<ActionDef | null>(null);
  const [dryTarget, setDryTarget] = useState("");

  const invalidate = () => qc.invalidateQueries({ queryKey: ["recovery-executions"] });

  const confirmM = useMutation({ mutationFn: (id: string) => confirmRecoveryExecution(id),
    onSuccess: (e) => { message.success(`confirmed: ${e.status}`); invalidate(); setSelected(e); },
    onError: (e) => message.error(String(e)) });
  const cancelM = useMutation({ mutationFn: (id: string) => cancelRecoveryExecution(id),
    onSuccess: (e) => { message.success(`cancelled: ${e.status}`); invalidate(); setSelected(e); },
    onError: (e) => message.error(String(e)) });
  const rollbackM = useMutation({ mutationFn: (id: string) => rollbackRecoveryExecution(id),
    onSuccess: (e) => { message.success(`rollback: ${e.status}`); invalidate(); setSelected(e); },
    onError: (e) => message.error(String(e)) });
  const reverifyM = useMutation({ mutationFn: (id: string) => reverifyRecoveryExecution(id),
    onSuccess: (e) => { message.success(`reverify: ${e.verify_status}`); invalidate(); setSelected(e); },
    onError: (e) => message.error(String(e)) });

  const rollbackable = (e: RecoveryExecution) => e.status === "succeeded" && !e.rollback_execution_id;

  const columns = [
    { title: "status", dataIndex: "status", width: 130, render: (s: string) => <Tag color={STATUS_COLOR[s] ?? "default"}>{s}</Tag> },
    { title: "action", dataIndex: "action_id", render: (a: string) => <code>{a}</code> },
    { title: "target", dataIndex: "target_resource_id", ellipsis: true, render: (t: string) => <code>{t}</code> },
    { title: "verify", dataIndex: "verify_status", width: 110, render: (v: string) => <Tag>{v}</Tag> },
    { title: "by", dataIndex: "initiated_by", width: 100 },
    { title: "ops", width: 240, render: (_: unknown, e: RecoveryExecution) => (
      <Space>
        {e.status === "awaiting_approval" && <Button size="small" type="primary" loading={confirmM.isPending} onClick={(ev) => { ev.stopPropagation(); confirmM.mutate(e.execution_id); }}>确认</Button>}
        {e.status === "awaiting_approval" && <Button size="small" danger loading={cancelM.isPending} onClick={(ev) => { ev.stopPropagation(); cancelM.mutate(e.execution_id); }}>取消</Button>}
        {rollbackable(e) && <Button size="small" loading={rollbackM.isPending} onClick={(ev) => { ev.stopPropagation(); Modal.confirm({ title: "回滚执行?", content: `反向 ${e.action_id} on ${e.target_resource_id}`, onOk: () => rollbackM.mutate(e.execution_id) }); }}>回滚</Button>}
        {(e.status === "succeeded" || e.status === "rolled_back") && <Button size="small" loading={reverifyM.isPending} onClick={(ev) => { ev.stopPropagation(); reverifyM.mutate(e.execution_id); }}>重验</Button>}
      </Space>
    )},
  ];

  return (
    <Card title="恢复执行" extra={<Button type="primary" onClick={() => setPickerOpen(true)}>新建执行</Button>}>
      <Table
        rowKey="execution_id"
        size="small"
        dataSource={execs}
        columns={columns}
        onRow={(e) => ({ onClick: () => setSelected(e), style: { cursor: "pointer" } })}
      />
      <Drawer open={!!selected} onClose={() => setSelected(null)} width={560} title={selected ? `execution ${selected.execution_id.slice(0, 8)}` : ""}>
        {selected && (
          <>
            <Descriptions size="small" column={1} bordered items={[
              { key: "s", label: "status", children: <Tag color={STATUS_COLOR[selected.status] ?? "default"}>{selected.status}</Tag> },
              { key: "a", label: "action", children: <code>{selected.action_id}</code> },
              { key: "t", label: "target", children: <code>{selected.target_resource_id}</code> },
              { key: "tt", label: "target_type", children: selected.target_resource_type },
              { key: "by", label: "initiated_by", children: selected.initiated_by },
              { key: "ia", label: "initiated_at", children: selected.initiated_at },
              { key: "r", label: "reason", children: selected.request_reason || "-" },
              { key: "vs", label: "verify_status", children: selected.verify_status },
              { key: "rb", label: "rollback_exec", children: selected.rollback_execution_id ?? "-" },
              { key: "rv", label: "reverses_exec", children: selected.reverses_execution_id ?? "-" },
            ]} />
            <div style={{ marginTop: 12 }}>input_params:</div>
            <pre style={PRE}>{JSON.stringify(selected.input_params, null, 2)}</pre>
            <div style={{ marginTop: 8 }}>result:</div>
            <pre style={PRE}>{JSON.stringify(selected.result, null, 2)}</pre>
            {selected.verify_result && Object.keys(selected.verify_result).length > 0 && (
              <>
                <div style={{ marginTop: 8 }}>verify_result:</div>
                <pre style={PRE}>{JSON.stringify(selected.verify_result, null, 2)}</pre>
              </>
            )}
          </>
        )}
      </Drawer>

      <NewExecutionPicker
        open={pickerOpen}
        onClose={() => setPickerOpen(false)}
        onPick={(action, target) => { setPickerOpen(false); setDryAction(action); setDryTarget(target); }}
      />
      <DryRunModal
        open={!!dryAction}
        action={dryAction}
        targetResourceId={dryTarget}
        onClose={() => setDryAction(null)}
        onExecuted={() => invalidate()}
      />
    </Card>
  );
}

function NewExecutionPicker({
  open, onClose, onPick,
}: {
  open: boolean;
  onClose: () => void;
  onPick: (action: ActionDef, target: string) => void;
}) {
  const [target, setTarget] = useState("");
  const [actionId, setActionId] = useState<string | undefined>();
  const { data: actions } = useQuery({ queryKey: ["recovery-actions-all"], queryFn: () => listRecoveryActions() });
  const action = actions?.find((a) => a.action_id === actionId);
  return (
    <Modal
      open={open}
      title="新建恢复执行"
      onCancel={onClose}
      footer={[
        <Button key="c" onClick={onClose}>取消</Button>,
        <Button key="p" type="primary" disabled={!action || !target} onClick={() => action && onPick(action, target)}>预演</Button>,
      ]}
    >
      <div style={{ marginBottom: 6 }}>目标资源 ID:</div>
      <Input value={target} onChange={(e) => setTarget(e.target.value)} placeholder="deploy:order-api" />
      <div style={{ marginTop: 12, marginBottom: 6 }}>选择动作:</div>
      <Select
        style={{ width: "100%" }}
        placeholder="选择动作"
        value={actionId}
        onChange={setActionId}
        options={(actions ?? []).map((a) => ({ value: a.action_id, label: `${a.name} (${a.action_id}) · ${a.risk_level}` }))}
      />
      {action && (
        <div style={{ marginTop: 12, color: "#666", fontSize: "0.85rem" }}>
          {action.description} · target_type={action.target_type} · risk={action.risk_level}
        </div>
      )}
    </Modal>
  );
}
