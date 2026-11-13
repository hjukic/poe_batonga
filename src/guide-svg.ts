// A zone guide's schematic as SVG markup: the zone's usual shape, the route to follow, the
// places on it and the clues that lead there (data/guides/act1.json). Drawn from where you
// come in, at the bottom; the game turns zones, the pattern stays. Pure: the panel inserts the
// markup, and scripts can render it to check a drawing.

export type Kind = 'in' | 'exit' | 'boss' | 'wp' | 'reward' | 'quest' | 'place' | 'maybe';
export interface Mark { at: [number, number]; kind: Kind; label: string; below?: boolean; anchor?: 'start' | 'middle' | 'end' }
export interface Line { points: [number, number][]; label?: string; at?: [number, number] }
export interface Drawing {
  /** The zone's rough shape: outlines, faint. */
  shape?: [number, number][][];
  /** The way to go: solid arrows, in order. */
  route?: Line[];
  /** Other ways (if the first is not there, detours): dashed. */
  alt?: Line[];
  /** Clues in the zone that lead somewhere (a river, signposts): wavy, with a label. */
  clues?: Line[];
  marks: Mark[];
}

const W = 100;
const H = 70;
const COLOR: Record<Kind, string> = {
  in: '#ffffff', exit: '#ff9a3c', boss: '#ff5a62', wp: '#5bd1ff', reward: '#ffd84d', quest: '#7fd18b',
  place: '#c8a2ff', maybe: '#ff9a3c',
};

const pts = (points: [number, number][]) => points.map(([x, y]) => `${x},${y}`).join(' ');
const esc = (text: string) => text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

/** A marker's shape at (x, y), as a path or element without paint. */
function shape([x, y]: [number, number], kind: Kind): string {
  switch (kind) {
    case 'in': return `<path d="M${x} ${y - 3.2} L${x - 2.8} ${y + 1.8} L${x + 2.8} ${y + 1.8} Z" stroke-linejoin="round"/>`;
    case 'exit':
    case 'maybe': return `<rect x="${x - 2.1}" y="${y - 2.1}" width="4.2" height="4.2" rx="1"/>`;
    case 'wp': return `<path d="M${x} ${y - 2.7} L${x + 2.3} ${y} L${x} ${y + 2.7} L${x - 2.3} ${y} Z" stroke-linejoin="round"/>`;
    case 'reward': return `<path d="M${x} ${y - 2.7} L${x + 0.8} ${y - 0.8} L${x + 2.7} ${y - 0.6} L${x + 1.2} ${y + 0.6} L${x + 1.6} ${y + 2.6} L${x} ${y + 1.5} L${x - 1.6} ${y + 2.6} L${x - 1.2} ${y + 0.6} L${x - 2.7} ${y - 0.6} L${x - 0.8} ${y - 0.8} Z" stroke-linejoin="round"/>`;
    case 'boss': return `<circle cx="${x}" cy="${y}" r="2.4"/>`;
    default: return `<circle cx="${x}" cy="${y}" r="2"/>`;
  }
}

/** A glass marker, like the panel: the colour as a tinted glass body with a bright rim and a
 *  highlight at its top left; a "maybe" is the rim alone. */
function marker({ at, kind }: Mark): string {
  const c = COLOR[kind];
  const [x, y] = at;
  const body = kind === 'maybe' ? 'none' : `${c}`;
  const parts = [
    `<g fill="${body}" fill-opacity="${kind === 'in' ? 0.55 : 0.32}" stroke="${c}" stroke-opacity="0.9" stroke-width="0.45">${shape(at, kind)}</g>`,
  ];
  if (kind !== 'maybe') {
    parts.push(`<ellipse cx="${x - 0.7}" cy="${y - 0.9}" rx="1.1" ry="0.55" fill="#ffffff" fill-opacity="0.45"/>`);
  }
  if (kind === 'boss') {
    parts.push(`<path d="M${x - 0.9} ${y + 0.2} h0.01 M${x + 0.9} ${y + 0.2} h0.01" stroke="${c}" stroke-width="0.8" stroke-linecap="round"/>`);
  }
  return parts.join('');
}

function label(mark: Mark): string {
  const [x, y] = mark.at;
  const anchor = mark.anchor ?? (x < 22 ? 'start' : x > 78 ? 'end' : 'middle');
  const dx = anchor === 'start' ? -2 : anchor === 'end' ? 2 : 0;
  const ty = mark.below ? y + 6.4 : y - 3.6;
  return `<text x="${x + dx}" y="${ty}" text-anchor="${anchor}" fill="${COLOR[mark.kind]}" fill-opacity="0.92" class="g-label">${esc(mark.label)}</text>`;
}

/** A wavy line along `points` (a clue: a river, roots, tracks). */
function wavy(points: [number, number][]): string {
  let d = '';
  for (let i = 1; i < points.length; i++) {
    const [x0, y0] = points[i - 1];
    const [x1, y1] = points[i];
    const len = Math.hypot(x1 - x0, y1 - y0);
    const steps = Math.max(2, Math.round(len / 3));
    const [nx, ny] = [-(y1 - y0) / len, (x1 - x0) / len];
    if (i === 1) d += `M${x0} ${y0}`;
    for (let k = 1; k <= steps; k++) {
      const t = k / steps;
      const w = (k % 2 ? 1 : -1) * 0.9;
      d += ` Q${x0 + (x1 - x0) * (t - 0.5 / steps) + nx * w} ${y0 + (y1 - y0) * (t - 0.5 / steps) + ny * w} ${x0 + (x1 - x0) * t} ${y0 + (y1 - y0) * t}`;
    }
  }
  return d;
}

export function guideSvg(drawing: Drawing): string {
  const parts: string[] = [];
  parts.push(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${W} ${H}" class="guide-svg">`);
  parts.push('<defs>'
    + '<marker id="g-arrow" viewBox="0 0 6 6" refX="4" refY="3" markerWidth="4" markerHeight="4" orient="auto-start-reverse"><path d="M0 0 L6 3 L0 6 Z" fill="#ffd27a" fill-opacity="0.9"/></marker>'
    + '<marker id="g-arrow-alt" viewBox="0 0 6 6" refX="4" refY="3" markerWidth="4" markerHeight="4" orient="auto-start-reverse"><path d="M0 0 L6 3 L0 6 Z" fill="#efebff" fill-opacity="0.45"/></marker>'
    + '<filter id="g-glow" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="0.9"/></filter>'
    + '<linearGradient id="g-sheen" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#ffffff" stop-opacity="0.07"/><stop offset="1" stop-color="#ffffff" stop-opacity="0"/></linearGradient>'
    + '</defs>');
  // A frosted card behind the drawing, as the panel's own glass.
  parts.push(`<rect x="0.6" y="0.6" width="${W - 1.2}" height="${H - 1.2}" rx="3.5" fill="#efebff" fill-opacity="0.025" stroke="#efebff" stroke-opacity="0.07" stroke-width="0.35"/>`);
  parts.push(`<rect x="0.6" y="0.6" width="${W - 1.2}" height="${(H - 1.2) / 2}" rx="3.5" fill="url(#g-sheen)"/>`);
  // The zone's shape as glass tubes: a faint bright rim, a dark translucent body, a sheen.
  for (const outline of drawing.shape ?? []) {
    const line = pts(outline);
    parts.push(`<polyline points="${line}" fill="none" stroke="#efebff" stroke-opacity="0.16" stroke-width="6.2" stroke-linejoin="round" stroke-linecap="round"/>`);
    parts.push(`<polyline points="${line}" fill="none" stroke="#14121c" stroke-opacity="0.55" stroke-width="5.4" stroke-linejoin="round" stroke-linecap="round"/>`);
    parts.push(`<polyline points="${line}" fill="none" stroke="#aaa0e6" stroke-opacity="0.12" stroke-width="5.4" stroke-linejoin="round" stroke-linecap="round"/>`);
  }
  for (const clue of drawing.clues ?? []) {
    parts.push(`<path d="${wavy(clue.points)}" fill="none" stroke="#8edce8" stroke-opacity="0.55" stroke-width="0.6"/>`);
    if (clue.label) {
      const [x, y] = clue.at ?? clue.points[Math.floor(clue.points.length / 2)];
      parts.push(`<text x="${x}" y="${y}" text-anchor="middle" fill="#8edce8" fill-opacity="0.8" class="g-clue">${esc(clue.label)}</text>`);
    }
  }
  for (const line of drawing.alt ?? []) {
    parts.push(`<polyline points="${pts(line.points)}" fill="none" stroke="#efebff" stroke-opacity="0.4" stroke-width="0.6" stroke-dasharray="1.6 1.4" stroke-linecap="round" marker-end="url(#g-arrow-alt)"/>`);
  }
  // The route: a soft amber glow under a thin bright line.
  for (const line of drawing.route ?? []) {
    parts.push(`<polyline points="${pts(line.points)}" fill="none" stroke="#ffb84d" stroke-opacity="0.45" stroke-width="2.2" stroke-linejoin="round" stroke-linecap="round" filter="url(#g-glow)"/>`);
    parts.push(`<polyline points="${pts(line.points)}" fill="none" stroke="#ffd27a" stroke-opacity="0.95" stroke-width="0.8" stroke-linejoin="round" stroke-linecap="round" marker-end="url(#g-arrow)"/>`);
  }
  for (const mark of drawing.marks) parts.push(marker(mark));
  for (const mark of drawing.marks) parts.push(label(mark));
  parts.push('</svg>');
  return parts.join('');
}
