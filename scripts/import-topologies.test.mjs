// Sanity checks for data/layouts/ggg/ and the graph parser: node --test scripts/import-topologies.test.mjs
import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'node:test';
import { importArea, isCampaignArea, missingFiles, parseFileGroups, parseGraph, roomUnitSize } from './import-topologies.mjs';

const DIR = new URL('../data/layouts/ggg/', import.meta.url).pathname;
const names = readdirSync(DIR).filter((n) => n.endsWith('.json') && n !== 'index.json');
const areas = new Map(names.map((n) => [n.replace(/\.json$/, ''), JSON.parse(readFileSync(join(DIR, n), 'utf8'))]));
const index = JSON.parse(readFileSync(join(DIR, 'index.json'), 'utf8'));

test('every file parses, names its own area and has layouts', () => {
  for (const [id, area] of areas) {
    assert.equal(area.areaId, id);
    assert.ok(area.topologies.length > 0, `${id}: no layouts`);
    assert.equal(area.connections.length, area.connectionNames.length, id);
    for (const t of area.topologies) {
      assert.equal(t.format, t.file.endsWith('.tgr') ? 'tgr' : 'dgr', `${id} ${t.id}`);
      assert.ok(t.size.every((n) => Number.isInteger(n) && n > 0), `${id} ${t.id}: size`);
      if (t.format === 'dgr') assert.ok(Number.isInteger(t.roomUnitSize), `${id} ${t.id}: roomUnitSize`);
      else assert.equal(t.roomUnitSize, undefined, `${id} ${t.id}: roomUnitSize on a .tgr`);
    }
  }
});

test('index.json lists exactly the area files', () => {
  assert.deepEqual(index.areas.map((a) => a.id).sort(), [...areas.keys()].sort());
  for (const a of index.areas) assert.equal(a.layouts, areas.get(a.id).topologies.length, a.id);
});

test('every edge joins existing nodes; every coordinate is a number', () => {
  for (const [id, area] of areas) {
    for (const t of area.topologies) {
      for (const n of t.nodes) assert.ok(Number.isFinite(n.x) && Number.isFinite(n.y), `${id} ${t.id}`);
      for (const e of t.edges) {
        assert.ok(e.a >= 0 && e.a < t.nodes.length && e.b >= 0 && e.b < t.nodes.length, `${id} ${t.id}: edge ${e.a}-${e.b}`);
        assert.ok(e.via.flat().every(Number.isFinite), `${id} ${t.id}: via`);
      }
    }
  }
});

test('every exit is one of the area connections, and a campaign exit has its own file', () => {
  for (const [id, area] of areas) {
    for (const t of area.topologies) {
      for (const n of t.nodes.filter((n) => n.exit)) {
        assert.ok(area.connections.includes(n.exit), `${id} ${t.id}: exit ${n.exit} not a connection`);
        // Exits to side areas (expedition, abyss) lead out of the campaign data.
        if (/^(G\d|P\d)/.test(n.exit)) assert.ok(areas.has(n.exit), `${id} ${t.id}: exit ${n.exit} has no file`);
      }
    }
  }
});

test('every Act 1 area has at least one layout', () => {
  const act1 = ['G1_town', ...Array.from({ length: 12 }, (_, k) => `G1_${k + 1}`), 'G1_13_1', 'G1_13_2', 'G1_14', 'G1_15'];
  for (const id of act1) assert.ok(areas.get(id)?.topologies.length > 0, `${id}: no layouts`);
});

// Areas whose layouts have no exit node, and why.
const NO_EXIT = {
  // Its only connection is the way back to the Ramparts; the graphs mark that spot only as the start.
  G1_15: 'only connection is the way back, marked as start',
  // Two one-node town rooms (present and past); the exits are inside the fixed rooms.
  G3_town: 'town: exits inside the fixed rooms',
  // No connections in WorldAreas and no area leads to it; its "entrance1" names nothing.
  G3_15: 'no connections in the table',
};

test('every layout of every area has an exit (or is a named exception)', () => {
  for (const [id, area] of areas) {
    for (const t of area.topologies) {
      if (NO_EXIT[id] && id === 'G3_town') continue;
      if (NO_EXIT[id]) assert.ok(t.nodes.some((n) => n.start), `${id} ${t.id}: no start either`);
      else assert.ok(t.nodes.some((n) => n.exit), `${id} ${t.id}: no exit`);
    }
  }
});

test('no layout keeps a sub-graph placeholder; expanded layouts say so', () => {
  for (const [id, area] of areas) {
    for (const t of area.topologies) {
      assert.ok(!t.nodes.some((n) => n.tag === 'graph' && n.keys.length >= 3), `${id} ${t.id}: placeholder left`);
      if (t.expanded) assert.ok(t.parts.length > 0 && t.parts.every((p) => p.file && p.at.length === 2), `${id} ${t.id}: parts`);
    }
  }
  assert.equal(areas.get('G1_12').topologies.length, 12);
  assert.ok(areas.get('G1_12').topologies.every((t) => t.expanded && t.nodes.some((n) => n.exit === 'G1_11')));
  // Ogham Manor: 16 layouts × 2 basements × 2 middle floors would be 64, over the cap: first files kept.
  assert.equal(areas.get('G1_15').topologies.length, 16);
  assert.ok(areas.get('G1_15').topologies.every((t) => t.alternatives?.length === 2));
});

test("Act 4's islands: the boat at the landing leads back to Kingsmarch", () => {
  for (const id of ['G4_7', 'G4_13', 'G4_14']) {
    for (const t of areas.get(id).topologies) {
      const exits = t.nodes.filter((n) => n.exit);
      assert.ok(exits.length > 0 && exits.every((n) => n.exit === 'G4_town' && n.keys.includes('teleport1')), `${id} ${t.id}`);
    }
  }
});

test('Clearfell and the Riverbank keep their layout counts', () => {
  assert.equal(areas.get('G1_1').topologies.length, 5);
  assert.equal(areas.get('G1_2').topologies.length, 33);
});

test('campaign rule: acts and interludes, no maps, hideouts, world maps or unused areas', () => {
  const row = (Id, extra = {}) => ({ Id, Name: 'x', Topologies: { Ids: ['t'] }, ...extra });
  assert.ok(isCampaignArea(row('G1_2')));
  assert.ok(isCampaignArea(row('P2_Town')));
  assert.ok(!isCampaignArea(row('G1_WorldMap')));
  assert.ok(!isCampaignArea(row('MapCrypt', { IsMapArea: true })));
  assert.ok(!isCampaignArea(row('HideoutCave')));
  assert.ok(!isCampaignArea(row('G4_6', { Name: '[DNT-UNUSED] Vahi Loa' })));
  assert.ok(!isCampaignArea(row('G2_3s', { Topologies: { Ids: [] } })));
});

// The head of metadata/terrain/gallows/act1/1_3/graphs/mudburrows_ne.dgr, cut to three nodes.
const DGR = `﻿version 25
Size: 5 6
MasterFile: "Metadata/Terrain/Gallows/Act1/1_3/master.tsi"
Nodes: 3
Edges: 2
""
""
""
Default%: 0 0 0
0
104 104 2 1 2 "entrance_flip" FI 2 "entrance1" "default" 100 0 0 0 0 0 0 0 0 N 0
104 81 1 0 "" (any) 1 "shuffle1" 100 0 0 0 0 0 0 0 0 N 0
12 12 1 0 "boss" I 5 "teleport1disabled-MudBurrowArena" "teleport2disabled-MudBurrowArena" "telepad1" "telepad2" "zone=2" 100 0 0 0 0 0 0 0 0 N 0
0 1 0 100 0 0 "Metadata/Terrain/Gallows/Act1/1_3/Rooms/cavern_rooms.et" 30 40 1 "" 100 "Metadata/Terrain/Gallows/Act1/1_3/Rooms/cavern_rooms.et" 0 0 P N 1
0 2 0 100 0 0 "Metadata/Terrain/Gallows/Act1/1_3/Rooms/cavern_rooms.et" 0 40 0 "" 0 "" 0 0 P N 1
`;

test('a .dgr graph: header lines skipped, keys kept, entrance words read', () => {
  const g = parseGraph(DGR);
  assert.deepEqual(g.size, [5, 6]);
  assert.equal(g.nodes.length, 3);
  assert.deepEqual(g.nodes[0], { x: 104, y: 104, tag: 'entrance_flip', start: false, exitWords: ['entrance1'], keys: ['entrance1', 'default'], turn: 'FI' });
  assert.equal(g.nodes[2].tag, 'boss');
  assert.deepEqual(g.nodes[2].keys, ['teleport1disabled-MudBurrowArena', 'teleport2disabled-MudBurrowArena', 'telepad1', 'telepad2', 'zone=2']);
  assert.deepEqual(g.edges[0], { a: 0, b: 1, via: [], kind: 'cavern_rooms' });
});

test('a .tgr node with tag "entrance" and key "entrance1" is both the start and an exit', () => {
  const tgr = `version 25\nSize: 10 10\nNodes: 1\nEdges: 0\n""\n""\n""\n265 1070 0 "entrance" (any) 3 "default" "entrance1" "killzone=5" 100 0 0 0 0 0 0 0 0 1 1\n`;
  const [node] = parseGraph(tgr).nodes;
  assert.equal(node.start, true);
  assert.deepEqual(node.exitWords, ['entrance1']);
  assert.deepEqual(node.keys, ['default', 'entrance1', 'killzone=5']);
});

test('RoomUnitSize is read from a master.tsi', () => {
  assert.equal(roomUnitSize('﻿version 3\nStoreyHeight\t16\nRoomUnitSize\t9\nOuterBufferSize 5\n'), 9);
  assert.equal(roomUnitSize('version 3\n'), null);
  assert.equal(roomUnitSize(null), null);
  assert.equal(parseGraph(DGR).master, 'metadata/terrain/gallows/act1/1_3/master.tsi');
});

test('a malformed graph is refused', () => {
  assert.throws(() => parseGraph('version 25\nSize: 1 1\nNodes: 1\nEdges: 0\n12 12 3 0 "x"\n'), /no quoted tag/);
});

// A tiny game: a parent with one placeholder naming a file group of two sub-graphs.
const head = (size, master, nodes, edges) =>
  `version 25\nSize: ${size}\nMasterFile: "${master}"\nNodes: ${nodes.length}\nEdges: ${edges.length}\n""\n""\n""\nDefault%: 0 0 0\n0\n${[...nodes, ...edges].join('\n')}\n`;
const N = (x, y, nb, tag, turn, keys) => `${x} ${y} ${[nb.length, ...nb].join(' ')} "${tag}" ${turn} ${keys.length} ${keys.map((k) => `"${k}"`).join(' ')} 100 0 0 0 0 0 0 0 0 N 0`;
const E = (a, b) => `${a} ${b} 0 100 0 0 "Metadata/x/rooms.et" 0 0 0 "" 0 "" 0 0 P N 1`;
const FILES = {
  'a/master.tsi': 'version 3\nRoomUnitSize\t7\nFileGroups\t"groups.fgp"\n',
  'a/groups.fgp': 'version 1\n"lower"\nGraphs/_lower_1.dgr\nGraphs/_lower_2.dgr\n',
  'a/graphs/top.dgr': head('4 2', 'a/master.tsi', [N(12, 12, [], 'entrance', 'I', ['default']), N(58, 12, [], 'graph', 'I', ['lower', '2', '2'])], []),
  'a/graphs/_lower_1.dgr': head('2 2', 'a/master.tsi', [N(12, 12, [1], 'stairs', 'I', []), N(35, 35, [0], 'exit', 'I', ['entrance1'])], [E(0, 1)]),
  'a/graphs/_lower_2.dgr': head('2 2', 'a/master.tsi', [N(35, 12, [], 'exit', 'I', ['entrance1'])], []),
};
const fake = (have) => ({ text: (p) => (have.includes(p) ? FILES[p] : null) });

test('sub-graphs: missing files are listed round by round, then expanded into the parent', () => {
  const all = Object.keys(FILES);
  assert.deepEqual(missingFiles(['a/graphs/top.dgr'], fake([])), ['a/graphs/top.dgr']);
  assert.deepEqual(missingFiles(['a/graphs/top.dgr'], fake(['a/graphs/top.dgr'])), ['a/master.tsi']);
  assert.deepEqual(missingFiles(['a/graphs/top.dgr'], fake(['a/graphs/top.dgr', 'a/master.tsi'])), ['a/groups.fgp']);
  assert.deepEqual(missingFiles(['a/graphs/top.dgr'], fake(all.slice(0, 3))), ['a/graphs/_lower_1.dgr', 'a/graphs/_lower_2.dgr']);
  assert.deepEqual(missingFiles(['a/graphs/top.dgr'], fake(all)), []);

  const area = { Id: 'X', Name: 'X', Act: 1, AreaLevel: 1, Connections: { Ids: ['Y'] }, Topologies: { Ids: ['Top'] } };
  const out = importArea(area, new Map([['Top', { DGRFile: 'A/Graphs/top.dgr' }]]), new Map(), fake(all));
  // Two files in the group: two layouts, under the cap.
  assert.deepEqual(out.topologies.map((t) => t.id), ['Top#1', 'Top#2']);
  const [one, two] = out.topologies;
  assert.equal(one.expanded, true);
  assert.equal(one.roomUnitSize, 7);
  assert.deepEqual(one.parts, [{ file: 'a/graphs/_lower_1.dgr', at: [46, 0], turn: 'I', scale: 1 }]);
  // The placeholder is gone; the sub-graph's nodes sit from its cell (2, 0) on; edges renumbered.
  assert.deepEqual(one.nodes.map((n) => [n.x, n.y, n.tag, n.exit]), [[12, 12, 'entrance', null], [58, 12, 'stairs', null], [81, 35, 'exit', 'Y']]);
  assert.deepEqual(one.edges.map((e) => [e.a, e.b]), [[1, 2]]);
  assert.deepEqual(two.nodes.map((n) => [n.x, n.y, n.exit]), [[12, 12, null], [81, 12, 'Y']]);
});

test('a file group: names, paths relative to the master folder or absolute', () => {
  const groups = parseFileGroups('﻿version 1\n"Upper"\n"Metadata/T/P3_5/Graphs/_Upper_01.dgr"\n"midlevel_E"\nGraphs/_midlevel_E_1.dgr\n\n', 'metadata/t/1_15');
  assert.deepEqual([...groups], [['Upper', ['metadata/t/p3_5/graphs/_upper_01.dgr']], ['midlevel_E', ['metadata/t/1_15/graphs/_midlevel_e_1.dgr']]]);
});
