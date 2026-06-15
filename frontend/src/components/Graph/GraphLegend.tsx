import { HEALTH_COLORS } from '../../utils/graphStyles';

const shapes: Record<string, string> = {
  'round-rectangle': '▬', hexagon: '⬡', rectangle: '▭', ellipse: '○', diamond: '◇', parallelogram: '▱', triangle: '△', tag: '⊿',
};

export default function GraphLegend() {
  return (
    <div className="graph-legend">
      <span className="legend-label" style={{ fontWeight: 600 }}>健康：</span>
      {Object.entries(HEALTH_COLORS).map(([s, c]) => (
        <div key={s} className="legend-item">
          <span style={{ backgroundColor: c, width: 14, height: 14, borderRadius: 3, display: 'inline-block', border: '1px solid #ccc' }} />
          <span className="legend-label">{s === 'normal' ? '正常' : s === 'warning' ? '警告' : '严重'}</span>
        </div>
      ))}
      <span className="legend-separator">|</span>
      <span className="legend-label" style={{ fontWeight: 600 }}>形状：</span>
      {Object.entries(shapes).map(([s, g]) => (
        <span key={s} style={{ fontSize: 14, margin: '0 2px' }} title={s}>{g}</span>
      ))}
    </div>
  );
}
