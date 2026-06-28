import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { TopologyView, type FactDto } from "./views/TopologyView";

/**
 * F + Phase 1 Step 2 — Tauri ↔ engine-wasm 串通的端到端验证页 + Cytoscape 拓扑视图。
 *
 * 启动时:`get_app_version` + `list_connectors` 各调一次。
 * 用户点「立即同步」→ `sync_all_now`(config 带 `with_topology: true` 让
 * k8s-mini 吐分层 mock 拓扑)→ 渲染 Cytoscape 视图 + per-connector 表 + Fact 表。
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

interface SyncSummaryDto {
  facts: FactDto[];
  per_connector: ConnectorStatusDto[];
  total_errors: number;
  total_duration_ms: number;
}

export default function App() {
  const [version, setVersion] = useState<string>("loading...");
  const [bootErr, setBootErr] = useState<string | null>(null);
  const [connectors, setConnectors] = useState<ConnectorInfo[]>([]);
  const [summary, setSummary] = useState<SyncSummaryDto | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [syncErr, setSyncErr] = useState<string | null>(null);

  useEffect(() => {
    invoke<string>("get_app_version")
      .then(setVersion)
      .catch((e) => setBootErr(`get_app_version: ${e}`));
    invoke<ConnectorInfo[]>("list_connectors")
      .then(setConnectors)
      .catch((e) => setBootErr(`list_connectors: ${e}`));
  }, []);

  async function handleSync() {
    setSyncing(true);
    setSyncErr(null);
    try {
      // 给 k8s-mini 传 with_topology=true,让它吐分层 mock(Cluster + Node +
      // Namespace + Pod + Service);hello-world 不读 config,这段对它无害。
      // Tauri 2.x 自动把 JS camelCase 转 Rust snake_case;此处用 snake 也可,
      // 显式保险一些。
      const configJson = JSON.stringify({
        cluster: "demo",
        namespaces: ["default", "app"],
        with_topology: true,
      });
      const s = await invoke<SyncSummaryDto>("sync_all_now", {
        configJson,
      });
      setSummary(s);
    } catch (e) {
      setSyncErr(String(e));
    } finally {
      setSyncing(false);
    }
  }

  return (
    <main
      style={{
        fontFamily: "system-ui, sans-serif",
        padding: "2rem",
        maxWidth: "960px",
        margin: "0 auto",
        color: "#1f2328",
      }}
    >
      <h1 style={{ margin: 0 }}>SRE Inspection Graph</h1>
      <p style={{ color: "#666", marginTop: "0.25rem" }}>
        Phase 1 — Tauri ↔ engine-wasm 桥接
      </p>
      <p style={{ color: "#888", fontSize: "0.875rem" }}>
        engine-core 版本: <code>{version}</code>
      </p>
      {bootErr && (
        <p style={{ color: "crimson" }}>启动错误:{bootErr}</p>
      )}
      <hr />

      <section>
        <h2 style={{ marginTop: "1.5rem" }}>
          已加载 Connector ({connectors.length})
        </h2>
        {connectors.length === 0 ? (
          <div style={emptyHintStyle}>
            <p style={{ margin: 0 }}>
              没有 connector 加载。可能是 wasm 还没 build:
            </p>
            <pre style={preStyle}>cd modules &amp;&amp; cargo wasi-build</pre>
            <p
              style={{ margin: 0, fontSize: "0.875rem", color: "#666" }}
            >
              build 完再重启 desktop。
            </p>
          </div>
        ) : (
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
                  <td style={tdStyle}>
                    {c.capabilities.length === 0 ? (
                      <em>—</em>
                    ) : (
                      c.capabilities.join(", ")
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>

      <section>
        <h2 style={{ marginTop: "1.5rem" }}>立即同步</h2>
        <button
          onClick={handleSync}
          disabled={syncing || connectors.length === 0}
          style={{
            padding: "0.5rem 1rem",
            fontSize: "1rem",
            cursor: connectors.length === 0 ? "not-allowed" : "pointer",
            opacity: connectors.length === 0 ? 0.5 : 1,
          }}
        >
          {syncing ? "Syncing..." : "Sync all now"}
        </button>
        {syncErr && (
          <p style={{ color: "crimson" }}>sync 错误:{syncErr}</p>
        )}

        {summary && (
          <div style={{ marginTop: "1rem" }}>
            <p style={{ marginBottom: "0.5rem" }}>
              <strong>总览:</strong> {summary.facts.length} fact ·{" "}
              {summary.per_connector.length} connector · errors{" "}
              {summary.total_errors} · {summary.total_duration_ms}ms
            </p>

            <h3 style={{ marginTop: "1rem", marginBottom: "0.5rem" }}>
              拓扑视图
            </h3>
            {summary.facts.length === 0 ? (
              <p style={{ color: "#666" }}>
                <em>本轮没有 fact,拓扑空</em>
              </p>
            ) : (
              <TopologyView facts={summary.facts} />
            )}

            <h3 style={{ marginTop: "1rem", marginBottom: "0.5rem" }}>
              Per-connector
            </h3>
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
                    <td style={tdStyle}>
                      {s.errors.length === 0 ? (
                        <em>—</em>
                      ) : (
                        s.errors.join("; ")
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>

            <h3 style={{ marginTop: "1rem", marginBottom: "0.5rem" }}>
              Facts
            </h3>
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
          </div>
        )}
      </section>

      <p
        style={{
          marginTop: "3rem",
          color: "#999",
          fontSize: "0.875rem",
        }}
      >
        Phase 2 起从 reference/frontend/src/ 迁入 MainLayout / 多视图 / 真 K8s
        connector。
      </p>
    </main>
  );
}

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
