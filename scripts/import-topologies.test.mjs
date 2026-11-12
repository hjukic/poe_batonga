// Sanity checks for data/layouts/ggg/ and the graph parser: node --test scripts/import-topologies.test.mjs
import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'node:test';
import { isCampaignArea, parseGraph, roomUnitSize } from './import-topologies.mjs';

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
  assert.deepEqual(g.nodes[0], { x: 104, y: 104, tag: 'entrance_flip', start: false, exitWord: 'entrance1', keys: ['entrance1', 'default'] });
  assert.equal(g.nodes[2].tag, 'boss');
  assert.deepEqual(g.nodes[2].keys, ['teleport1disabled-MudBurrowArena', 'teleport2disabled-MudBurrowArena', 'telepad1', 'telepad2', 'zone=2']);
  assert.deepEqual(g.edges[0], { a: 0, b: 1, via: [], kind: 'cavern_rooms' });
});

test('a .tgr node with tag "entrance" and key "entrance1" is both the start and an exit', () => {
  const tgr = `version 25\nSize: 10 10\nNodes: 1\nEdges: 0\n""\n""\n""\n265 1070 0 "entrance" (any) 3 "default" "entrance1" "killzone=5" 100 0 0 0 0 0 0 0 0 1 1\n`;
  const [node] = parseGraph(tgr).nodes;
  assert.equal(node.start, true);
  assert.equal(node.exitWord, 'entrance1');
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
