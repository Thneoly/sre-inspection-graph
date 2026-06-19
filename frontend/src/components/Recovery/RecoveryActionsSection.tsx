import { useState, useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Card, Button, Space, Tag, Typography, Empty, Spin } from 'antd';
import { ToolOutlined, EyeOutlined } from '@ant-design/icons';
import {
  fetchRecoveryActions,
  type RecoveryAction,
  type RiskLevel,
} from '../../api/client';
import DryRunModal from './DryRunModal';

const { Text } = Typography;

interface Props {
  /** 节点 ID(node_id),如 deploy:cce-prod-01:order:order-api */
  resourceId: string;
  /** 节点类型(label),如 Deployment / Pod / Service / MySQL / Redis / Secret / KubernetesNode */
  resourceType: string;
  /** 可选的 finding_id,从 NodeDetailPanel 传入(目前还没集成到节点详情,留接口) */
  findingId?: string;
}

const riskTagColor: Record<RiskLevel, string> = {
  low: 'success',
  medium: 'warning',
  high: 'error',
};

export default function RecoveryActionsSection({ resourceId, resourceType, findingId }: Props) {
  const [selectedAction, setSelectedAction] = useState<RecoveryAction | null>(null);
  const [modalOpen, setModalOpen] = useState(false);

  // 查询所有动作,按 target_resource_type 过滤
  const { data, isLoading } = useQuery({
    queryKey: ['recovery-actions', resourceType],
    queryFn: () => fetchRecoveryActions({ target_type: resourceType }).then((r) => r.data),
    enabled: !!resourceType,
  });

  const actions = useMemo(() => data?.actions || [], [data]);

  const openDryRun = (action: RecoveryAction) => {
    setSelectedAction(action);
    setModalOpen(true);
  };

  if (isLoading) {
    return (
      <Card
        title={<Space><ToolOutlined />快恢动作</Space>}
        size="small"
        style={{ marginBottom: 16 }}
      >
        <Spin size="small" />
      </Card>
    );
  }

  if (!actions.length) {
    return (
      <Card
        title={<Space><ToolOutlined />快恢动作</Space>}
        size="small"
        style={{ marginBottom: 16 }}
      >
        <Empty
          imageStyle={{ height: 40 }}
          description={
            <Text type="secondary" style={{ fontSize: 12 }}>
              此资源类型({resourceType})无可用恢复动作
            </Text>
          }
        />
      </Card>
    );
  }

  return (
    <>
      <Card
        title={
          <Space>
            <ToolOutlined />
            快恢动作
            <Tag>{actions.length} 个可用</Tag>
          </Space>
        }
        size="small"
        style={{ marginBottom: 16 }}
      >
        <Space direction="vertical" style={{ width: '100%' }} size={6}>
          {actions.map((a) => (
            <div
              key={a.action_id}
              style={{
                padding: '8px 10px',
                border: '1px solid #f0f0f0',
                borderRadius: 4,
                background: '#fafafa',
              }}
            >
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                <Space size={4}>
                  <Text strong style={{ fontSize: 13 }}>{a.action_name}</Text>
                  <Tag color={riskTagColor[a.risk_level]}>{a.risk_level}</Tag>
                  {a.requires_approval && <Tag color="orange">审批</Tag>}
                </Space>
                <Button
                  size="small"
                  type="primary"
                  ghost
                  icon={<EyeOutlined />}
                  onClick={() => openDryRun(a)}
                >
                  预演
                </Button>
              </div>
              <Text type="secondary" style={{ fontSize: 11 }}>
                ~{a.estimated_duration_seconds}s · SLA {a.sla_impact_estimate} · {a.description.split('。')[0]}
              </Text>
            </div>
          ))}
        </Space>
      </Card>

      <DryRunModal
        open={modalOpen}
        action={selectedAction}
        targetResourceId={resourceId}
        findingId={findingId}
        onClose={() => setModalOpen(false)}
      />
    </>
  );
}
