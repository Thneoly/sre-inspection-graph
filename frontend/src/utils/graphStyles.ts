// Shape = resource type.  Color = health status (green / yellow / red).
// No per-type colors.  No icons.  Clean and instantly readable.

export interface NodeStyle {
  shape: string;
  size: number;
}

export const NODE_STYLES: Record<string, NodeStyle> = {
  // ── Business — round-rectangle ──
  Environment:        { shape: 'round-rectangle', size: 60 },
  Application:        { shape: 'round-rectangle', size: 55 },
  ApplicationComponent: { shape: 'round-rectangle', size: 48 },

  // ── Compute / K8s — hexagon ──
  KubernetesCluster:  { shape: 'hexagon', size: 52 },
  KubernetesNode:     { shape: 'hexagon', size: 48 },
  ContainerRegistry:  { shape: 'hexagon', size: 44 },

  // ── Workload — rectangle ──
  Namespace:          { shape: 'rectangle', size: 44 },
  Deployment:         { shape: 'rectangle', size: 44 },
  ContainerImage:     { shape: 'rectangle', size: 40 },
  Dashboard:          { shape: 'rectangle', size: 34 },

  // ── Pod / Container — ellipse ──
  Pod:                { shape: 'ellipse', size: 40 },
  Container:          { shape: 'ellipse', size: 34 },

  // ── Network — diamond ──
  Service:            { shape: 'diamond', size: 42 },
  Ingress:            { shape: 'diamond', size: 42 },
  ELB:                { shape: 'diamond', size: 42 },
  Gateway:            { shape: 'diamond', size: 42 },
  APIG:               { shape: 'diamond', size: 44 },

  // ── Config — parallelogram ──
  ConfigMap:          { shape: 'parallelogram', size: 38 },
  Secret:             { shape: 'parallelogram', size: 38 },

  // ── Alerts — triangle ──
  AlertRule:          { shape: 'triangle', size: 36 },
  AlertEvent:         { shape: 'triangle', size: 36 },

  // ── Inspection — tag ──
  InspectionFinding:  { shape: 'tag', size: 36 },
  InspectionRun:      { shape: 'round-rectangle', size: 38 },
  InspectionRule:     { shape: 'tag', size: 32 },

  // ── Middleware — rectangle ──
  MySQL:              { shape: 'rectangle', size: 40 },
  Redis:              { shape: 'rectangle', size: 40 },
  Kafka:              { shape: 'rectangle', size: 40 },
  Nacos:              { shape: 'diamond', size: 42 },

  // ── Infrastructure — hexagon ──
  Region:             { shape: 'hexagon', size: 64 },
  AZ:                 { shape: 'hexagon', size: 56 },
};

// Health → fill color.  The ONLY colors on nodes.
export const HEALTH_COLORS: Record<string, string> = {
  normal:   '#81C784',
  warning:  '#FFB300',
  critical: '#E57373',
};

// Health → text color (dark, readable on colored fills)
export const HEALTH_TEXT_COLORS: Record<string, string> = {
  normal:   '#1B5E20',
  warning:  '#3E2723',
  critical: '#B71C1C',
};

// Risk → border color
export const RISK_BORDER_COLORS: Record<string, string> = {
  low:      '#A5D6A7',
  medium:   '#FFCC80',
  high:     '#EF9A9A',
  critical: '#E57373',
};

export function getNodeStyle(nodeType: string): NodeStyle {
  return NODE_STYLES[nodeType] || { shape: 'ellipse', size: 40 };
}

export function getHealthFillColor(healthStatus: string): string {
  return HEALTH_COLORS[healthStatus] || HEALTH_COLORS.normal;
}

export function getHealthTextColor(healthStatus: string): string {
  return HEALTH_TEXT_COLORS[healthStatus] || HEALTH_TEXT_COLORS.normal;
}

export function getRiskBorder(riskLevel: string): string {
  return RISK_BORDER_COLORS[riskLevel] || '#BDBDBD';
}

export function getEdgeColor(_strength: string): string {
  return '#78909C';
}

export function getEdgeWidth(strength: string): number {
  switch (strength) {
    case '强': return 2;
    case '中': return 1.2;
    default:   return 0.8;
  }
}

export function getEdgeLineStyle(strength: string): string {
  return strength === '强' ? 'solid' : 'dashed';
}
