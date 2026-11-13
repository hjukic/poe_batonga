// Builds data/tips/act1-setups.json: for each Act 1 zone, every layout the game can give it,
// laid over each other from the entrance. Each layout is moved so the entrance is at (0, 0),
// turned so the way into the zone (entrance to the middle of the layout) points up, and scaled
// so its farthest node is 1 away. Where the drawings of many layouts agree (a corridor, an
// exit, a boss) is where it usually is, however the zone is turned in the game.
//
//   node scripts/build-zone-setups.mjs [data/layouts/ggg] [data/tips/act1-setups.json]
import { readFileSync, writeFileSync } from 'node:fs';

const [dir = 'data/layouts/ggg', out = 'data/tips/act1-setups.json'] = process.argv.slice(2);

/** The zone each one is first entered from, in the campaign (null: where the act starts). */
export const ENTERED_FROM = {
  G1_1: null, G1_2: 'G1_town', G1_3: 'G1_2', G1_4: 'G1_2', G1_5: 'G1_4', G1_6: 'G1_4', G1_7: 'G1_6',
  G1_8: 'G1_7', G1_9: 'G1_7', G1_11: 'G1_7', G1_12: 'G1_11', G1_13_1: 'G1_11', G1_13_2: 'G1_13_1',
  G1_14: 'G1_13_2', G1_15: 'G1_14',
};

/** Tagged places worth drawing, by the layout files' tags, and what to call them. */
export const PLACES = {
  boss: 'Boss', bossman: 'Boss', arena: 'Boss arena', campsite: 'Campsite', witch_hut: "Areagne's hut",
  blacksmith: 'Smithing house', gallows: 'Gallows', quest: "Una's lute", crowbell_v1: 'The Crowbell',
  crowbell_v3: 'The Crowbell', encounter: 'Rust obelisk', encounter_mid: 'Rust obelisk', ritual_1: 'Ritual',
  ritual_2: 'Ritual', ritual_3: 'Ritual', tomb_mid: 'Centre tile', mausoleum_centre: 'Wheel tile',
  boss_entry: 'Boss stairs', StairsDown1: 'Stairs down', StairsUp2: 'Stairs up',
};

const C = Math.SQRT1_2;
const FLAT = 0.54; // the map's up-down axis, as the radar measured it (src-tauri layout/guide.rs)
const screen = (x, y) => [C * (x - y), -C * (x + y) * FLAT];
const round = (v) => Math.round(v * 1000) / 1000;

export function setup(area, data) {
  const from = ENTERED_FROM[area];
  const names = Object.fromEntries(data.connections.map((id, i) => [id, data.connectionNames[i]]));
  const paths = new Map();
  const points = [];
  let layouts = 0;
  for (const t of data.topologies) {
    const unit = t.format === 'dgr' ? Math.max((t.roomUnitSize ?? 9) - 1, 1) : 1;
    const at = t.nodes.map((n) => screen(n.x * unit, n.y * unit));
    const all = t.nodes.map((n, i) => [n, i]);
    // By the exit come in by; else where the zone starts (the Riverbank; Ogham Manor's floors).
    let entries = from ? all.filter(([n]) => n.exit === from) : [];
    if (!entries.length) entries = all.filter(([n]) => n.start || n.tag === 'entrance' || (n.keys || []).includes('entrance'));
    if (!entries.length) continue;
    const [, e] = entries.find(([n]) => (n.keys || []).includes('default')) ?? entries[0];
    const [ex, ey] = at[e];
    const [cx, cy] = at.reduce(([sx, sy], [x, y]) => [sx + x / at.length, sy + y / at.length], [0, 0]);
    const span = Math.max(...at.map(([x, y]) => Math.hypot(x - ex, y - ey))) || 1;
    // Turn the way in (entrance to the middle) to point up (screen y down, so -y).
    const turn = -Math.PI / 2 - Math.atan2(cy - ey, cx - ex);
    const [cos, sin] = [Math.cos(turn), Math.sin(turn)];
    const place = ([x, y]) => {
      const [dx, dy] = [(x - ex) / span, (y - ey) / span];
      return [round(dx * cos - dy * sin), round(dx * sin + dy * cos)];
    };
    layouts++;
    for (const edge of t.edges) {
      const line = [t.nodes[edge.a], ...(edge.via || []).map(([x, y]) => ({ x, y })), t.nodes[edge.b]]
        .map((n) => place(screen(n.x * unit, n.y * unit)));
      const key = JSON.stringify(line);
      paths.set(key, line);
    }
    t.nodes.forEach((n, i) => {
      if (i === e) return;
      const label = n.exit ? (n.exit === from ? null : names[n.exit] ?? n.exit) : PLACES[n.tag] ?? null;
      if (!label) return;
      const kind = n.exit ? 'exit' : /boss|arena|crowbell/i.test(n.tag) ? 'boss' : 'place';
      const [x, y] = place(at[i]);
      points.push({ kind, label, x, y });
    });
  }
  return { name: data.name, layouts, paths: [...paths.values()], points };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const zones = {};
  for (const area of Object.keys(ENTERED_FROM)) {
    const data = JSON.parse(readFileSync(`${dir}/${area}.json`, 'utf8'));
    zones[area] = setup(area, data);
  }
  const file = {
    act: 1,
    about: 'Every layout of each Act 1 zone laid over each other from the entrance (0, 0), the way in pointing up, each scaled to its farthest node (scripts/build-zone-setups.mjs).',
    zones,
  };
  writeFileSync(out, JSON.stringify(file) + '\n');
  for (const [area, z] of Object.entries(zones)) console.log(area.padEnd(8), z.name.padEnd(26), 'layouts', z.layouts, 'paths', z.paths.length, 'points', z.points.length);
}
