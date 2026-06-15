import { describe, it, expect } from 'vitest';
import {
  NODE_STYLES, getNodeStyle, getHealthFillColor, getHealthTextColor,
  getRiskBorder, getEdgeColor, getEdgeWidth, getEdgeLineStyle,
} from '../utils/graphStyles';

describe('NODE_STYLES', () => {
  it('30 types defined', () => expect(Object.keys(NODE_STYLES)).toHaveLength(30));
  it('each has shape and size', () => {
    for (const [t, s] of Object.entries(NODE_STYLES)) {
      expect(s.shape, t).toBeTruthy();
      expect(s.size, t).toBeGreaterThan(0);
    }
  });
  it('shapes use meaningful categories', () => {
    expect(getNodeStyle('Pod').shape).toBe('ellipse');
    expect(getNodeStyle('Secret').shape).toBe('parallelogram');
    expect(getNodeStyle('Service').shape).toBe('diamond');
    expect(getNodeStyle('KubernetesCluster').shape).toBe('hexagon');
    expect(getNodeStyle('Deployment').shape).toBe('rectangle');
  });
});

describe('health colors', () => {
  it('normal=green', () => expect(getHealthFillColor('normal')).toBe('#81C784'));
  it('warning=amber', () => expect(getHealthFillColor('warning')).toBe('#FFB300'));
  it('critical=red', () => expect(getHealthFillColor('critical')).toBe('#E57373'));
});

describe('edges', () => {
  it('single color', () => expect(getEdgeColor('强')).toBe('#78909C'));
  it('widths', () => { expect(getEdgeWidth('强')).toBe(2); expect(getEdgeWidth('弱')).toBe(0.8); });
  it('styles', () => { expect(getEdgeLineStyle('强')).toBe('solid'); expect(getEdgeLineStyle('弱')).toBe('dashed'); });
});
