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

function marker({ at: [x, y], kind }: Mark): string {
  const c = COLOR[kind];
  switch (kind) {
    case 'in': return `<path d="M${x} ${y - 3.2} L${x - 2.8} ${y + 1.8} L${x + 2.8} ${y + 1.8} Z" fill="${c}"/>`;
    case 'exit': return `<rect x="${x - 2}" y="${y - 2}" width="4" height="4" rx="0.8" fill="${c}"/>`;
    case 'maybe': return `<rect x="${x - 2}" y="${y - 2}" width="4" height="4" rx="0.8" fill="none" stroke="${c}" stroke-width="0.7"/>`;
    case 'boss': return `<circle cx="${x}" cy="${y}" r="2.3" fill="${c}"/><path d="M${x - 1} ${y - 0.4} h0.01 M${x + 1} ${y - 0.4} h0.01" stroke="#1a1720" stroke-width="0.9" stroke-linecap="round"/>`;
    case 'wp': return `<path d="M${x} ${y - 2.6} L${x + 2.2} ${y} L${x} ${y + 2.6} L${x - 2.2} ${y} Z" fill="${c}"/>`;
    case 'reward': return `<path d="M${x} ${y - 2.6} L${x + 0.8} ${y - 0.8} L${x + 2.6} ${y - 0.6} L${x + 1.2} ${y + 0.6} L${x + 1.6} ${y + 2.5} L${x} ${y + 1.5} L${x - 1.6} ${y + 2.5} L${x - 1.2} ${y + 0.6} L${x - 2.6} ${y - 0.6} L${x - 0.8} ${y - 0.8} Z" fill="${c}"/>`;
    default: return `<circle cx="${x}" cy="${y}" r="1.9" fill="${c}"/>`;
  }
}

function label(mark: Mark): string {
  const [x, y] = mark.at;
  const anchor = mark.anchor ?? (x < 22 ? 'start' : x > 78 ? 'end' : 'middle');
  const dx = anchor === 'start' ? -2 : anchor === 'end' ? 2 : 0;
  const ty = mark.below ? y + 6.4 : y - 3.6;
  return `<text x="${x + dx}" y="${ty}" text-anchor="${anchor}" fill="${COLOR[mark.kind]}" class="g-label">${esc(mark.label)}</text>`;
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
  parts.push(`<defs><marker id="g-arrow" viewBox="0 0 6 6" refX="4" refY="3" markerWidth="4" markerHeight="4" orient="auto-start-reverse"><path d="M0 0 L6 3 L0 6 Z" fill="#ffd27a"/></marker>`
    + `<marker id="g-arrow-alt" viewBox="0 0 6 6" refX="4" refY="3" markerWidth="4" markerHeight="4" orient="auto-start-reverse"><path d="M0 0 L6 3 L0 6 Z" fill="#a49cc0"/></marker></defs>`);
  for (const outline of drawing.shape ?? []) {
    parts.push(`<polyline points="${pts(outline)}" fill="none" stroke="rgba(170,160,230,0.45)" stroke-width="5" stroke-linejoin="round" stroke-linecap="round"/>`);
  }
  for (const clue of drawing.clues ?? []) {
    parts.push(`<path d="${wavy(clue.points)}" fill="none" stroke="#5bd1ff" stroke-opacity="0.75" stroke-width="0.7"/>`);
    if (clue.label) {
      const [x, y] = clue.at ?? clue.points[Math.floor(clue.points.length / 2)];
      parts.push(`<text x="${x}" y="${y}" text-anchor="middle" fill="#8edce8" class="g-clue">${esc(clue.label)}</text>`);
    }
  }
  for (const line of drawing.alt ?? []) {
    parts.push(`<polyline points="${pts(line.points)}" fill="none" stroke="#a49cc0" stroke-width="0.8" stroke-dasharray="2 1.6" marker-end="url(#g-arrow-alt)"/>`);
  }
  for (const line of drawing.route ?? []) {
    parts.push(`<polyline points="${pts(line.points)}" fill="none" stroke="#ffd27a" stroke-width="1.1" stroke-linejoin="round" marker-end="url(#g-arrow)"/>`);
  }
  for (const mark of drawing.marks) parts.push(marker(mark));
  for (const mark of drawing.marks) parts.push(label(mark));
  parts.push('</svg>');
  return parts.join('');
}
