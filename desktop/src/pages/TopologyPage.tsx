import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Button, Card, Space, Tag, message, Alert, Typography } from "antd";
import {
  TopologyView,
  type GraphResponse,
  HEALTH_LEVELS,
  RISK_LEVELS,
  healthFill,
  riskBorder,
  toggleStr,
} from "../views/TopologyView";
import NodeDetailPanel from "../components/Graph/NodeDetailPanel";
import {
  getGraph, listConnectors, syncAllNow, proxyStatus, startKubectlProxy, stopKubectlProxy,
  type ProxyStatusDto,
} from "../api/client";

/**
 * Phase 3.6 - 拓扑页(原 App.tsx 拓扑功能 + AntD + react-query + 节点详情)。
 *
 * 启动 get_graph(从 SQLite 恢复拓扑);「立即同步」sync_all_now 后回读;
 * 「Connect」托管 kubectl proxy 后 sync 一次;点节点 -> NodeDetailPanel(恢复动作 + 变更历史)。
 */
export default function TopologyPage() {
  const qc = useQueryClient();
  const { data: graph } = useQuery({ queryKey: ["graph"], queryFn: getGraph });
  const { data: connectors } = useQuery({ queryKey: ["connectors"], queryFn: listConnectors });
  const { data: proxy } = useQuery({ queryKey: ["proxy-status"], queryFn: proxyStatus });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [activeHealth, setActiveHealth] = useState<string[]>([]);
  const [activeRisk, setActiveRisk] = useState<string[]>([]);

  const syncM = useMutation({
    mutationFn: (cfg: string) => syncAllNow(cfg),
    onSuccess: async (s) => {
      message.success(`sync done · ${s.per_connector.length} connector · errors ${s.total_errors} · Δ +${s.changes.nodes_upserted}n/${s.changes.edges_upserted}e −${s.changes.nodes_removed}n/${s.changes.edges_removed}e`);
      await qc.invalidateQueries({ queryKey: ["graph"] });
    },
    onError: (e) => message.error(String(e)),
  });
  const connectM = useMutation({
    mutationFn: (port: number) => startKubectlProxy(port),
    onSuccess: async (p) => {
      message.success(p.running ? `proxy running · ${p.api_base}` : "proxy not running");
      await qc.invalidateQueries({ queryKey: ["proxy-status"] });
      if (p.running) syncM.mutate("{}");
    },
    onError: (e) => message.error(String(e)),
  });
  const disconnectM = useMutation({
    mutationFn: () => stopKubectlProxy(),
    onSuccess: async () => { await qc.invalidateQueries({ queryKey: ["proxy-status"] }); },
    onError: (e) => message.error(String(e)),
  });

  const selectedNode = selectedId ? graph?.nodes.find((n) => n.id === selectedId) ?? null : null;
  const summary = graph?.summary;

  return (
    <div>
      <Card style={{ marginBottom: 16 }}>
        <Space wrap>
          <Button
            type="primary"
            loading={syncM.isPending}
            disabled={(connectors?.length ?? 0) === 0}
            onClick={() => syncM.mutate("{}")}
          >
            Sync all now
          </Button>
          <Button
            loading={connectM.isPending}
            disabled={proxy?.running ?? false}
            onClick={() => connectM.mutate(8001)}
          >
            Connect (kubectl proxy)
          </Button>
          <Button
            loading={disconnectM.isPending}
            disabled={!(proxy?.running ?? false)}
            onClick={() => disconnectM.mutate()}
          >
            Disconnect
          </Button>
          {proxy && (
            <Tag color={proxy.running ? "green" : "default"}>
              {proxy.running ? `running · ${proxy.api_base} (pid ${proxy.pid ?? "?"})` : "proxy off"}
            </Tag>
          )}
          {(connectors?.length ?? 0) === 0 && (
            <Typography.Text type="warning">无 connector(先 `cd modules && cargo wasi-build`)</Typography.Text>
          )}
        </Space>
      </Card>

      {summary && (
        <Card title="拓扑视图" extra={<span>{summary.total_nodes} node · {summary.total_edges} edge · 点节点看详情</span>}>
          {graph && graph.nodes.length > 0 ? (
            <TopologyView
              graph={graph}
              onSelectNode={(id) => setSelectedId(id)}
              activeHealth={activeHealth}
              activeRisk={activeRisk}
            />
          ) : (
            <Alert type="info" showIcon message="等待同步" description="点 Sync all now 后这里渲染 Cytoscape 拓扑;重启后会从 SQLite 恢复。" />
          )}
          {summary && (
            <Space wrap style={{ marginTop: 12 }}>
              <Typography.Text type="secondary" style={{ fontSize: 12 }}>Health(点击过滤):</Typography.Text>
              {HEALTH_LEVELS.map((lvl) => {
                const v = summary.health_counts[lvl] ?? 0;
                const active = activeHealth.includes(lvl);
                return (
                  <Tag key={`h-${lvl}`} color={healthFill(lvl)}
                    style={{ cursor: "pointer", opacity: active ? 1 : 0.4 }}
                    onClick={() => setActiveHealth((a) => toggleStr(a, lvl))}>
                    {lvl} {v}
                  </Tag>
                );
              })}
              <Typography.Text type="secondary" style={{ fontSize: 12, marginLeft: 8 }}>Risk(点击过滤):</Typography.Text>
              {RISK_LEVELS.map((lvl) => {
                const v = summary.risk_counts[lvl] ?? 0;
                const active = activeRisk.includes(lvl);
                return (
                  <Tag key={`r-${lvl}`} color={riskBorder(lvl).color}
                    style={{ cursor: "pointer", opacity: active ? 1 : 0.4 }}
                    onClick={() => setActiveRisk((a) => toggleStr(a, lvl))}>
                    {lvl} {v}
                  </Tag>
                );
              })}
            </Space>
          )}
        </Card>
      )}

      <NodeDetailPanel node={selectedNode} open={!!selectedId} onClose={() => setSelectedId(null)} />
    </div>
  );
}
