// Where exits and tagged places lie relative to the entrance, over every layout of the
// Act 1 zones: the evidence of docs/act1-zone-tips.md. Usage: node scripts/zone-layout-stats.mjs data/layouts/ggg
import { readFileSync } from 'node:fs';
const dir = process.argv[2];
const C = Math.SQRT1_2, FLAT = 0.54;
const screen = (x, y) => [C * (x - y), -C * (x + y) * FLAT];
const from = { G1_1: null, G1_2: 'G1_town', G1_3: 'G1_2', G1_4: 'G1_2', G1_5: 'G1_4', G1_6: 'G1_4', G1_7: 'G1_6', G1_8: 'G1_7', G1_9: 'G1_7',
  G1_11: 'G1_7', G1_12: 'G1_11', G1_13_1: 'G1_11', G1_13_2: 'G1_13_1', G1_14: 'G1_13_2', G1_15: 'G1_14' };
const PLACE = /^(boss|bossman|arena|campsite|witch_hut|blacksmith|gallows|hamlet|quest|crowbell_v\d|tomb_mid|mausoleum_centre|hub|highhut|boss_entry|StairsDown1|StairsUp2|encounter(_mid)?|lumberjack|post)$/;
const oct = (dx, dy) => ['E', 'SE', 'S', 'SW', 'W', 'NW', 'N', 'NE'][((Math.round(Math.atan2(dy, dx) / (Math.PI / 4)) % 8) + 8) % 8];
for (const area of Object.keys(from)) {
  let data; try { data = JSON.parse(readFileSync(`${dir}/${area}.json`)); } catch { continue; }
  const names = Object.fromEntries(data.connections.map((c, i) => [c, data.connectionNames[i]]));
  const S = {}; let n = 0;
  for (const t of data.topologies) {
    const unit = t.format === 'dgr' ? Math.max((t.roomUnitSize ?? 9) - 1, 1) : 1;
    const pts = t.nodes.map((q) => screen(q.x * unit, q.y * unit));
    let ei = from[area] ? t.nodes.findIndex((q) => q.exit === from[area] && (q.keys || []).includes('default')) : -1;
    if (ei < 0 && from[area]) ei = t.nodes.findIndex((q) => q.exit === from[area]);
    if (ei < 0) ei = t.nodes.findIndex((q) => q.start || q.tag === 'entrance' || (q.keys || []).includes('entrance'));
    if (ei < 0) continue; n++;
    const e = pts[ei];
    const span = Math.max(...pts.map((p) => Math.hypot(p[0] - e[0], p[1] - e[1])));
    t.nodes.forEach((q, i) => {
      if (i === ei) return;
      const what = q.exit ? (q.exit === from[area] ? null : `exit ${names[q.exit] ?? q.exit}`) : PLACE.test(q.tag || '') ? q.tag.replace(/_v\d+$/, '').replace('bossman', 'boss') : null;
      if (!what) return;
      const p = pts[i], d = Math.hypot(p[0] - e[0], p[1] - e[1]) / span;
      const s = (S[what] ??= { n: 0, far: 0, oct: {}, d: [] });
      s.n++; s.d.push(d); if (d > 0.6) s.far++;
      const o = oct(p[0] - e[0], p[1] - e[1]); s.oct[o] = (s.oct[o] ?? 0) + 1;
    });
  }
  console.log(`\n## ${area} ${data.name} (${n}/${data.topologies.length} layouts, ${data.topologies[0].format}) from ${names[from[area]] ?? 'start'}`);
  for (const [w, s] of Object.entries(S)) {
    const o = Object.entries(s.oct).sort((a, b) => b[1] - a[1]).map(([k, v]) => `${k}${v}`).join(' ');
    const dm = s.d.sort((a, b) => a - b)[s.d.length >> 1];
    console.log(`  ${w.padEnd(32)} n${String(s.n).padStart(3)} far(>0.6) ${String(Math.round(100 * s.far / s.n)).padStart(3)}%  median dist ${dm.toFixed(2)}  dir from entrance: ${o}`);
  }
}
