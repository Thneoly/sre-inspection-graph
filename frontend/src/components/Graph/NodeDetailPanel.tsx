import { useQuery } from '@tanstack/react-query';
import { Drawer, Descriptions, Tag, Typography, Space, Card, Statistic } from 'antd';
import { CheckCircleOutlined, WarningOutlined, CloseCircleOutlined, LinkOutlined } from '@ant-design/icons';
import { fetchResourceMetrics } from '../../api/client';
import type { GraphResponse } from '../../api/client';
import RecoveryActionsSection from '../Recovery/RecoveryActionsSection';
import ChangeTimelineSection from './ChangeTimelineSection';

interface DetailPanelProps {
  selectedId: string | null;
  nodeType?: string;
  nodeProperties?: Record<string, unknown>;
  isEdge?: boolean;
  allNodes?: GraphResponse['nodes'];
  allEdges?: GraphResponse['edges'];
  onClose: () => void;
}

const healthConfig: Record<string, { color: string; icon: React.ReactNode }> = {
  normal:   { color: 'success', icon: <CheckCircleOutlined /> },
  warning:  { color: 'warning', icon: <WarningOutlined /> },
  critical: { color: 'error',   icon: <CloseCircleOutlined /> },
};
const riskConfig: Record<string, string> = {
  low: 'success', medium: 'warning', high: 'error', critical: 'red',
};

export default function NodeDetailPanel({ selectedId, nodeType, nodeProperties, isEdge, allNodes, allEdges, onClose }: DetailPanelProps) {
  const { data: metricsData } = useQuery({
    queryKey: ['metrics', selectedId],
    queryFn: () => fetchResourceMetrics(selectedId!).then(r => r.data),
    enabled: !!selectedId && !isEdge,
  });

  const health = String(nodeProperties?.health_status || 'normal');
  const risk = String(nodeProperties?.risk_level || 'low');
  const hc = healthConfig[health] || healthConfig.normal;
  const type = nodeType || '';

  if (!selectedId) return null;

  return (
    <Drawer
      title={
        <Space>
          <Tag color={isEdge ? 'processing' : hc.color} icon={isEdge ? <LinkOutlined /> : hc.icon}>
            {isEdge ? '关系' : type}
          </Tag>
          <Typography.Text strong>
            {isEdge
              ? String(nodeProperties?.relationship_name || nodeProperties?.relationship_type || selectedId)
              : String(nodeProperties?.name || selectedId)}
          </Typography.Text>
        </Space>
      }
      open={!!selectedId}
      onClose={onClose}
      width={isEdge ? 320 : 460}
      placement="right"
      mask={false}
      styles={{ body: { padding: 16 } }}
    >
      {isEdge ? (
        <>
          <Descriptions column={1} size="small" bordered style={{ marginBottom: 16 }}>
            <Descriptions.Item label="关系类型">
              <Tag color="processing">{nodeType || String(nodeProperties?.relationship_type || '-')}</Tag>
            </Descriptions.Item>
            <Descriptions.Item label="名称">{String(nodeProperties?.relationship_name || '-')}</Descriptions.Item>
            <Descriptions.Item label="依赖强度">{String(nodeProperties?.dependency_strength || '-')}</Descriptions.Item>
            <Descriptions.Item label="必需">{String(nodeProperties?.is_required || '-')}</Descriptions.Item>
            {nodeProperties?.risk_signal && (
              <Descriptions.Item label="风险信号">
                <Tag color="warning">{String(nodeProperties.risk_signal)}</Tag>
              </Descriptions.Item>
            )}
            <Descriptions.Item label="健康状态">
              <Tag color={hc.color}>{health}</Tag>
            </Descriptions.Item>
          </Descriptions>

          {/* Source & Target nodes */}
          {(() => {
            const edge = allEdges?.find(e => e.id === selectedId);
            if (!edge) return null;
            const srcNode = allNodes?.find(n => n.id === edge.source);
            const tgtNode = allNodes?.find(n => n.id === edge.target);
            return (
              <Card title="关系双方" size="small" style={{ marginBottom: 16 }}>
                <Descriptions column={1} size="small">
                  <Descriptions.Item label="源 (Source)">
                    <Tag>{srcNode?.type || edge.source}</Tag>
                    <Typography.Text style={{ fontSize: 12 }}>{srcNode?.properties?.name || edge.source}</Typography.Text>
                  </Descriptions.Item>
                  <Descriptions.Item label="目标 (Target)">
                    <Tag>{tgtNode?.type || edge.target}</Tag>
                    <Typography.Text style={{ fontSize: 12 }}>{tgtNode?.properties?.name || edge.target}</Typography.Text>
                  </Descriptions.Item>
                </Descriptions>
              </Card>
            );
          })()}
          <Card title="全部属性" size="small">
            <pre style={{ fontSize: 11, maxHeight: 300, overflow: 'auto', background: '#f5f5f5', padding: 8, borderRadius: 4 }}>
              {JSON.stringify(nodeProperties, null, 2)}
            </pre>
          </Card>
        </>
      ) : (
        <>
          <Descriptions column={2} size="small" bordered style={{ marginBottom: 16 }}>
            <Descriptions.Item label="ID" span={2}>
              <Typography.Text copyable code style={{ fontSize: 11 }}>{selectedId}</Typography.Text>
            </Descriptions.Item>
            <Descriptions.Item label="健康状态">
              <Tag color={hc.color}>{health}</Tag>
            </Descriptions.Item>
            <Descriptions.Item label="风险等级">
              <Tag color={riskConfig[risk] || 'default'}>{risk}</Tag>
            </Descriptions.Item>
            {nodeProperties?.owner_team && (
              <Descriptions.Item label="负责团队">{String(nodeProperties.owner_team)}</Descriptions.Item>
            )}
            {nodeProperties?.cluster_id && (
              <Descriptions.Item label="集群">{String(nodeProperties.cluster_id)}</Descriptions.Item>
            )}
            {nodeProperties?.namespace && (
              <Descriptions.Item label="命名空间">{String(nodeProperties.namespace)}</Descriptions.Item>
            )}
            {nodeProperties?.pod_ip && (
              <Descriptions.Item label="Pod IP">
                <Typography.Text code>{String(nodeProperties.pod_ip)}</Typography.Text>
              </Descriptions.Item>
            )}
            {nodeProperties?.phase && (
              <Descriptions.Item label="Phase">{String(nodeProperties.phase)}</Descriptions.Item>
            )}
            {nodeProperties?.restart_count !== undefined && (
              <Descriptions.Item label="重启次数">{String(nodeProperties.restart_count)}</Descriptions.Item>
            )}
            {nodeProperties?.ready !== undefined && (
              <Descriptions.Item label="Ready">{String(nodeProperties.ready)}</Descriptions.Item>
            )}
          </Descriptions>

          {metricsData && metricsData.metrics.length > 0 && (
            <Card title="实时指标" size="small" style={{ marginBottom: 16 }}>
              <Space wrap>
                {metricsData.metrics.map((m) => (
                  <Card key={m.id} size="small" style={{ width: 100, textAlign: 'center' }} styles={{ body: { padding: 8 } }}>
                    <Statistic
                      title={m.metric_name}
                      value={m.current_value}
                      precision={1}
                      suffix={m.unit === 'percent' ? '%' : m.unit}
                      valueStyle={{ fontSize: 18, color: m.critical_breached ? '#ff4d4f' : m.warning_breached ? '#faad14' : '#52c41a' }}
                    />
                  </Card>
                ))}
              </Space>
            </Card>
          )}

          {/* 快恢动作区(PRD-001 Sprint 1+2) */}
          {type && <RecoveryActionsSection resourceId={selectedId} resourceType={type} />}

          {/* 变更时间线(PRD-002 Sprint 2) */}
          {selectedId && (
            <Card title="📅 变更时间线(近 50 条)" size="small" style={{ marginTop: 12, marginBottom: 12 }}>
              <ChangeTimelineSection resourceId={selectedId} />
            </Card>
          )}

          <Card title="全部属性" size="small">
            <pre style={{ fontSize: 11, maxHeight: 300, overflow: 'auto', background: '#f5f5f5', padding: 8, borderRadius: 4 }}>
              {JSON.stringify(nodeProperties, null, 2)}
            </pre>
          </Card>
        </>
      )}
    </Drawer>
  );
}
