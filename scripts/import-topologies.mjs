#!/usr/bin/env node
// Imports the game's zone layout graphs (.tgr and .dgr "topologies") into data/layouts/ggg/.
//
// Each campaign zone has one or more layout graphs: fixed points (zone exits, boss arena,
// campsite, …) and the corridors between them. Outdoor zones use .tgr files, in world
// units (23 per tile, x east, y north). Dungeons, towns and most indoor zones use .dgr
// files: the same text format, but on a room grid (see docs/data-sources.md).
// The walls are generated around the corridors per instance, but exits and arenas
// sit where the graph puts them, so the graph tells which way to walk.
//
// scripts/update-layouts.sh does the whole refresh from the game files. By hand:
//
//   node scripts/import-topologies.mjs --list <tables>             prints the graph files to extract
//   node scripts/import-topologies.mjs --masters <tables> <files>  then the master.tsi files they name
//   node scripts/import-topologies.mjs <tables> <files> [options] [areaId ...]
//
// <tables> holds data/balance/{worldareas,topologies}.datc64.json (poe_data_tools dump-tables --mode json),
// <files> the extracted graph files (lower-case paths). Without area ids it writes every campaign area,
// index.json, and removes files of areas that are gone. Options (recorded in index.json):
//   --game-build <id>  --game-updated <unix seconds>  --tool <name and version>
import { mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

const OUT_DIR = 'data/layouts/ggg';

/** Campaign areas: acts 1–4 (`G1_…` to `G4_…`) and the interludes (`P1_…` to `P3_…`, act 6 in the table),
 *  with at least one topology. Not world maps, map areas, hideouts, or areas named "[DNT…]" (unused). */
export function isCampaignArea(row) {
  return (
    /^(G\d+|P\d+)_/.test(row.Id) &&
    !row.Id.endsWith('_WorldMap') &&
    !row.IsMapArea &&
    !row.IsHideout &&
    !/^\[DNT/.test(row.Name ?? '') &&
    (row.Topologies?.Ids ?? []).length > 0
  );
}

/** Splits a line into tokens, keeping "quoted strings" whole: {text, quoted}. */
function tokens(line) {
  return [...line.matchAll(/"([^"]*)"|(\S+)/g)].map((m) => ({ text: m[1] ?? m[2], quoted: m[1] !== undefined }));
}

const NUMBER_LINE = /^-?\d+ -?\d+ \d+ /;

/** Parses a .tgr or .dgr graph (UTF-16 text). Throws when a line does not have the expected shape. */
export function parseGraph(text, name = 'graph') {
  const lines = text.replace(/^﻿/, '').replace(/\r/g, '').split('\n');
  const fail = (why) => {
    throw new Error(`${name}: ${why}`);
  };
  let i = 0;
  const header = {};
  while (i < lines.length && !lines[i].startsWith('Nodes:')) {
    const at = lines[i].indexOf(':');
    if (at > 0) header[lines[i].slice(0, at).trim()] = lines[i].slice(at + 1).trim();
    i++;
  }
  if (i >= lines.length) fail('no "Nodes:" line');
  const nodeCount = Number(lines[i++].split(':')[1]);
  const edgeCount = Number(lines[i++].split(':')[1]);
  // Between the counts and the first node: ground-type lines ("" or "….gt"), and in .dgr files
  // "Default%: …" and a lone number. A node or edge line starts with three numbers.
  while (i < lines.length && !NUMBER_LINE.test(lines[i])) i++;
  const nodes = [];
  for (let k = 0; k < nodeCount; k++) {
    const t = tokens(lines[i + k] ?? '');
    const degree = Number(t[2]?.text);
    const tag = t[3 + degree];
    if (!tag?.quoted) fail(`node ${k}: no quoted tag after ${degree} neighbours`);
    const rest = t.slice(3 + degree).map((w) => w.text);
    // The words that say where the node leads: "entrance" (the zone starts here) and the first
    // "entranceN", tag or key. A node can be both: the start is often also the way back.
    const start = rest.includes('entrance');
    const exitWord = rest.find((w) => /^entrance\d+$/.test(w)) ?? null;
    const keys = t.slice(4 + degree).filter((w) => w.quoted && w.text !== '').map((w) => w.text);
    nodes.push({ x: Number(t[0].text), y: Number(t[1].text), tag: tag.text, start, exitWord, keys });
  }
  i += nodeCount;
  const edges = [];
  for (let k = 0; k < edgeCount; k++) {
    const t = tokens(lines[i + k] ?? '');
    const count = Number(t[2]?.text);
    if (!NUMBER_LINE.test(lines[i + k] ?? '') || !Number.isInteger(count)) fail(`edge ${k}: not an edge line`);
    const via = [];
    for (let j = 0; j < count; j++) via.push([Number(t[3 + 2 * j].text), Number(t[4 + 2 * j].text)]);
    const kind = (t.find((w) => w.text.endsWith('.et'))?.text ?? '').split('/').pop().replace(/\.et$/, '');
    const edge = { a: Number(t[0].text), b: Number(t[1].text), via, kind };
    if (!(edge.a < nodeCount && edge.b < nodeCount) || via.flat().some(Number.isNaN)) fail(`edge ${k}: bad numbers`);
    edges.push(edge);
  }
  const [cols, rows] = (header.Size ?? fail('no "Size:"')).split(/\s+/).map(Number);
  const master = header.MasterFile?.replace(/^"|"$/g, '').toLowerCase() || null;
  return { version: Number(header.version ?? lines[0].split(' ')[1]), master, size: [cols, rows], nodes, edges };
}

/** "RoomUnitSize" from a master.tsi (UTF-16 or UTF-8 text): tiles per room of a .dgr grid cell. */
export function roomUnitSize(text) {
  const n = text?.match(/^\s*RoomUnitSize\s+(\d+)/m)?.[1];
  return n ? Number(n) : null;
}

/** The area's JSON (the "Layout JSON" contract in the build plan; field order kept from before).
 *  `readMaster(path)` gives a master.tsi's text, or null when it was not extracted. */
export function importArea(area, topologies, worldAreas, readGraph, readMaster = () => null) {
  const connections = area.Connections?.Ids ?? [];
  const graphs = [];
  for (const id of area.Topologies?.Ids ?? []) {
    const topology = topologies.get(id);
    if (!topology) throw new Error(`${area.Id}: topology ${id} not in Topologies`);
    const path = topology.DGRFile.toLowerCase();
    const format = path.endsWith('.tgr') ? 'tgr' : 'dgr';
    const { master, size, nodes, edges } = parseGraph(readGraph(path), path);
    // "entranceN" leads to the area's N-th connection; a bare "entrance" is where the zone starts.
    const outNodes = nodes.map((node) => {
      const n = node.exitWord?.match(/^entrance(\d+)$/);
      return {
        x: node.x,
        y: node.y,
        tag: node.tag,
        exit: n ? connections[Number(n[1]) - 1] ?? null : null,
        start: node.start,
        keys: node.keys,
      };
    });
    // .dgr only: a grid cell (23 grid units) is a room of RoomUnitSize tiles; rooms share their border tile.
    const extra = format === 'dgr' ? { roomUnitSize: master ? roomUnitSize(readMaster(master)) : null } : {};
    graphs.push({ id, file: path, format, size, ...extra, nodes: outNodes, edges });
  }
  const connectionNames = connections.map((id) => worldAreas.get(id)?.Name ?? id);
  return {
    areaId: area.Id,
    name: area.Name.trim(),
    act: area.Act,
    level: area.AreaLevel,
    connections,
    connectionNames,
    topologies: graphs,
  };
}

function loadTables(tables) {
  const table = (name) => JSON.parse(readFileSync(join(tables, 'data/balance', `${name}.datc64.json`), 'utf8'));
  const rows = table('worldareas');
  return {
    rows,
    worldAreas: new Map(rows.map((r) => [r.Id, r])),
    topologies: new Map(table('topologies').map((r) => [r.Id, r])),
  };
}

function main(argv) {
  const options = {};
  const args = [];
  for (let k = 0; k < argv.length; k++) {
    if (argv[k] === '--list' || argv[k] === '--masters') options[argv[k].slice(2)] = true;
    else if (argv[k].startsWith('--')) options[argv[k].slice(2)] = argv[++k];
    else args.push(argv[k]);
  }
  const [tables, files, ...areaArgs] = args;
  if (!tables || (!files && !options.list)) {
    console.error('usage: node scripts/import-topologies.mjs --list <tables folder>');
    console.error('       node scripts/import-topologies.mjs --masters <tables folder> <files folder>');
    console.error('       node scripts/import-topologies.mjs <tables folder> <files folder> [options] [areaId ...]');
    process.exit(2);
  }
  const { rows, worldAreas, topologies } = loadTables(tables);
  const campaign = rows.filter(isCampaignArea);
  if (options.list) {
    const paths = new Set(campaign.flatMap((a) => a.Topologies.Ids.map((id) => topologies.get(id).DGRFile.toLowerCase())));
    process.stdout.write([...paths].join('\n') + '\n');
    return;
  }
  const readGraph = (path) => readFileSync(join(files, path), 'utf16le');
  if (options.masters) {
    const paths = new Set(campaign.flatMap((a) => a.Topologies.Ids.map((id) => topologies.get(id).DGRFile.toLowerCase())));
    const masters = new Set([...paths].filter((p) => p.endsWith('.dgr')).map((p) => parseGraph(readGraph(p), p).master).filter(Boolean));
    process.stdout.write([...masters].join('\n') + '\n');
    return;
  }
  const readMaster = (path) => {
    try {
      const bytes = readFileSync(join(files, path));
      return bytes[0] === 0xff || bytes[1] === 0 ? bytes.toString('utf16le') : bytes.toString('utf8');
    } catch {
      return null;
    }
  };
  const full = areaArgs.length === 0;
  const areas = full ? campaign : areaArgs.map((id) => worldAreas.get(id) ?? (() => { throw new Error(`${id}: not in WorldAreas`); })());
  mkdirSync(OUT_DIR, { recursive: true });
  const index = [];
  for (const area of areas) {
    const out = importArea(area, topologies, worldAreas, readGraph, readMaster);
    writeFileSync(join(OUT_DIR, `${area.Id}.json`), JSON.stringify(out) + '\n');
    const formats = [...new Set(out.topologies.map((t) => t.format))];
    index.push({ id: out.areaId, name: out.name, act: out.act, level: out.level, layouts: out.topologies.length, format: formats.join('+') });
    console.log(`${out.areaId} ${out.name}: ${out.topologies.length} layouts (${formats.join('+')})`);
  }
  if (!full) return;
  const kept = new Set(areas.map((a) => `${a.Id}.json`));
  for (const name of readdirSync(OUT_DIR)) {
    if (name.endsWith('.json') && name !== 'index.json' && !kept.has(name)) {
      rmSync(join(OUT_DIR, name));
      console.log(`removed ${name} (area gone)`);
    }
  }
  const game = { steamBuild: options['game-build'] ?? null, updated: options['game-updated'] ? new Date(Number(options['game-updated']) * 1000).toISOString() : null };
  const meta = { imported: new Date().toISOString().slice(0, 10), game, tool: options.tool ?? null, areas: index };
  writeFileSync(join(OUT_DIR, 'index.json'), JSON.stringify(meta) + '\n');
  console.log(`${index.length} areas, ${index.reduce((n, a) => n + a.layouts, 0)} layouts`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main(process.argv.slice(2));
