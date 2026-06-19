import { Routes, Route, Navigate } from 'react-router-dom';
import MainLayout from './components/Layout/MainLayout';
import TopologyView from './components/Views/TopologyView';
import AccessLinkView from './components/Views/AccessLinkView';
import NodeImpactView from './components/Views/NodeImpactView';
import ConfigImpactView from './components/Views/ConfigImpactView';
import ImageRiskView from './components/Views/ImageRiskView';
import AlertAggregationView from './components/Views/AlertAggregationView';
import SimulationView from './components/Views/SimulationView';
import ExecutionsView from './components/Recovery/ExecutionsView';
import ApprovalsView from './components/Recovery/ApprovalsView';

export default function App() {
  return (
    <Routes>
      <Route element={<MainLayout />}>
        <Route path="/" element={<Navigate to="/topology" replace />} />
        <Route path="/topology" element={<TopologyView />} />
        <Route path="/access-link" element={<AccessLinkView />} />
        <Route path="/node-impact" element={<NodeImpactView />} />
        <Route path="/config-impact" element={<ConfigImpactView />} />
        <Route path="/image-risk" element={<ImageRiskView />} />
        <Route path="/alert-aggregation" element={<AlertAggregationView />} />
        <Route path="/recovery/approvals" element={<ApprovalsView />} />
        <Route path="/recovery/history" element={<ExecutionsView />} />
      </Route>
      <Route path="/simulation" element={<SimulationView />} />
    </Routes>
  );
}
