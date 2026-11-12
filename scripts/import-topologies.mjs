#!/usr/bin/env node
// Imports the game's zone layout graphs (.tgr "topologies") into data/layouts/ggg/.
//
// Each campaign zone has a few layout graphs: fixed points (zone exits, boss arena,
// campsite, …) and the corridors between them, in world units (x east, y north).
// The walls are generated around the corridors per instance, but exits and arenas
// sit where the graph puts them, so the graph tells which way to walk.
//
// Extract the files first with poe_data_tools (https://github.com/adamthedash/poe_data_tools):
//
//   poe_data_tools --patch 2 --steam "<PoE 2 folder>" dump-tables --mode json <tables> \
//     data/balance/worldareas.datc64 data/balance/topologies.datc64
//   poe_data_tools --patch 2 --steam "<PoE 2 folder>" extract <files> 'metadata/terrain/gallows/act1/**/graphs/**'
//
// then:
//
//   node scripts/import-topologies.mjs <tables> <files> [areaId ...]   (default: G1_1 G1_town G1_2)
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const [tables, files, ...areaArgs] = process.argv.slice(2);
if (!tables || !files) {
  console.error('usage: node scripts/import-topologies.mjs <tables folder> <files folder> [areaId ...]');
  process.exit(2);
}
const areas = areaArgs.length ? areaArgs : ['G1_1', 'G1_town', 'G1_2'];
const table = (name) => JSON.parse(readFileSync(join(tables, 'data/balance', `${name}.datc64.json`), 'utf8'));
const worldAreas = new Map(table('worldareas').map((r) => [r.Id, r]));
const topologies = new Map(table('topologies').map((r) => [r.Id, r]));

/** Splits a line into words, keeping "quoted strings" whole (quotes removed). */
function words(line) {
  return [...line.matchAll(/"([^"]*)"|(\S+)/g)].map((m) => m[1] ?? m[2]);
}

function parseGraph(text) {
  const lines = text.replace(/^﻿/, '').replace(/\r/g, '').split('\n');
  let i = 0;
  const header = {};
  while (!lines[i].startsWith('Nodes:')) {
    const at = lines[i].indexOf(':');
    if (at > 0) header[lines[i].slice(0, at).trim()] = lines[i].slice(at + 1).trim();
    i++;
  }
  const nodeCount = Number(lines[i++].split(':')[1]);
  const edgeCount = Number(lines[i++].split(':')[1]);
  while (lines[i].trim() === '""') i++;
  const nodes = [];
  for (let k = 0; k < nodeCount; k++) {
    const t = words(lines[i + k]);
    const degree = Number(t[2]);
    const rest = t.slice(3 + degree);
    const entrance = rest.find((w) => /^entrance\d*$/.test(w)) ?? null;
    nodes.push({ x: Number(t[0]), y: Number(t[1]), tag: rest[0], entrance });
  }
  i += nodeCount;
  const edges = [];
  for (let k = 0; k < edgeCount; k++) {
    const t = words(lines[i + k]);
    const count = Number(t[2]);
    const via = [];
    for (let j = 0; j < count; j++) via.push([Number(t[3 + 2 * j]), Number(t[4 + 2 * j])]);
    const kind = (t.find((w) => w.endsWith('.et')) ?? '').split('/').pop().replace(/\.et$/, '');
    edges.push({ a: Number(t[0]), b: Number(t[1]), via, kind });
  }
  const [cols, rows] = header.Size.split(/\s+/).map(Number);
  return { size: [cols, rows], nodes, edges };
}

const outDir = 'data/layouts/ggg';
mkdirSync(outDir, { recursive: true });
for (const areaId of areas) {
  const area = worldAreas.get(areaId);
  if (!area) throw new Error(`${areaId}: not in WorldAreas`);
  const connections = area.Connections?.Ids ?? [];
  const graphs = [];
  for (const id of area.Topologies?.Ids ?? []) {
    const path = topologies.get(id).DGRFile.toLowerCase();
    const graph = parseGraph(readFileSync(join(files, path), 'utf16le'));
    // "entranceN" leads to the area's N-th connection; a bare "entrance" is where the zone starts.
    for (const node of graph.nodes) {
      const n = node.entrance?.match(/^entrance(\d+)$/);
      node.exit = n ? connections[Number(n[1]) - 1] ?? null : null;
      node.start = node.entrance === 'entrance';
      delete node.entrance;
    }
    graphs.push({ id, file: path, ...graph });
  }
  const connectionNames = connections.map((id) => worldAreas.get(id)?.Name ?? id);
  const out = { areaId, name: area.Name, connections, connectionNames, topologies: graphs };
  writeFileSync(join(outDir, `${areaId}.json`), JSON.stringify(out) + '\n');
  console.log(`${areaId} ${area.Name}: ${graphs.length} layouts`);
}
