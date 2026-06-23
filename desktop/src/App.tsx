import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/**
 * Phase 1 — 占位 App。仅演示 Tauri command 通信链路:
 * 调用 backend `get_app_version` → 返回 engine-core 版本号。
 *
 * Phase 2 起从 frontend/src/ 迁入 MainLayout / Views / Graph 组件。
 */
export default function App() {
  const [version, setVersion] = useState<string>("loading...");
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    invoke<string>("get_app_version")
      .then(setVersion)
      .catch((e) => setErr(String(e)));
  }, []);

  return (
    <main
      style={{
        fontFamily: "system-ui, sans-serif",
        padding: "2rem",
        maxWidth: "640px",
        margin: "0 auto",
      }}
    >
      <h1>SRE Inspection Graph</h1>
      <p style={{ color: "#666" }}>Phase 1 — Tauri 桌面骨架</p>
      <hr />
      <p>
        <strong>engine-core 版本:</strong>
        <code style={{ marginLeft: "0.5rem" }}>{version}</code>
      </p>
      {err && <p style={{ color: "crimson" }}>错误:{err}</p>}
      <p style={{ marginTop: "2rem", color: "#999", fontSize: "0.875rem" }}>
        Phase 2 将迁移 frontend/src/ 视图(6 巡检 + 4 PRD)进来。
      </p>
    </main>
  );
}
