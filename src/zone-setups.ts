// "Our own layout setups": every layout the game can give a zone, laid over each other from
// where you come in (data/tips/act1-setups.json, built by scripts/build-zone-setups.mjs).
// Where many layouts agree, the drawing is bright: that is where a corridor, an exit or the
// boss usually is, however the zone is turned in the game.
import act1 from '../data/tips/act1-setups.json';

interface Point { kind: 'exit' | 'boss' | 'place'; label: string; x: number; y: number }
interface Setup { name: string; layouts: number; paths: [number, number][][]; points: Point[] }

const setups: Record<string, Setup> = (act1 as unknown as { zones: Record<string, Setup> }).zones;
const EXIT_COLORS = ['#ff9a3c', '#5bd1ff', '#c8a2ff', '#ffe066'];
const BOSS = '#ff4d57';
const PLACE = '#7fd18b';

export function hasSetup(zone: string | null | undefined): boolean {
  return !!zone && (setups[zone]?.layouts ?? 0) > 0;
}

/** Draws the zone's setup on `canvas`; returns how many layouts it lays over each other. */
export function drawSetup(canvas: HTMLCanvasElement, zone: string): number {
  const setup = setups[zone];
  const context = canvas.getContext('2d');
  if (!setup || !context) return 0;
  const { width, height } = canvas;
  context.clearRect(0, 0, width, height);
  const xs = [0, ...setup.paths.flat().map(([x]) => x)];
  const ys = [0, ...setup.paths.flat().map(([, y]) => y)];
  const [x0, x1, y0, y1] = [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)];
  const pad = 22;
  const scale = Math.min((width - 2 * pad) / (x1 - x0 || 1), (height - 2 * pad) / (y1 - y0 || 1));
  const left = (width - (x1 - x0) * scale) / 2;
  const at = (x: number, y: number): [number, number] => [left + (x - x0) * scale, pad + (y - y0) * scale];

  // Corridors: faint, so the ones many layouts share stand out.
  context.lineWidth = 3;
  context.strokeStyle = `rgba(170, 160, 230, ${Math.min(0.35, 2 / setup.layouts)})`;
  for (const line of setup.paths) {
    context.beginPath();
    line.forEach(([x, y], i) => (i ? context.lineTo(...at(x, y)) : context.moveTo(...at(x, y))));
    context.stroke();
  }

  // Exits, bosses and places as soft spots: bright where many layouts put them.
  const groups = new Map<string, Point[]>();
  for (const point of setup.points) {
    const key = point.kind === 'boss' && /^Boss/.test(point.label) ? 'Boss' : point.label;
    groups.set(key, [...(groups.get(key) ?? []), point]);
  }
  const exits = [...groups.keys()].filter((key) => groups.get(key)?.[0].kind === 'exit');
  const color = (key: string) => {
    const kind = groups.get(key)?.[0].kind;
    return kind === 'exit' ? EXIT_COLORS[exits.indexOf(key) % EXIT_COLORS.length] : kind === 'boss' ? BOSS : PLACE;
  };
  const alpha = Math.min(0.85, 3.5 / setup.layouts);
  const radius = Math.max(10, Math.min(width, height) / 22);
  for (const [key, points] of groups) {
    const rgb = hexToRgb(color(key));
    for (const { x, y } of points) {
      const [px, py] = at(x, y);
      const glow = context.createRadialGradient(px, py, 0, px, py, radius);
      glow.addColorStop(0, `rgba(${rgb}, ${alpha})`);
      glow.addColorStop(1, `rgba(${rgb}, 0)`);
      context.fillStyle = glow;
      context.fillRect(px - radius, py - radius, 2 * radius, 2 * radius);
    }
  }

  // Each name once, where most layouts put it; moved down when it would cover another.
  context.font = '600 22px system-ui, sans-serif';
  context.textAlign = 'center';
  context.lineJoin = 'round';
  const taken: [number, number, number, number][] = [];
  for (const [key, points] of groups) {
    let best = points[0];
    let most = 0;
    for (const p of points) {
      const near = points.filter((q) => Math.hypot(q.x - p.x, q.y - p.y) < 0.15).length;
      if (near > most) [most, best] = [near, p];
    }
    let [x, y] = at(best.x, best.y);
    y -= 14;
    const w = context.measureText(key).width;
    x = Math.min(Math.max(x, w / 2 + 4), width - w / 2 - 4);
    const overlaps = () => taken.some(([a, b, c, d]) => x - w / 2 < c && x + w / 2 > a && y - 20 < d && y + 4 > b);
    for (let tries = 0; tries < 8 && overlaps(); tries++) y += 24;
    taken.push([x - w / 2, y - 20, x + w / 2, y + 4]);
    context.lineWidth = 5;
    context.strokeStyle = 'rgba(14, 12, 20, 0.9)';
    context.strokeText(key, x, y);
    context.fillStyle = color(key);
    context.fillText(key, x, y);
  }

  // Where you come in.
  const [ex, ey] = at(0, 0);
  context.fillStyle = '#ffffff';
  context.beginPath();
  context.moveTo(ex, ey - 12);
  context.lineTo(ex - 10, ey + 8);
  context.lineTo(ex + 10, ey + 8);
  context.closePath();
  context.fill();
  return setup.layouts;
}

function hexToRgb(hex: string): string {
  const n = parseInt(hex.slice(1), 16);
  return `${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}`;
}
