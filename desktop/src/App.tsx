import { useQuery } from "@tanstack/react-query";
import { Routes, Route } from "react-router-dom";
import MainLayout from "./components/Layout/MainLayout";
import TopologyPage from "./pages/TopologyPage";
import RecoveryPage from "./pages/RecoveryPage";
import ChangesPage from "./pages/ChangesPage";
import ReportsPage from "./pages/ReportsPage";
import ConnectorsPage from "./pages/ConnectorsPage";
import NodeImpactPage from "./pages/NodeImpactPage";
import ConfigImpactPage from "./pages/ConfigImpactPage";
import AccessLinkPage from "./pages/AccessLinkPage";
import ImageRiskPage from "./pages/ImageRiskPage";
import AlertAggregationPage from "./pages/AlertAggregationPage";
import { getAppVersion } from "./api/client";

/**
 * Phase 3.6 - router shell。HashRouter(main.tsx 包)下嵌套布局路由:父路由
 * 渲染 `MainLayout`(Sider 菜单 + Outlet),子路由 Topology / Recovery / Changes /
 * Reports 渲染进 Outlet。版本经 react-query 拉一次喂给 Header。
 */
export default function App() {
  const { data: version } = useQuery({ queryKey: ["app-version"], queryFn: getAppVersion });
  return (
    <Routes>
      <Route element={<MainLayout version={version ?? "loading..."} />}>
        <Route path="/" element={<TopologyPage />} />
        <Route path="/recovery" element={<RecoveryPage />} />
        <Route path="/changes" element={<ChangesPage />} />
        <Route path="/reports" element={<ReportsPage />} />
        <Route path="/connectors" element={<ConnectorsPage />} />
        <Route path="/node-impact" element={<NodeImpactPage />} />
        <Route path="/config-impact" element={<ConfigImpactPage />} />
        <Route path="/access-link" element={<AccessLinkPage />} />
        <Route path="/image-risk" element={<ImageRiskPage />} />
        <Route path="/alert-aggregation" element={<AlertAggregationPage />} />
      </Route>
    </Routes>
  );
}
