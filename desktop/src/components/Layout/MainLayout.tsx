import { Layout, Menu, Typography } from "antd";
import { DashboardOutlined, ToolOutlined, ThunderboltOutlined, FileTextOutlined, AimOutlined, SettingOutlined, LinkOutlined, PictureOutlined, AlertOutlined, ApiOutlined } from "@ant-design/icons";
import { Link, Outlet, useLocation } from "react-router-dom";

const { Sider, Header, Content } = Layout;
const { Text } = Typography;

/**
 * Phase 3.6 - AntD Layout shell(移植自 reference `MainLayout.tsx`,精简到四视图)。
 *
 * Sider 菜单:Topology / Recovery / Changes / Reports / Connectors + 5 巡视视图
 * (node/config/access/image/alert)。(原 "Phase 4+" 死占位组已移除;Connectors 页 Phase 6
 *   落地 —— connector/handler 运行时观测,补「sync 状态只能看日志」的 dogfood 痛点。)
 * Content 走 react-router `<Outlet />`。Header 显示 app 名 + 版本(react-query 拉一次)。
 */
export default function MainLayout({ version }: { version: string }) {
  const location = useLocation();
  const selectedKey = location.pathname === "/" ? "topology" : location.pathname.slice(1);

  return (
    <Layout style={{ minHeight: "100vh" }}>
      <Sider breakpoint="lg" collapsedWidth="0" theme="light">
        <div style={{ padding: "1rem 1.1rem", fontWeight: 700, fontSize: "1.05rem" }}>
          SRE Graph
        </div>
        <Menu mode="inline" selectedKeys={[selectedKey]} items={[
          { key: "topology", icon: <DashboardOutlined />, label: <Link to="/">Topology</Link> },
          { key: "recovery", icon: <ToolOutlined />, label: <Link to="/recovery">Recovery</Link> },
          { key: "changes", icon: <ThunderboltOutlined />, label: <Link to="/changes">Changes</Link> },
          { key: "reports", icon: <FileTextOutlined />, label: <Link to="/reports">Reports</Link> },
          { key: "connectors", icon: <ApiOutlined />, label: <Link to="/connectors">Connectors</Link> },
          { key: "node-impact", icon: <AimOutlined />, label: <Link to="/node-impact">Node Impact</Link> },
          { key: "config-impact", icon: <SettingOutlined />, label: <Link to="/config-impact">Config Impact</Link> },
          { key: "access-link", icon: <LinkOutlined />, label: <Link to="/access-link">Access Link</Link> },
          { key: "image-risk", icon: <PictureOutlined />, label: <Link to="/image-risk">Image Risk</Link> },
          { key: "alert-aggregation", icon: <AlertOutlined />, label: <Link to="/alert-aggregation">Alert Aggregation</Link> },
        ]} />
      </Sider>
      <Layout>
        <Header style={{ background: "#fff", padding: "0 1.25rem", display: "flex",
          alignItems: "center", justifyContent: "space-between", borderBottom: "1px solid #f0f0f0" }}>
          <Text strong>SRE Inspection Graph</Text>
          <Text type="secondary" style={{ fontSize: "0.85rem" }}>
            engine-core <code>{version}</code>
          </Text>
        </Header>
        <Content style={{ padding: "1.25rem" }}>
          <Outlet />
        </Content>
      </Layout>
    </Layout>
  );
}
