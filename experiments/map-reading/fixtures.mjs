import { PNG } from 'pngjs';

// Original synthetic pixels; no game screenshots or assets.
export function scene({ segments = [], dx = 0, dy = 0, players = [[35, 40]], fogX = Infinity, noise = 0 } = {}) {
  const png = new PNG({ width: 128, height: 128 });
  let seed = noise || 1;
  for (let i = 0; i < png.data.length; i += 4) {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
    const value = noise ? seed % 90 : 20;
    png.data.set([value, value + 5, value + 12, 255], i);
  }
  const pixel = (x, y, rgba) => { if (x >= 0 && x < png.width && y >= 0 && y < png.height) png.data.set(rgba, (y * png.width + x) * 4); };
  for (const [x1, y1, x2, y2] of segments) {
    const length = Math.max(Math.abs(x2 - x1), Math.abs(y2 - y1));
    for (let i = 0; i <= length; i++) {
      const x = Math.round(x1 + (x2 - x1) * i / (length || 1));
      const y = Math.round(y1 + (y2 - y1) * i / (length || 1));
      if (x <= fogX) pixel(x + dx, y + dy, [220, 218, 215, 255]);
    }
  }
  for (const [x, y] of players) for (let oy = -1; oy <= 1; oy++) for (let ox = -1; ox <= 1; ox++) pixel(x + ox, y + oy, [40, 240, 90, 255]);
  return png;
}

export const alpha = [[14, 20, 86, 20], [35, 20, 35, 98], [35, 65, 101, 65], [86, 20, 86, 45], [64, 65, 64, 108]];
export const beta = [[14, 20, 86, 20], [35, 20, 35, 98], [35, 65, 101, 65], [86, 20, 86, 45], [82, 65, 104, 98]];
export const cases = [
  { id: 'clear-alpha', image: scene({ segments: alpha }), expected: 'candidate', templateId: 'alpha', player: [35, 40] },
  { id: 'translated-alpha', image: scene({ segments: alpha, dx: 7, dy: -6, players: [[42, 34]] }), expected: 'candidate', templateId: 'alpha', player: [35, 40] },
  { id: 'textured-beta', image: scene({ segments: beta, noise: 812 }), expected: 'candidate', templateId: 'beta', player: [35, 40] },
  { id: 'fog-common-corridors', image: scene({ segments: alpha, fogX: 55 }), expected: 'unknown', reason: 'ambiguous-layout-or-position' },
  { id: 'missing-marker', image: scene({ segments: alpha, players: [] }), expected: 'unknown', reason: 'missing-player-marker' },
  { id: 'multiple-markers', image: scene({ segments: alpha, players: [[35, 40], [85, 30]] }), expected: 'unknown', reason: 'ambiguous-player-marker' },
  { id: 'map-hidden', image: scene({ players: [], noise: 9 }), expected: 'unknown', reason: 'insufficient-map-evidence' },
  { id: 'marker-only', image: scene(), expected: 'unknown', reason: 'insufficient-map-evidence' },
  { id: 'unrelated-geometry', image: scene({ segments: [[20, 100, 100, 28], [20, 100, 100, 100], [100, 28, 100, 100]] }), expected: 'unknown', reason: 'poor-geometry-match' },
  { id: 'unsupported-rotation', image: scene({ segments: alpha.map(([x1, y1, x2, y2]) => [127 - y1, x1, 127 - y2, x2]) }), expected: 'unknown', reason: 'poor-geometry-match' },
  { id: 'unsupported-zoom', image: scene({ segments: alpha.map((segment) => segment.map((v) => Math.round(v * .7))) }), expected: 'unknown', reason: 'poor-geometry-match' },
  { id: 'outside-search-window', image: scene({ segments: alpha, dx: 35, players: [[70, 40]] }), expected: 'unknown', reason: 'poor-geometry-match' },
];
