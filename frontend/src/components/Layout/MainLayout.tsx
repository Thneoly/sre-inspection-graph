import { useState } from 'react';
import { Outlet, useNavigate, useLocation } from 'react-router-dom';
import { Layout, Menu, Typography, theme } from 'antd';
import type { MenuProps } from 'antd';
import {
  ApartmentOutlined,
  LinkOutlined,
  RadiusSettingOutlined,
  SettingOutlined,
  ContainerOutlined,
  AlertOutlined,
  HistoryOutlined,
  AuditOutlined,
} from '@ant-design/icons';

const { Sider, Content, Header: AntHeader } = Layout;

const menuItems: MenuProps['items'] = [
  { key: '/topology',          icon: <ApartmentOutlined />,   label: '应用拓扑' },
  { key: '/access-link',       icon: <LinkOutlined />,        label: '访问链路' },
  { key: '/node-impact',       icon: <RadiusSettingOutlined />, label: '节点影响' },
  { key: '/config-impact',     icon: <SettingOutlined />,     label: '配置影响' },
  { key: '/image-risk',        icon: <ContainerOutlined />,   label: '镜像风险' },
  { key: '/alert-aggregation', icon: <AlertOutlined />,       label: '告警归并' },
  { type: 'divider' },
  { key: '/recovery/approvals', icon: <AuditOutlined />,      label: '审批中心' },
  { key: '/recovery/history',  icon: <HistoryOutlined />,     label: '恢复历史' },
];

export default function MainLayout() {
  const [collapsed, setCollapsed] = useState(false);
  const navigate = useNavigate();
  const location = useLocation();
  const { token } = theme.useToken();

  return (
    <Layout style={{ height: '100vh', overflow: 'hidden' }}>
      <Sider
        collapsible
        collapsed={collapsed}
        onCollapse={setCollapsed}
        theme="light"
        width={200}
        style={{ borderRight: `1px solid ${token.colorBorderSecondary}` }}
      >
        <div style={{
          height: 36, display: 'flex', alignItems: 'center', justifyContent: 'center',
          color: token.colorPrimary, fontWeight: 700, fontSize: collapsed ? 14 : 16,
          whiteSpace: 'nowrap', overflow: 'hidden',
        }}>
          {collapsed ? '🔍' : '🔍 SRE 巡检图谱'}
        </div>
        <Menu
          theme="light"
          mode="inline"
          selectedKeys={[location.pathname]}
          items={menuItems}
          onClick={({ key }) => navigate(key)}
        />
      </Sider>
      <Layout style={{ overflow: 'hidden' }}>
        <AntHeader style={{
          height: 36, lineHeight: '36px', padding: '0 16px',
          background: token.colorBgContainer, borderBottom: `1px solid ${token.colorBorderSecondary}`,
          display: 'flex', alignItems: 'center',
        }}>
          <Typography.Text strong style={{ fontSize: 14 }}>
            Cloud Native Inspection Graph
          </Typography.Text>
        </AntHeader>
        <Content style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', overflow: 'hidden', background: token.colorBgLayout }}>
          <Outlet />
        </Content>
      </Layout>
    </Layout>
  );
}
