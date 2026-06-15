import { Tag, Space, Typography } from 'antd';
import { LAYERS, type LayerName } from '../../utils/layers';

interface LayerToggleProps {
  activeLayers: Set<LayerName>;
  onChange: (layers: Set<LayerName>) => void;
}

export default function LayerToggle({ activeLayers, onChange }: LayerToggleProps) {
  const toggle = (name: LayerName) => {
    const next = new Set(activeLayers);
    if (next.has(name)) {
      next.delete(name);
    } else {
      next.add(name);
    }
    onChange(next);
  };

  return (
    <Space size="small">
      <Typography.Text style={{ fontSize: 12, color: '#999' }}>图层</Typography.Text>
      {(Object.entries(LAYERS) as [LayerName, typeof LAYERS[keyof typeof LAYERS]][]).map(([name, cfg]) => {
        const active = activeLayers.has(name);
        return (
          <Tag
            key={name}
            color={active ? cfg.color : undefined}
            style={{
              cursor: 'pointer',
              opacity: active ? 1 : 0.4,
              border: active ? `1px solid ${cfg.color}` : '1px dashed #ccc',
              background: active ? undefined : 'transparent',
              color: active ? undefined : '#999',
            }}
            onClick={() => toggle(name)}
          >
            {cfg.label}
          </Tag>
        );
      })}
    </Space>
  );
}
