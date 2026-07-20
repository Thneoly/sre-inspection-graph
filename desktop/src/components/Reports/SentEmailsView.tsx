import { useQuery } from "@tanstack/react-query";
import { Table, Tag, Drawer, Empty, Card } from "antd";
import { useState } from "react";
import { listSentEmails, type SentEmail } from "../../api/client";

const PRE: React.CSSProperties = {
  background: "#0d1117", color: "#c9d1d9", padding: "0.5rem 0.75rem",
  borderRadius: 4, fontSize: "0.8rem", overflow: "auto", maxHeight: 320, whiteSpace: "pre-wrap",
};

/**
 * 已发送邮件调试面板(Phase 4.3)。仅 InMemory 模式(SMTP_HOST 未配置)返回捕获的邮件;
 * Smtp 模式返回空(邮件真发,不经过 InMemory)。
 */
export default function SentEmailsView() {
  const { data: emails } = useQuery({
    queryKey: ["sent-emails"],
    queryFn: () => listSentEmails(),
    refetchInterval: 5000,
  });
  const [selected, setSelected] = useState<SentEmail | null>(null);

  const columns = [
    { title: "subject", dataIndex: "subject", ellipsis: true },
    { title: "recipients", dataIndex: "recipients", ellipsis: true, render: (r: string[]) => r.join(", ") },
    { title: "attachment", dataIndex: "attachment_filename", width: 180, render: (s: string) => <code>{s}</code> },
  ];

  return (
    <Card title={<span>已发送邮件 <Tag color="blue">InMemory 调试</Tag></span>} size="small">
      {emails && emails.length === 0 ? (
        <Empty description="无邮件(Smtp 模式不捕获;或 InMemory 模式尚未触发)" />
      ) : (
        <Table
          rowKey={(e, i) => `${e.attachment_filename}-${i}`}
          size="small"
          dataSource={emails}
          columns={columns}
          onRow={(e) => ({ onClick: () => setSelected(e), style: { cursor: "pointer" } })}
        />
      )}
      <Drawer open={!!selected} onClose={() => setSelected(null)} width={640} title={selected?.subject}>
        {selected && (
          <>
            <div style={{ marginBottom: 8, color: "#666", fontSize: "0.85rem" }}>
              to: {selected.recipients.join(", ")} · 附件: <code>{selected.attachment_filename}</code>
            </div>
            <pre style={PRE}>{selected.body}</pre>
          </>
        )}
      </Drawer>
    </Card>
  );
}
