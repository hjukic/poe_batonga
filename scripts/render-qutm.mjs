#!/usr/bin/env node
// Draws Qutm's layout diagrams (data/layouts/qutm/layouts.json) for some zones as
// one sheet: a panel per variant with its icons, labels and suggested route.
//
//   node scripts/render-qutm.mjs [areaId ...] [--out file.svg]
//
// Default: the first three zones, to experiments/layouts/output/qutm.svg.
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';

const args = process.argv.slice(2);
const outAt = args.indexOf('--out');
const output = outAt >= 0 ? args.splice(outAt, 2)[1] : 'experiments/layouts/output/qutm.svg';
const ids = args.length ? args : ['G1_1', 'G1_town', 'G1_2'];
const layouts = JSON.parse(readFileSync('data/layouts/qutm/layouts.json', 'utf8'));

const COLORS = { myplayer: '#3fa34d', entrance: '#f2c14e', waypoint: '#3fb6e0', boss: '#e5484d', checkpoint: '#8bd17c' };
const escape = (text) => String(text).replace(/[<>&"]/g, (c) => `&#${c.charCodeAt(0)};`);
const SIZE = 420;
const GAP = 24;
const HEAD = 34;

function panel(areaId, variant, ox, oy) {
  const at = (icon) => [ox + 30 + icon.x * (SIZE - 60), oy + HEAD + 20 + icon.y * (SIZE - 60)];
  const lines = variant.lines
    .filter((l) => l.fromIconId !== l.toIconId)
    .map((l) => {
      const [a, b] = [at(variant.icons[l.fromIconId]), at(variant.icons[l.toIconId])];
      return `<line x1="${a[0]}" y1="${a[1]}" x2="${b[0]}" y2="${b[1]}" stroke="#e8752a" stroke-width="3" stroke-dasharray="8 6" marker-end="url(#arrow)"/>`;
    })
    .join('');
  const icons = variant.icons
    .map((icon) => {
      const [x, y] = at(icon);
      const text = icon.id === 'entrance' ? `→ ${icon.label}` : icon.id === 'myplayer' ? 'start' : icon.label || icon.id;
      return `<circle cx="${x}" cy="${y}" r="9" fill="${COLORS[icon.id] ?? '#bbb'}" stroke="#111" stroke-width="2"/>`
        + `<text x="${x + 13}" y="${y + 5}">${escape(text)}</text>`;
    })
    .join('');
  return `<rect x="${ox}" y="${oy}" width="${SIZE}" height="${SIZE + HEAD}" rx="8" fill="#1b1d22" stroke="#444"/>
<text x="${ox + 12}" y="${oy + 24}" font-size="17" font-weight="bold">${escape(areaId)} · variant "${escape(variant.name)}"</text>
${lines}<g paint-order="stroke" stroke="#1b1d22" stroke-width="4">${icons}</g>`;
}

const rows = ids.map((id) => layouts[id] ?? []);
const cols = Math.max(...rows.map((r) => r.length), 1);
const width = GAP + cols * (SIZE + GAP);
const height = GAP + rows.length * (SIZE + HEAD + GAP);
const panels = rows
  .flatMap((variants, r) => variants.map((v, c) => panel(ids[r], v, GAP + c * (SIZE + GAP), GAP + r * (SIZE + HEAD + GAP))))
  .join('\n');

mkdirSync(dirname(output), { recursive: true });
writeFileSync(output, `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}" font-family="sans-serif" font-size="14" fill="#eee">
<defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto"><path d="M0 0L10 5L0 10z" fill="#e8752a"/></marker></defs>
<rect width="100%" height="100%" fill="#0d0e10"/>
${panels}
</svg>
`);
console.log(output);
