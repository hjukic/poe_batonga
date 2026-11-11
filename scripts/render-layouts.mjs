#!/usr/bin/env node
// Renders recorded zone layouts (see src-tauri/src/layout.rs) as SVG pictures for review.
//
//   node scripts/render-layouts.mjs [layouts folder] [output folder]
//
// Defaults: the app's layouts folder on Linux, and experiments/layouts/output
// (git-ignored; recordings come from gameplay and stay local).
import { mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { basename, join } from 'node:path';

const input = process.argv[2] ?? join(homedir(), '.local/share/com.poebatonga.overlaytest/layouts');
const output = process.argv[3] ?? 'experiments/layouts/output';
const COLORS = { arrived: '#3fa34d', start: '#3fa34d', left: '#2a8fb0', npc: '#c99a06' };
const escape = (text) => String(text).replace(/[<>&"]/g, (c) => `&#${c.charCodeAt(0)};`);

function render(recording, part, index) {
  const solid = part.walls.filter(([, , seen]) => seen >= 2);
  const xs = [...solid.map((w) => w[0]), ...part.trail.map((t) => t[0]), 0];
  const ys = [...solid.map((w) => w[1]), ...part.trail.map((t) => t[1]), 0];
  const pad = 20;
  const [x0, y0] = [Math.min(...xs) - pad, Math.min(...ys) - pad];
  const [width, height] = [Math.max(...xs) - x0 + pad, Math.max(...ys) - y0 + pad + 24];
  const walls = solid.map(([x, y]) => `M${x - x0} ${y - y0}h1v1h-1z`).join('');
  const trail = part.trail.map(([x, y]) => `${x - x0},${y - y0}`).join(' ');
  const pois = part.pois.map((p) => {
    const [x, y] = [p.x - x0, p.y - y0];
    const label = p.kind === 'npc' ? p.label : `${p.kind}${p.label ? ` ${p.label}` : ''}`;
    return `<circle cx="${x}" cy="${y}" r="4" fill="${COLORS[p.kind] ?? '#000'}"/>`
      + `<text x="${x + 6}" y="${y + 4}">${escape(label)}</text>`;
  }).join('');
  const title = `${recording.area} seed ${recording.seed ?? '?'} part ${index + 1}: `
    + `${part.matched}/${part.frames} frames placed, ${part.lost} lost`;
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width * 2}" height="${height * 2}" font-family="sans-serif" font-size="9">
<rect width="100%" height="100%" fill="#f7f6fa"/>
<path d="${walls}" fill="#4b4778"/>
<polyline points="${trail}" fill="none" stroke="#e8752a" stroke-width="1" stroke-opacity=".6"/>
${pois}
<text x="4" y="${height - 6}" fill="#333">${escape(title)}</text>
</svg>
`;
}

mkdirSync(output, { recursive: true });
let count = 0;
for (const area of readdirSync(input)) {
  const folder = join(input, area);
  if (!statSync(folder).isDirectory()) continue;
  for (const file of readdirSync(folder).filter((f) => f.endsWith('.json'))) {
    const recording = JSON.parse(readFileSync(join(folder, file), 'utf8'));
    recording.parts.forEach((part, index) => {
      const name = `${area}-${basename(file, '.json')}-part${index + 1}.svg`;
      writeFileSync(join(output, name), render(recording, part, index));
      count += 1;
    });
  }
}
console.log(`Rendered ${count} part(s) to ${output}`);
