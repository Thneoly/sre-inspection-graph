import { useState, useEffect } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Select, Button, Card, Space, Tag, Typography, Divider, Slider, message, Descriptions, Badge } from 'antd';
import { ThunderboltOutlined, ForwardOutlined, ReloadOutlined, ArrowLeftOutlined } from '@ant-design/icons';
import { useNavigate } from 'react-router-dom';
import api, { type GraphResponse } from '../../api/client';
import GraphCanvas from '../Graph/GraphCanvas';
import GraphStatsBar from '../Graph/GraphStatsBar';
import NodeDetailPanel from '../Graph/NodeDetailPanel';

// ── API helpers ──
async function fetchTypes() {
  const { data } = await api.get('/datasource/fault-types');
  return data.types as Record<string, { name: string; category: string; target_type: string; stages: number; duration_s: number }>;
}

async function fetchStatus() {
  const { data } = await api.get('/datasource/fault-status');
  return data as { active: Record<string, unknown>[]; active_count: number; unhealthy_nodes: Record<string, string>[] };
}

async function injectFault(ft: string, tid: string) {
  const { data } = await api.post('/datasource/inject-fault', { fault_type: ft, target_id: tid });
  return data;
}

async function stepSim(seconds: number) {
  const { data } = await api.post(`/datasource/step?seconds=${seconds}`);
  return data;
}

async function resetSim() {
  const { data } = await api.post('/datasource/reset');
  return data;
}

async function fetchTopology(appCode: string, depth: number) {
  const { data } = await api.get<GraphResponse>(`/topology/app/${appCode}`, { params: { depth } });
  return data;
}

// ── Component ──
export default function SimulationView() {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const [faultType, setFaultType] = useState<string>('cpu_spike');
  const [targetId, setTargetId] = useState('pod:cce-prod-01:order:order-api-6fd9c8b7c9-abcdf');
  const [stepSec, setStepSec] = useState(300);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [isEdge, setIsEdge] = useState(false);

  const { data: types } = useQuery({ queryKey: ['sim-types'], queryFn: fetchTypes });
  const { data: simStatus, refetch: refetchStatus } = useQuery({ queryKey: ['sim-status'], queryFn: fetchStatus, refetchInterval: 5000 });
  const { data: topoData } = useQuery({ queryKey: ['topology', 'order', 5], queryFn: () => fetchTopology('order', 5) });

  const injectMut = useMutation({ mutationFn: () => injectFault(faultType, targetId), onSuccess: () => { refetchStatus(); qc.invalidateQueries({ queryKey: ['topology'] }); message.success('故障已注入'); } });
  const stepMut = useMutation({ mutationFn: () => stepSim(stepSec), onSuccess: () => { refetchStatus(); qc.invalidateQueries({ queryKey: ['topology'] }); message.success(`推进 ${stepSec}s`); } });
  const resetMut = useMutation({ mutationFn: resetSim, onSuccess: () => { refetchStatus(); qc.invalidateQueries({ queryKey: ['topology'] }); message.success('已重置'); } });

  const selectedNode = !isEdge ? topoData?.nodes.find(n => n.id === selectedId) : undefined;
  const selectedEdge = isEdge ? topoData?.edges.find(e => e.id === selectedId) : undefined;

  // Target options: filter by fault type's target_type
  const targetType = types?.[faultType]?.target_type || '';
  const targetOptions = (topoData?.nodes || [])
    .filter(n => !targetType || n.type === targetType)
    .map(n => ({
      value: n.id,
      label: `${n.type} | ${n.properties.name || n.id}`,
    }));

  // Auto-select first compatible target when fault type changes
  const validTargets = targetOptions.map(o => o.value);
  const isTargetValid = validTargets.includes(targetId);

  // Auto-select first compatible target when fault type changes
  useEffect(() => {
    if (validTargets.length > 0 && !isTargetValid) {
      setTargetId(validTargets[0]);
    }
  }, [faultType, validTargets.length]);

  const typeInfo = types?.[faultType];

  return (
    <div style={{ height: '100vh', display: 'flex', flexDirection: 'column', overflow: 'hidden', background: '#fff' }}>
      {/* Toolbar */}
      <div style={{ padding: '8px 16px', borderBottom: '1px solid #f0f0f0', background: '#fafafa', display: 'flex', gap: 24, alignItems: 'center', flexWrap: 'wrap' }}>
        <Button icon={<ArrowLeftOutlined />} onClick={() => navigate('/topology')}>返回</Button>
        <Divider type="vertical" />
        {/* Fault type */}
        <Space>
          <Typography.Text strong>故障类型:</Typography.Text>
          <Select value={faultType} onChange={setFaultType} style={{ width: 180 }}
            options={types ? Object.entries(types).map(([k, v]) => ({ value: k, label: v.name })) : []} />
        </Space>

        {/* Target */}
        <Space>
          <Typography.Text strong>目标:</Typography.Text>
          <Select value={targetId} onChange={setTargetId} style={{ width: 420 }} showSearch
            optionFilterProp="label" options={targetOptions} />
        </Space>

        {/* Inject */}
        <Button type="primary" icon={<ThunderboltOutlined />} loading={injectMut.isPending} onClick={() => injectMut.mutate()}
          disabled={!faultType || !targetId || !isTargetValid}>
          注入故障
        </Button>
        {!isTargetValid && targetType && (
          <Typography.Text type="danger">
            目标类型不匹配：故障需要 <Tag>{targetType}</Tag>，当前选中不是{targetType}
          </Typography.Text>
        )}

        <Divider type="vertical" />

        {/* Step */}
        <Space>
          <Typography.Text>推进:</Typography.Text>
          <Slider min={60} max={1800} step={60} value={stepSec} onChange={setStepSec} style={{ width: 120 }}
            tooltip={{ formatter: (v) => `${Math.floor((v || 0) / 60)}m` }} />
          <Button icon={<ForwardOutlined />} loading={stepMut.isPending} onClick={() => stepMut.mutate()}>
            +{Math.floor(stepSec / 60)}m
          </Button>
        </Space>

        {/* Reset */}
        <Button danger icon={<ReloadOutlined />} loading={resetMut.isPending} onClick={() => resetMut.mutate()}>
          重置
        </Button>

        {/* Status badges */}
        <Space>
          <Badge status={simStatus?.active_count ? 'error' : 'success'} />
          <Typography.Text type="secondary">
            {simStatus?.active_count ? `${simStatus.active_count} 活跃故障` : '无活跃故障'}
          </Typography.Text>
          {simStatus?.unhealthy_nodes?.length ? (
            <Tag color="error">{simStatus.unhealthy_nodes.length} 异常节点</Tag>
          ) : null}
        </Space>
      </div>

      {/* Main area: graph + detail panel */}
      <div style={{ display: 'flex', flex: 1, overflow: 'hidden' }}>
        {/* Left: graph */}
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0, minHeight: 0 }}>
          <GraphCanvas
            data={topoData}
            isLoading={!topoData}
            onNodeSelect={(id) => { setSelectedId(id); setIsEdge(false); }}
            onEdgeSelect={(id) => { setSelectedId(id); setIsEdge(true); }}
            selectedNodeId={selectedId}
          />
          <GraphStatsBar summary={topoData?.summary} />
        </div>

        {/* Right: detail panel */}
        <NodeDetailPanel
          selectedId={selectedId}
          nodeType={selectedNode?.type || selectedEdge?.type || ''}
          nodeProperties={selectedNode?.properties || selectedEdge?.properties || {}}
          isEdge={isEdge}
          allNodes={topoData?.nodes}
          allEdges={topoData?.edges}
          onClose={() => setSelectedId(null)}
        />
      </div>

      {/* Bottom: fault info */}
      {typeInfo && (
        <Card size="small" style={{ borderTop: '1px solid #f0f0f0', borderRadius: 0 }} styles={{ body: { padding: '6px 16px' } }}>
          <Space size="middle">
            <Typography.Text type="secondary">{typeInfo.name}</Typography.Text>
            <Tag>{typeInfo.category}</Tag>
            <Tag color="processing">{typeInfo.target_type}</Tag>
            <Typography.Text type="secondary">阶段: {typeInfo.stages}</Typography.Text>
            <Typography.Text type="secondary">总时长: {Math.floor(typeInfo.duration_s / 60)}min</Typography.Text>
          </Space>
        </Card>
      )}
    </div>
  );
}
