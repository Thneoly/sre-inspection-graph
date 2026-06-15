# 07 — 前端组件树与实现方案

> React + TypeScript + Cytoscape.js 前端设计。6 个巡检视图共享核心图渲染组件。

## 1. 技术栈

| 技术 | 版本 | 用途 |
|------|------|------|
| React | 18.x | UI 框架 |
| TypeScript | 5.x | 类型安全 |
| Vite | 5.x | 构建工具，HMR 开发服务器 |
| Cytoscape.js | 3.x | 图渲染引擎 |
| cytoscape-dagre | 2.x | 分层布局（DAG 拓扑） |
| TanStack Query | 5.x | 服务端状态管理，缓存，自动刷新 |
| React Router | 6.x | 客户端路由 |
| Axios | 1.x | HTTP 请求 |
| Tailwind CSS | 3.x | 样式 |

## 2. 路由设计

```
/                         → redirect to /topology
/topology                 → TopologyView (View 1)
/access-link              → AccessLinkView (View 2)
/node-impact              → NodeImpactView (View 3)
/config-impact            → ConfigImpactView (View 4)
/image-risk               → ImageRiskView (View 5)
/alert-aggregation        → AlertAggregationView (View 6)
```

## 3. 组件树

```
<App>
  <QueryClientProvider>
    <BrowserRouter>
      <MainLayout>
        ├── <Header />
        │    ├── Logo + Title "SRE 巡检图谱"
        │    └── HealthIndicator (green/yellow/red dot)
        │
        ├── <Sidebar />
        │    ├── NavLink "应用拓扑" → /topology
        │    ├── NavLink "访问链路" → /access-link
        │    ├── NavLink "节点影响" → /node-impact
        │    ├── NavLink "配置影响" → /config-impact
        │    ├── NavLink "镜像风险" → /image-risk
        │    ├── NavLink "告警归并" → /alert-aggregation
        │    └── SystemHealthSummary
        │
        └── <main>  (React Router Outlet)
             └── {View Component}
                  ├── <GraphToolbar />
                  │    ├── SearchBox (node id lookup)
                  │    ├── DepthSlider (1-10)
                  │    ├── SeverityFilter (dropdown)
                  │    └── RefreshButton
                  │
                  ├── <GraphCanvas />          ← CORE: Cytoscape wrapper
                  │    └── cytoscape instance with dagre layout
                  │
                  ├── <GraphLegend />
                  │    └── Color/Shape legend per node type
                  │
                  ├── <GraphStatsBar />
                  │    ├── Total Nodes/Edges count
                  │    └── Risk breakdown (high/medium/low)
                  │
                  └── <NodeDetailPanel />      ← Slide-out right panel
                       ├── BasicInfo section
                       │    ├── Name, Type, ID, Owner Team
                       │    └── Health Status badge
                       ├── MetricsSection (fetched on node select)
                       │    ├── CPU gauge
                       │    ├── Memory gauge
                       │    ├── Restart count
                       │    └── QPS/Error rate
                       └── FindingsSection (fetched on node select)
                            └── List of InspectionFinding cards
```

## 4. 核心组件详设

### GraphCanvas.tsx

最重要的组件，封装 Cytoscape.js。

```typescript
// Props
interface GraphCanvasProps {
  data: GraphResponse | undefined;
  isLoading: boolean;
  onNodeSelect: (nodeId: string) => void;
  onEdgeSelect: (edgeId: string) => void;
  highlightNodeId?: string;  // Node to center on (for impact views)
}

// Internal state
const cyRef = useRef<cytoscape.Core | null>(null);

// Initialization
useEffect(() => {
  cyRef.current = cytoscape({
    container: containerRef.current,
    elements: [], // Start empty, add via json()
    style: getCytoscapeStylesheet(),
    layout: {
      name: 'dagre',
      rankDir: 'TB', // Top to Bottom
      spacingFactor: 1.5,
      nodeDimensionsIncludeLabels: true,
    },
    wheelSensitivity: 0.3,
  });

  // Event handlers
  cyRef.current.on('tap', 'node', (evt) => {
    onNodeSelect(evt.target.id());
  });

  return () => cyRef.current?.destroy();
}, []);

// Update data
useEffect(() => {
  if (data && cyRef.current) {
    cyRef.current.json({ elements: toCytoscapeElements(data) });
    cyRef.current.layout({ name: 'dagre' }).run();
    cyRef.current.fit(undefined, 50);
  }
}, [data]);
```

### NodeDetailPanel.tsx

右侧滑出面板，展示节点详情、指标和巡检发现。

```typescript
interface NodeDetailPanelProps {
  nodeId: string | null;
  onClose: () => void;
}

// Data fetching
const { data: resourceDetail } = useQuery({
  queryKey: ['resource', nodeId],
  queryFn: () => fetchResource(nodeId!),
  enabled: !!nodeId,
});

const { data: metrics } = useQuery({
  queryKey: ['metrics', nodeId],
  queryFn: () => fetchMetrics(nodeId!),
  enabled: !!nodeId,
});

const { data: findings } = useQuery({
  queryKey: ['findings', nodeId],
  queryFn: () => fetchFindings(nodeId!),
  enabled: !!nodeId,
});
```

### View Components (6 个)

每个 View 是薄封装，核心逻辑在父组件/自定义 hooks：

```typescript
// TopologyView.tsx 示例
function TopologyView() {
  const [appCode, setAppCode] = useState('order');
  const [depth, setDepth] = useState(5);
  const [selectedNode, setSelectedNode] = useState<string | null>(null);

  const { data, isLoading } = useGraphData(`/topology/app/${appCode}`, { depth });

  return (
    <>
      <GraphToolbar>
        <AppCodeInput value={appCode} onChange={setAppCode} />
        <DepthSlider value={depth} onChange={setDepth} />
      </GraphToolbar>
      <GraphCanvas data={data} isLoading={isLoading} onNodeSelect={setSelectedNode} />
      <GraphStatsBar summary={data?.summary} />
      <NodeDetailPanel nodeId={selectedNode} onClose={() => setSelectedNode(null)} />
    </>
  );
}
```

## 5. 自定义 Hooks

### useGraphData

```typescript
import { useQuery } from '@tanstack/react-query';

function useGraphData(endpoint: string, params?: Record<string, any>) {
  return useQuery({
    queryKey: [endpoint, params],
    queryFn: () => api.get<GraphResponse>(endpoint, { params }).then(r => r.data),
    staleTime: 30_000,    // 30s cache
    refetchInterval: 60_000, // auto-refresh every 60s
  });
}
```

### useGraphLayout

辅助计算布局配置：

```typescript
function useGraphLayout(viewType: string) {
  return useMemo(() => {
    switch (viewType) {
      case 'topology': return { name: 'dagre', rankDir: 'TB', spacingFactor: 1.5 };
      case 'node-impact': return { name: 'dagre', rankDir: 'LR', spacingFactor: 1.8 };
      default: return { name: 'dagre', rankDir: 'TB', spacingFactor: 1.5 };
    }
  }, [viewType]);
}
```

## 6. Cytoscape 样式映射

`src/utils/graphStyles.ts` — 20+ 节点类型的颜色/形状/大小映射：

```typescript
export const NODE_STYLES: Record<string, NodeStyle> = {
  Environment:        { color: '#e8e8e8', shape: 'round-rectangle', size: 60 },
  Application:        { color: '#4a90d9', shape: 'round-rectangle', size: 55 },
  ApplicationComponent: { color: '#50c878', shape: 'round-rectangle', size: 45 },
  KubernetesCluster:  { color: '#ff8c00', shape: 'hexagon', size: 50 },
  Namespace:          { color: '#ffd700', shape: 'rectangle', size: 40 },
  Deployment:         { color: '#9370db', shape: 'rectangle', size: 40 },
  Service:            { color: '#20b2aa', shape: 'diamond', size: 30 },
  Ingress:            { color: '#ff6347', shape: 'diamond', size: 30 },
  ConfigMap:          { color: '#deb887', shape: 'parallelogram', size: 30 },
  Secret:             { color: '#dc143c', shape: 'parallelogram', size: 30 },
  ContainerRegistry:  { color: '#708090', shape: 'hexagon', size: 40 },
  ContainerImage:     { color: '#4682b4', shape: 'rectangle', size: 35 },
  AlertRule:          { color: '#ff4500', shape: 'triangle', size: 25 },
  Dashboard:          { color: '#2e8b57', shape: 'rectangle', size: 25 },
  // L3 additions
  Pod:                { color: '#6a5acd', shape: 'ellipse', size: 30 },
  Container:          { color: '#87ceeb', shape: 'ellipse', size: 20 },
  KubernetesNode:     { color: '#cd853f', shape: 'hexagon', size: 45 },
  // L4 additions
  AlertEvent:         { color: '#ff0000', shape: 'triangle', size: 25 },
  InspectionFinding:  { color: '#ffa500', shape: 'tag', size: 25 },
  InspectionRun:      { color: '#2f4f4f', shape: 'round-rectangle', size: 30 },
  InspectionRule:     { color: '#8b4513', shape: 'tag', size: 22 },
};
```

**Edge styling**:
- 强依赖: `#333 solid 3px`
- 中依赖: `#888 dashed 2px`
- 弱依赖: `#ccc dotted 1px`

**Risk coloring on nodes**:
- risk_level=high/critical: red border (3px)
- risk_level=medium: yellow border (3px)
- risk_level=low: green border (1px)

## 7. API Client

```typescript
// src/api/client.ts
import axios from 'axios';

const api = axios.create({
  baseURL: import.meta.env.VITE_API_BASE_URL || 'http://localhost:8000/api/v1',
  timeout: 15000,
});

// Response types
export interface GraphNode {
  id: string;
  label: string;
  type: string;
  properties: Record<string, any>;
}

export interface GraphEdge {
  id: string;
  source: string;
  target: string;
  type: string;
  properties: Record<string, any>;
}

export interface GraphResponse {
  nodes: GraphNode[];
  edges: GraphEdge[];
  summary: {
    total_nodes: number;
    total_edges: number;
    risk_counts: Record<string, number>;
    health_counts: Record<string, number>;
  };
}

// API functions
export function fetchTopology(appCode: string, depth = 5) {
  return api.get<GraphResponse>(`/topology/app/${appCode}`, { params: { depth } });
}
export function fetchAccessLink(appCode: string) {
  return api.get<GraphResponse>(`/access-link/${appCode}`);
}
export function fetchNodeImpact(nodeId: string, depth = 4) {
  return api.get<GraphResponse>(`/node-impact/${nodeId}`, { params: { depth } });
}
export function fetchConfigImpact(resourceId: string) {
  return api.get<GraphResponse>(`/config-impact/${resourceId}`);
}
export function fetchImageRisk(imageId: string) {
  return api.get<GraphResponse>(`/image-risk/${imageId}`);
}
export function fetchAlertAggregation(params?: Record<string, any>) {
  return api.get<GraphResponse>('/alert-aggregation', { params });
}
```

## 8. 状态管理

| 状态 | 管理方式 | 理由 |
|------|----------|------|
| 图数据 | TanStack Query (server state) | 自动缓存、后台刷新、loading/error 状态 |
| 选中节点 | useState (local) | 仅影响 NodeDetailPanel 显示 |
| 视图筛选 | useSearchParams (URL) | 可分享 URL、浏览器前进后退 |
| 布局配置 | useMemo | 纯计算，无副作用 |
| Cytoscape 实例 | useRef | mutable ref，避免重渲染 |

## 9. Vite 开发代理

```typescript
// vite.config.ts
export default defineConfig({
  plugins: [react()],
  server: {
    port: 3000,
    proxy: {
      '/api': {
        target: 'http://localhost:8000',
        changeOrigin: true,
      },
    },
  },
});
```
