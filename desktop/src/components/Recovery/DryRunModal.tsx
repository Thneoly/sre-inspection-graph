import { useState } from "react";
import { Modal, Input, Button, Alert, Descriptions, Tag, Space, message } from "antd";
import {
  dryRunRecovery,
  executeRecovery,
  type ActionDef,
  type DryRunResult,
  type RecoveryExecution,
} from "../../api/client";

interface Props {
  open: boolean;
  action: ActionDef | null;
  targetResourceId: string;
  onClose: () => void;
  /** 执行创建后回调(让父视图刷列表)。 */
  onExecuted?: (exec: RecoveryExecution) => void;
}

/**
 * 两步:先 dry-run 看影响范围,再「执行」(low)或「请求审批」(medium/high)。
 * 移植自 reference `DryRunModal.tsx`,axios -> invoke。input_params 是 JSON 文本,
 * medium/high 额外收 request_reason。
 */
export default function DryRunModal({ open, action, targetResourceId, onClose, onExecuted }: Props) {
  const [paramsText, setParamsText] = useState("{}");
  const [reason, setReason] = useState("");
  const [dry, setDry] = useState<DryRunResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);

  function handleClose() {
    setDry(null);
    setParamsText("{}");
    setReason("");
    onClose();
  }

  async function doDryRun() {
    if (!action) return;
    setLoading(true);
    try {
      const params = JSON.parse(paramsText || "{}");
      const r = await dryRunRecovery(action.action_id, targetResourceId, params);
      setDry(r);
    } catch (e) {
      message.error(String(e));
    } finally {
      setLoading(false);
    }
  }

  async function doExecute() {
    if (!action) return;
    setBusy(true);
    try {
      const params = JSON.parse(paramsText || "{}");
      const exec = await executeRecovery({
        actionId: action.action_id,
        targetResourceId,
        inputParams: params,
        requestReason: reason,
      });
      onExecuted?.(exec);
      message.success(`执行已创建:status=${exec.status}`);
      handleClose();
    } catch (e) {
      message.error(String(e));
    } finally {
      setBusy(false);
    }
  }

  const needsApproval = action?.requires_approval || action?.risk_level !== "low";

  return (
    <Modal
      open={open}
      title={action ? `预演:${action.name}` : ""}
      onCancel={handleClose}
      width={640}
      footer={[
        <Button key="cancel" onClick={handleClose}>取消</Button>,
        <Button key="dry" loading={loading} disabled={!action} onClick={doDryRun}>预演 dry-run</Button>,
        dry?.target_valid ? (
          <Button key="exec" type="primary" loading={busy} onClick={doExecute}>
            {needsApproval ? "请求审批" : "执行"}
          </Button>
        ) : null,
      ]}
    >
      {action && (
        <>
          <Descriptions size="small" column={1} items={[
            { key: "aid", label: "action", children: <code>{action.action_id}</code> },
            { key: "tgt", label: "target", children: <code>{targetResourceId}</code> },
            { key: "risk", label: "risk", children: <Tag color={action.risk_level === "high" ? "red" : action.risk_level === "medium" ? "orange" : "green"}>{action.risk_level}</Tag> },
            { key: "ap", label: "需审批", children: action.requires_approval ? "是" : "否" },
          ]} />
          <div style={{ marginTop: 12 }}>参数(JSON):</div>
          <Input.TextArea value={paramsText} onChange={(e) => setParamsText(e.target.value)} rows={4} style={{ fontFamily: "monospace" }} />
          {needsApproval && (
            <Input value={reason} onChange={(e) => setReason(e.target.value)} placeholder="审批理由(request reason)" style={{ marginTop: 8 }} />
          )}
          {dry && !dry.target_valid && (
            <Alert style={{ marginTop: 12 }} type="error" message={`预演失败:${dry.validation_error}`} />
          )}
          {dry && dry.target_valid && (
            <Alert style={{ marginTop: 12 }} type="info" showIcon message={
              <>
                <div>影响 {dry.affected_count} 资源 · 预估 {dry.estimated_duration_seconds}s · SLA {dry.estimated_sla_impact}</div>
                <Space wrap style={{ marginTop: 4 }}>
                  {dry.affected_resources.slice(0, 8).map((r) => (
                    <Tag key={r.resource_id}>{r.type}:{r.name} · {r.impact_severity}</Tag>
                  ))}
                </Space>
                {dry.rollback_action_id && <div style={{ marginTop: 4 }}>可回滚:<code>{dry.rollback_action_id}</code></div>}
              </>
            } />
          )}
        </>
      )}
    </Modal>
  );
}
