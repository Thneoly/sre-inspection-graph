import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Card, Table, Tag, Button, message, Tooltip, Descriptions, Empty, Typography } from "antd";
import { ReloadOutlined } from "@ant-design/icons";
import {
  getConnectorsStatus, syncAllNow,
  type ConnectorStatus,
} from "../../api/client";

const { Text } = Typography;

/**
 * Phase 6 connectors-ui —— connector / handler 运行时观测。
 *
 * dogfood 痛点:之前 connector 健康/sync 状态只能看日志(`list_connectors` 只列已加载
 * 模块,后台 sync_loop 的 per-connector 产出被丢)。本视图吃 `get_connectors_status`
 * (持久注册表,manifest 全量播种 + 每次 sync 刷新),一张表同时回答「配了啥」+「跑得咋样」。
 * 5s 轮询让后台 sync_loop 的活动可见(last_synced_at / fact_count / errors 实时变)。
 */
export default function ConnectorsView() {
  const qc = useQueryClient();
  const { data } = useQuery({
    queryKey: ["connectors-status"],
    queryFn: getConnectorsStatus,
    refetchInterval: 5000,
  });

  const syncM = useMutation({
    mutationFn: () => syncAllNow(),
    onSuccess: (s) => {
      message.success(`sync 完成:${s.facts.length} facts, ${s.total_errors} errors, ${s.total_duration_ms}ms`);
      qc.invalidateQueries({ queryKey: ["connectors-status"] });
    },
    onError: (e) => message.error(String(e)),
  });

  const connectors = (data ?? []).filter((c) => c.kind === "connector");
  const handlers = (data ?? []).filter((c) => c.kind === "handler");

  return (
    <>
      <Card
        title={`Connectors (${connectors.length})`}
        extra={
          <Button type="primary" icon={<ReloadOutlined />} loading={syncM.isPending} onClick={() => syncM.mutate()}>
            Sync all now
          </Button>
        }
      >
        <Table<ConnectorStatus>
          rowKey="name"
          size="small"
          dataSource={connectors}
          pagination={false}
          expandable={{ expandedRowRender }}
          columns={columns}
        />
      </Card>

      <Card title={`Handlers (${handlers.length})`} style={{ marginTop: 16 }}>
        {handlers.length === 0 ? (
          <Empty description="无 handler 模块(handler 驱动 recovery real-mode)" image={Empty.PRESENTED_IMAGE_SIMPLE} />
        ) : (
          <Table<ConnectorStatus>
            rowKey="name"
            size="small"
            dataSource={handlers}
            pagination={false}
            columns={[
              { title: "name", dataIndex: "name", render: (t: string) => <code>{t}</code> },
              { title: "version", dataIndex: "version", width: 90 },
              { title: "capabilities", dataIndex: "capabilities", render: (caps: string[]) => caps.map((c) => <Tag key={c}>{c}</Tag>) },
              { title: "status", width: 110, render: statusTag },
              { title: "note", render: (_: unknown, c: ConnectorStatus) => c.load_error ? <Text type="danger">{c.load_error}</Text> : <Text type="secondary">recovery real-mode 用</Text> },
            ]}
          />
        )}
      </Card>
    </>
  );
}

const columns = [
  { title: "name", dataIndex: "name", width: 130, render: (t: string) => <code>{t}</code> },
  { title: "status", width: 110, render: statusTag },
  { title: "capabilities", dataIndex: "capabilities", render: (caps: string[]) => caps.map((c) => <Tag key={c}>{c}</Tag>) },
  { title: "interval", dataIndex: "sync_interval_seconds", width: 80, render: (s: number) => `${s}s` },
  { title: "config", width: 220, render: configCell },
  { title: "last sync", width: 100, render: (_: unknown, c: ConnectorStatus) => relativeFrom(c.last_synced_at) },
  { title: "facts", width: 70, render: (_: unknown, c: ConnectorStatus) => (c.last_fact_count ?? "—") },
  { title: "dur", width: 80, render: (_: unknown, c: ConnectorStatus) => (c.last_duration_ms != null ? `${c.last_duration_ms}ms` : "—") },
  { title: "errors", width: 90, render: errorsCell },
];

/** enabled/loaded/load_error 三态徽章。loaded≠enabled:失败模块 enabled 仍 true。 */
function statusTag(c: ConnectorStatus) {
  if (!c.enabled) return <Tag>disabled</Tag>;
  if (c.loaded) return <Tag color="green">running</Tag>;
  if (c.load_error) return <Tag color="red">failed</Tag>;
  return <Tag color="orange">not loaded</Tag>;
}

/** per-connector config 头两个键作 hint,详情进展开行。 */
function configCell(_: unknown, c: ConnectorStatus) {
  if (!c.config) return <Text type="secondary">—</Text>;
  const entries = Object.entries(c.config).slice(0, 2);
  return (
    <span>
      {entries.map(([k, v]) => (
        <Tag key={k} style={{ marginInlineEnd: 4 }}>
          {k}={truncate(String(v))}
        </Tag>
      ))}
    </span>
  );
}

function errorsCell(_: unknown, c: ConnectorStatus) {
  const n = c.last_errors.length;
  if (n === 0) return <Tag color="green">0</Tag>;
  return (
    <Tooltip title={c.last_errors.join("\n")}>
      <Tag color="red">{n}</Tag>
    </Tooltip>
  );
}

/** 展开行:全量 config / fs_roots / load_error / 最近错误。 */
function expandedRowRender(c: ConnectorStatus) {
  return (
    <Descriptions size="small" column={1} bordered>
      <Descriptions.Item label="config">
        {c.config ? <pre style={{ margin: 0, fontSize: 12 }}>{JSON.stringify(c.config, null, 2)}</pre> : <Text type="secondary">—</Text>}
      </Descriptions.Item>
      {c.fs_roots.length > 0 && (
        <Descriptions.Item label="fs_roots">{c.fs_roots.map((r) => <code key={r} style={{ marginInlineEnd: 8 }}>{r}</code>)}</Descriptions.Item>
      )}
      {c.load_error && (
        <Descriptions.Item label="load_error"><Text type="danger">{c.load_error}</Text></Descriptions.Item>
      )}
      {c.last_synced_at && (
        <Descriptions.Item label="last_synced_at"><code>{c.last_synced_at}</code></Descriptions.Item>
      )}
      {(c.history ?? []).length > 0 && (
        <Descriptions.Item label={`近 ${c.history.length} 轮(耗时/错误趋势)`}>
          {(() => {
            const hist = c.history ?? [];
            const maxDur = Math.max(...hist.map((s) => s.duration_ms), 1);
            return (
              <div style={{ display: "flex", alignItems: "flex-end", gap: 2, height: 40 }}>
                {hist.map((s, i) => (
                  <Tooltip key={i} title={`${s.synced_at} · ${s.fact_count} facts · ${s.duration_ms}ms · ${s.error_count} err`}>
                    <div style={{
                      width: 10,
                      height: Math.max(4, Math.round((s.duration_ms / maxDur) * 40)),
                      background: s.error_count > 0 ? "#cf1322" : "#52c41a",
                      borderRadius: 1,
                    }} />
                  </Tooltip>
                ))}
              </div>
            );
          })()}
        </Descriptions.Item>
      )}
      {c.last_errors.length > 0 && (
        <Descriptions.Item label="last_errors">
          <ul style={{ margin: 0, paddingLeft: 18 }}>{c.last_errors.map((e, i) => <li key={i}><Text type="danger">{e}</Text></li>)}</ul>
        </Descriptions.Item>
      )}
    </Descriptions>
  );
}

/** ISO8601 -> "Ns ago" / "Nm ago" / "Nh ago";无值/非法 -> "—"。由 5s 轮询驱动刷新。 */
function relativeFrom(iso: string | null): string {
  if (!iso) return "—";
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "—";
  const sec = Math.max(0, Math.floor((Date.now() - t) / 1000));
  if (sec < 60) return `${sec}s ago`;
  if (sec < 3600) return `${Math.floor(sec / 60)}m ago`;
  return `${Math.floor(sec / 3600)}h ago`;
}

function truncate(s: string, n = 24): string {
  return s.length > n ? `${s.slice(0, n)}…` : s;
}
