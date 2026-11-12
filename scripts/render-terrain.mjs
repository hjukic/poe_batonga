#!/usr/bin/env node
// Renders poe2way zone terrain (data/layouts/poe2way) as a readable map: walkable
// floor, walls (outline) and points of interest with labels.
//
//   node scripts/render-terrain.mjs [areaId ...] [--out dir]
//
// Writes <areaId>.svg (north-up as stored) and <areaId>.game.svg (rotated 45°
// to the in-game map orientation) to experiments/layouts/output by default.
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const dataDir = 'data/layouts/poe2way';
const args = process.argv.slice(2);
const outAt = args.indexOf('--out');
const output = outAt >= 0 ? args.splice(outAt, 2)[1] : 'experiments/layouts/output';
const index = JSON.parse(readFileSync(join(dataDir, 'index.json'), 'utf8'));
const ids = args.length ? args : Object.values(index.areas).flatMap((a) => a.variants);

const STYLE = {
  waypoint: { color: '#3fb6e0', r: 9, label: true },
  transition: { color: '#f2c14e', r: 9, label: true },
  boss: { color: '#e5484d', r: 9, label: true },
  checkpoint: { color: '#8bd17c', r: 5, label: false },
  npc: { color: '#c9a7ff', r: 5, label: true },
  stash: { color: '#ffffff', r: 5, label: true },
  portal: { color: '#7aa2ff', r: 6, label: true },
  object: { color: '#bbbbbb', r: 5, label: true },
};
const escape = (text) => String(text).replace(/[<>&"]/g, (c) => `&#${c.charCodeAt(0)};`);

function label(o) {
  if (o.kind === 'transition') return `→ ${o.destination?.name ?? o.destination?.areaId ?? 'exit'}`;
  if (o.kind === 'waypoint') return 'Waypoint';
  return o.label ?? o.kind;
}

// The in-game map shows the world grid turned 45° with y pointing up (fitted on a
// recorded encampment visit; see src-tauri/src/layout/guide.rs `to_screen`).
const C = Math.SQRT1_2;
const toGame = ([x, y]) => [C * (x - y), -C * (x + y)];

function render(id, game) {
  const zone = JSON.parse(readFileSync(join(dataDir, `${id}.json`), 'utf8'));
  const outline = readFileSync(join(dataDir, `${id}.svg`), 'utf8').match(/ d="([^"]+)"/)[1];
  const project = game ? toGame : (p) => p;
  const [bx0, by0, bx1, by1] = zone.terrain.bounds;
  const corners = [[bx0, by0], [bx1, by0], [bx0, by1], [bx1, by1]].map(project);
  // Sizes in map units, scaled so markers and text look the same in every zone.
  const u = Math.max(bx1 - bx0, by1 - by0) / 1100;
  const pad = 60 * u;
  const x0 = Math.min(...corners.map((c) => c[0])) - pad;
  const y0 = Math.min(...corners.map((c) => c[1])) - pad;
  const w = Math.max(...corners.map((c) => c[0])) - x0 + pad;
  const h = Math.max(...corners.map((c) => c[1])) - y0 + pad + 40 * u;
  // SVG matrix(a b c d e f): x' = a x + c y, y' = b x + d y.
  const transform = game ? ` transform="matrix(${C} ${-C} ${-C} ${-C} 0 0)"` : '';

  const marks = zone.objects
    .slice()
    .sort((a, b) => (STYLE[a.kind]?.r ?? 5) - (STYLE[b.kind]?.r ?? 5))
    .map((o) => {
      const s = STYLE[o.kind] ?? STYLE.object;
      const r = s.r * u;
      const [x, y] = project([o.x, o.y]);
      const text = s.label ? `<text x="${x + r + 4 * u}" y="${y + 6 * u}">${escape(label(o))}</text>` : '';
      return `<circle cx="${x}" cy="${y}" r="${r}" fill="${s.color}" stroke="#111" stroke-width="${2 * u}"/>${text}`;
    })
    .join('\n');

  const legend = Object.entries(STYLE)
    .filter(([kind]) => zone.objects.some((o) => o.kind === kind))
    .map(([kind, s], i) => `<circle cx="${x0 + (20 + i * 130) * u}" cy="${y0 + h - 22 * u}" r="${6 * u}" fill="${s.color}"/>`
      + `<text x="${x0 + (32 + i * 130) * u}" y="${y0 + h - 16 * u}">${kind}</text>`)
    .join('');
  const title = `${zone.name} (${zone.areaId})${game ? ', in-game orientation' : ', north-up'}`;

  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${x0} ${y0} ${w} ${h}" width="${Math.round(w / u)}" height="${Math.round(h / u)}" font-family="sans-serif" font-size="${18 * u}" fill="#eee">
<rect x="${x0}" y="${y0}" width="${w}" height="${h}" fill="#0d0e10"/>
<g${transform}><path d="${outline}" fill="#3a3d44" fill-rule="evenodd" stroke="#c8c4b8" stroke-width="${3 * u}" stroke-linejoin="round"/></g>
<g paint-order="stroke" stroke="#0d0e10" stroke-width="${4 * u}">
${marks}
${legend}
<text x="${x0 + 16 * u}" y="${y0 + 30 * u}" font-size="${24 * u}">${escape(title)}</text>
</g>
</svg>
`;
}

mkdirSync(output, { recursive: true });
for (const id of ids) {
  for (const game of [false, true]) {
    const file = join(output, `${id}${game ? '.game' : ''}.svg`);
    writeFileSync(file, render(id, game));
    console.log(file);
  }
}
