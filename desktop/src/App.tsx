import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { TopologyView, type FactDto, type GraphResponse } from "./views/TopologyView";

/**
 * F + Phase 1 Step 2 + Phase 2.4 — Tauri ↔ engine-wasm 端到端 + Cytoscape 拓扑视图。
 *
 * 启动时:`get_app_version` + `list_connectors` + `get_graph`(从 SQLite 恢复拓扑)。
 * 用户点「立即同步」→ `sync_all_now`(config 带 `with_topology: true`,sync 后
 * upsert 到 SQLite)→ 再 `get_graph` 拉成图的 GraphResponse 渲染 Cytoscape。
 *
 * 2.4 起拓扑渲染走 `get_graph`(后端 `facts_to_graph` 已去重 / 连边 / 统计),
 * 前端不再 client 端解 Fact JSON。`get_topology` 仍保留供诊断。
 *
 * 设计:Phase 1 占位 UI,只用浏览器原生标签 + 朴素 CSS + cytoscape。Phase 2 起
 * 从 reference/frontend/src/ 把 antd Layout / 多视图组件迁进来。
 */

interface ConnectorInfo {
  name: string;
  version: string;
  kind: string;
  sync_interval_seconds: number;
  capabilities: string[];
}

interface ConnectorStatusDto {
  name: string;
  fact_count: number;
  errors: string[];
}

interface ChangeSummaryDto {
  nodes_upserted: number;
  nodes_removed: number;
  edges_upserted: number;
  edges_removed: number;
}

interface SyncSummaryDto {
  facts: FactDto[];
  per_connector: ConnectorStatusDto[];
  total_errors: number;
  total_duration_ms: number;
  changes: ChangeSummaryDto;
}

/** desktop 托管的 kubectl proxy 状态(Phase 2.7)。 */
interface ProxyStatusDto {
  running: boolean;
  port: number;
  api_base: string;
  pid: number | null;
  message: string;
}

export default function App() {
  const [version, setVersion] = useState<string>("loading...");
  const [bootErr, setBootErr] = useState<string | null>(null);
  const [connectors, setConnectors] = useState<ConnectorInfo[]>([]);
  const [summary, setSummary] = useState<SyncSummaryDto | null>(null);
  const [graph, setGraph] = useState<GraphResponse | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [syncErr, setSyncErr] = useState<string | null>(null);
  const [proxy, setProxy] = useState<ProxyStatusDto | null>(null);
  const [proxyBusy, setProxyBusy] = useState(false);
  const [proxyErr, setProxyErr] = useState<string | null>(null);

  useEffect(() => {
    invoke<string>("get_app_version")
      .then(setVersion)
      .catch((e) => setBootErr(`get_app_version: ${e}`));
    invoke<ConnectorInfo[]>("list_connectors")
      .then(setConnectors)
      .catch((e) => setBootErr(`list_connectors: ${e}`));
    invoke<GraphResponse>("get_graph")
      .then(setGraph)
      .catch((e) => setBootErr(`get_graph: ${e}`));
    invoke<ProxyStatusDto>("proxy_status")
      .then(setProxy)
      .catch((e) => setBootErr(`proxy_status: ${e}`));
  }, []);

  async function refreshGraph() {
    const g = await invoke<GraphResponse>("get_graph");
    setGraph(g);
  }

  async function handleSync() {
    setSyncing(true);
    setSyncErr(null);
    try {
      // Phase 2.7 起 per-connector config 在 manifest 里(k8s 拿 api_base、prom 拿
      // prometheus_url),这里传 "{}" 作全局兜底。无 manifest config 的 connector
      // 才回退用它。
      const configJson = "{}";
      const s = await invoke<SyncSummaryDto>("sync_all_now", {
        configJson,
      });
      setSummary(s);
      // sync_all_now 已把 facts upsert + resolve(+merge)->diff->apply 到 SQLite;
      // 回读成图的 GraphResponse 渲染。
      await refreshGraph();
    } catch (e) {
      setSyncErr(String(e));
    } finally {
      setSyncing(false);
    }
  }

  /** 启动 desktop 托管的 kubectl proxy,成功后顺手 sync 一次拉真集群拓扑。 */
  async function handleConnect() {
    setProxyBusy(true);
    setProxyErr(null);
    try {
      const p = await invoke<ProxyStatusDto>("start_kubectl_proxy", {
        port: 8001,
      });
      setProxy(p);
      if (p.running) {
        await handleSync();
      }
    } catch (e) {
      setProxyErr(String(e));
    } finally {
      setProxyBusy(false);
    }
  }

  async function handleDisconnect() {
    setProxyBusy(true);
    setProxyErr(null);
    try {
      const p = await invoke<ProxyStatusDto>("stop_kubectl_proxy");
      setProxy(p);
    } catch (e) {
      setProxyErr(String(e));
    } finally {
      setProxyBusy(false);
    }
  }

  return (
    <main
      style={{
        fontFamily: "system-ui, sans-serif",
        padding: "2rem",
        maxWidth: "1120px",
        margin: "0 auto",
        color: "#1f2328",
      }}
    >
      <h1 style={{ margin: 0 }}>SRE Inspection Graph</h1>
      <p style={{ color: "#666", marginTop: "0.25rem" }}>
        Phase 2.7 — 真集群拓扑 + kubectl proxy 托管 + metric health 合并
      </p>
      <p style={{ color: "#888", fontSize: "0.875rem" }}>
        engine-core 版本: <code>{version}</code>
      </p>
      {bootErr && (
        <p style={{ color: "crimson" }}>启动错误:{bootErr}</p>
      )}
      <hr />

      <section style={heroSectionStyle}>
        <div>
          <h2 style={{ marginTop: 0, marginBottom: "0.5rem" }}>集群连接</h2>
          <p style={{ marginTop: 0, color: "#666" }}>
            desktop 托管 <code>kubectl proxy --port=8001</code>(TLS+认证留 kubeconfig)。
            连接后 k8s connector 经 <code>api_base</code> 拉真集群拓扑,自动 sync 一次。
          </p>
          {proxy && (
            <p style={{ margin: 0, fontSize: "0.875rem" }}>
              <span
                style={{
                  display: "inline-block",
                  width: "0.6rem",
                  height: "0.6rem",
                  borderRadius: "50%",
                  background: proxy.running ? "#3fb950" : "#8b949e",
                  marginRight: "0.4rem",
                  verticalAlign: "middle",
                }}
              />
              {proxy.running
                ? `running · ${proxy.api_base} (pid ${proxy.pid ?? "?"})`
                : "not running"}
            </p>
          )}
          {proxyErr && (
            <p style={{ margin: "0.25rem 0 0", color: "crimson", fontSize: "0.85rem" }}>
              {proxyErr}
            </p>
          )}
        </div>
        <div style={{ display: "flex", gap: "0.5rem" }}>
          <button
            onClick={handleConnect}
            disabled={proxyBusy || (proxy?.running ?? false)}
            style={btnStyle(proxyBusy || (proxy?.running ?? false))}
          >
            {proxyBusy ? "..." : "Connect"}
          </button>
          <button
            onClick={handleDisconnect}
            disabled={proxyBusy || !(proxy?.running ?? false)}
            style={btnStyle(proxyBusy || !(proxy?.running ?? false))}
          >
            Disconnect
          </button>
        </div>
      </section>

      <section style={{ ...heroSectionStyle, marginTop: "0.75rem" }}>
        <div>
          <h2 style={{ marginTop: 0, marginBottom: "0.5rem" }}>拓扑同步</h2>
          <p style={{ marginTop: 0, color: "#666" }}>
            点击触发 <code>sync_all_now</code>:各 connector 按 manifest per-connector
            config 采集(k8s / prometheus),resolve + metric-health 合并后落 materialized
            拓扑,前端回读渲染。配色:shape=类型 / fill=health / border=risk。
          </p>
        </div>
        <button
          onClick={handleSync}
          disabled={syncing || connectors.length === 0}
          style={btnStyle(syncing || connectors.length === 0)}
        >
          {syncing ? "Syncing..." : "Sync all now"}
        </button>
      </section>
      {syncErr && <p style={{ color: "crimson" }}>sync 错误:{syncErr}</p>}

      {connectors.length === 0 && (
        <div style={emptyHintStyle}>
          <p style={{ margin: 0 }}>没有 connector 加载。可能是 wasm 还没 build:</p>
          <pre style={preStyle}>cd modules &amp;&amp; cargo wasi-build</pre>
          <p style={{ margin: 0, fontSize: "0.875rem", color: "#666" }}>
            build 完再重启 desktop。
          </p>
        </div>
      )}

      <section style={{ marginTop: "1rem" }}>
        <h2 style={{ marginTop: "1rem", marginBottom: "0.5rem" }}>拓扑视图</h2>
        {graph && graph.nodes.length > 0 ? (
          <>
            <p style={{ marginTop: 0, marginBottom: "0.75rem", color: "#666" }}>
              {graph.summary.total_nodes} node · {graph.summary.total_edges} edge
              {summary && (
                <>
                  {" "}· {summary.per_connector.length} connector · errors {summary.total_errors} ·{" "}
                  {summary.total_duration_ms}ms · Δ +{summary.changes.nodes_upserted}n/
                  {summary.changes.edges_upserted}e −{summary.changes.nodes_removed}n/
                  {summary.changes.edges_removed}e
                </>
              )}
            </p>
            <TopologyView graph={graph} />
          </>
        ) : summary ? (
          <p style={{ color: "#666" }}>
            <em>本轮没有 fact,拓扑空</em>
          </p>
        ) : (
          <div style={topologyPlaceholderStyle}>
            <strong>等待同步</strong>
            <span>点击 Sync all now 后,这里会渲染 Cytoscape 拓扑图;重启后会从 SQLite 恢复。</span>
          </div>
        )}
      </section>

      <section style={{ marginTop: "1rem" }}>
        <details>
          <summary style={summaryStyle}>Connector 诊断 ({connectors.length})</summary>
          {connectors.length > 0 && (
            <table style={tableStyle}>
              <thead>
                <tr>
                  <th style={thStyle}>name</th>
                  <th style={thStyle}>version</th>
                  <th style={thStyle}>kind</th>
                  <th style={thStyle}>interval (s)</th>
                  <th style={thStyle}>capabilities</th>
                </tr>
              </thead>
              <tbody>
                {connectors.map((c) => (
                  <tr key={c.name}>
                    <td style={tdStyle}>
                      <code>{c.name}</code>
                    </td>
                    <td style={tdStyle}>{c.version}</td>
                    <td style={tdStyle}>{c.kind}</td>
                    <td style={tdStyle}>{c.sync_interval_seconds}</td>
                    <td style={tdStyle}>{c.capabilities.length === 0 ? <em>—</em> : c.capabilities.join(", ")}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </details>

        {summary && (
          <>
            <details style={{ marginTop: "0.75rem" }} open>
              <summary style={summaryStyle}>Per-connector sync status</summary>
              <table style={tableStyle}>
                <thead>
                  <tr>
                    <th style={thStyle}>connector</th>
                    <th style={thStyle}>facts</th>
                    <th style={thStyle}>errors</th>
                  </tr>
                </thead>
                <tbody>
                  {summary.per_connector.map((s) => (
                    <tr key={s.name}>
                      <td style={tdStyle}>
                        <code>{s.name}</code>
                      </td>
                      <td style={tdStyle}>{s.fact_count}</td>
                      <td style={tdStyle}>{s.errors.length === 0 ? <em>—</em> : s.errors.join("; ")}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </details>

            <details style={{ marginTop: "0.75rem" }}>
              <summary style={summaryStyle}>Facts ({summary.facts.length})</summary>
              {summary.facts.length === 0 ? (
                <p style={{ color: "#666" }}>
                  <em>no facts</em>
                </p>
              ) : (
                <table style={tableStyle}>
                  <thead>
                    <tr>
                      <th style={thStyle}>id</th>
                      <th style={thStyle}>source</th>
                      <th style={thStyle}>resource_type</th>
                      <th style={thStyle}>resource_id</th>
                      <th style={thStyle}>ts</th>
                    </tr>
                  </thead>
                  <tbody>
                    {summary.facts.map((f) => (
                      <tr key={f.id}>
                        <td style={tdStyle}>
                          <code>{f.id}</code>
                        </td>
                        <td style={tdStyle}>{f.source}</td>
                        <td style={tdStyle}>{f.resource_type}</td>
                        <td style={tdStyle}>
                          <code>{f.resource_id}</code>
                        </td>
                        <td style={tdStyle}>{f.timestamp}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </details>
          </>
        )}
      </section>

      <p
        style={{
          marginTop: "3rem",
          color: "#999",
          fontSize: "0.875rem",
        }}
      >
        Phase 2.7:真集群拓扑已通(kubectl proxy 托管 + per-connector config +
        metric health 合并)。后续从 reference/frontend/src/ 迁入 MainLayout / 多视图。
      </p>
    </main>
  );
}

const heroSectionStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  gap: "1rem",
  marginTop: "1rem",
  padding: "1rem",
  border: "1px solid #d0d7de",
  borderRadius: "8px",
  background: "#f6f8fa",
};

/** 按钮统一样式;disabled 时灰化 + not-allowed。 */
function btnStyle(disabled: boolean): React.CSSProperties {
  return {
    padding: "0.6rem 1.1rem",
    fontSize: "1rem",
    cursor: disabled ? "not-allowed" : "pointer",
    opacity: disabled ? 0.5 : 1,
    whiteSpace: "nowrap",
  };
}

const topologyPlaceholderStyle: React.CSSProperties = {
  height: "480px",
  border: "1px dashed #d0d7de",
  borderRadius: "6px",
  background: "#fafbfc",
  display: "flex",
  flexDirection: "column",
  alignItems: "center",
  justifyContent: "center",
  gap: "0.4rem",
  color: "#6e7781",
};

const summaryStyle: React.CSSProperties = {
  cursor: "pointer",
  fontWeight: 600,
  padding: "0.5rem 0",
};

const tableStyle: React.CSSProperties = {
  width: "100%",
  borderCollapse: "collapse",
  fontSize: "0.9rem",
};
const thStyle: React.CSSProperties = {
  textAlign: "left",
  padding: "0.5rem",
  borderBottom: "2px solid #d0d7de",
  background: "#f6f8fa",
};
const tdStyle: React.CSSProperties = {
  padding: "0.4rem 0.5rem",
  borderBottom: "1px solid #eaeef2",
};
const emptyHintStyle: React.CSSProperties = {
  background: "#fff8c5",
  border: "1px solid #d4a72c",
  padding: "1rem",
  borderRadius: "6px",
};
const preStyle: React.CSSProperties = {
  background: "#0d1117",
  color: "#c9d1d9",
  padding: "0.5rem 0.75rem",
  borderRadius: "4px",
  fontSize: "0.85rem",
  margin: "0.5rem 0",
};
