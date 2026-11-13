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
//   node scripts/import-topologies.mjs --list <tables>          prints the graph files to extract
//   node scripts/import-topologies.mjs --list <tables> <files>  then what is still missing there: the
//                                                               master.tsi, .fgp and sub-graph files
//                                                               (repeat until it prints nothing)
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

/** Campaign areas: acts 1–4 (`G1_…` to `G4_…`; the interludes `P1_…` to `P3_…` are left out: the 1.0 scope
 *  has no interludes, owner 2026-11-12), with at least one topology. Not world maps, map areas, hideouts, or areas named "[DNT…]" (unused). */
export function isCampaignArea(row) {
  return (
    /^G\d+_/.test(row.Id) &&
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
    // The words that say where the node leads: "entrance" (the zone starts here) and "entranceN",
    // tag or key. A node can be both: the start is often also the way back.
    const start = rest.includes('entrance');
    const exitWords = rest.filter((w) => /^entrance\d+$/.test(w));
    // "entranceNdisabled": a way out the game opens later (Ogham Village's to the Manor
    // Ramparts, through the Executioner's arena; the Halani Gates'). Linked below where no
    // node of the layout leads there otherwise.
    const laterWords = rest.filter((w) => /^entrance\d+disabled$/.test(w));
    const keys = t.slice(4 + degree).filter((w) => w.quoted && w.text !== '').map((w) => w.text);
    // The word after the tag is the node's turn: I, FI, R90, FR270, (any), (best), ….
    const turn = t[4 + degree]?.text ?? 'I';
    nodes.push({ x: Number(t[0].text), y: Number(t[1].text), tag: tag.text, start, exitWords, laterWords, keys, turn });
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

const GRID = 23;
const norm = (path) => path.toLowerCase().replace(/\/+/g, '/');
const dirOf = (path) => path.replace(/\/[^/]*$/, '');

/** The FileGroups (.fgp) files a master.tsi names, relative to its folder. */
export function fileGroupsOf(masterText) {
  return [...(masterText ?? '').matchAll(/^\s*FileGroups\s+"([^"]+)"/gm)].map((m) => m[1]);
}

/** A .fgp file: groups of files under a quoted name. Paths are relative to `base` unless they
 *  start with "Metadata/". Returns Map(name → [lower-case paths]). */
export function parseFileGroups(text, base) {
  const groups = new Map();
  let current = null;
  for (const line of text.replace(/^﻿/, '').replace(/\r/g, '').split('\n')) {
    const word = line.trim().replace(/^"|"$/g, '');
    if (!word || /^version \d+$/.test(word)) continue;
    if (current && /\.[a-z0-9]+$/i.test(word)) groups.get(current).push(norm(/^metadata\//i.test(word) ? word : `${base}/${word}`));
    else groups.set((current = word), []);
  }
  return groups;
}

/** A "graph" node is a placeholder for a sub-graph: keys = [file or group name, width, height]. */
const isPlaceholder = (node) => node.tag === 'graph' && node.keys.length >= 3;

/** The files a placeholder can stand for, and the files still missing to know them. */
function placeholderFiles(node, master, src) {
  const ref = node.keys[0];
  if (/\.(dgr|tgr)$/i.test(ref)) return { files: [norm(ref)], missing: [] };
  if (!master) return { files: [], missing: [] };
  const masterText = src.text(master);
  if (masterText == null) return { files: [], missing: [master] };
  const files = [];
  const missing = [];
  for (const name of fileGroupsOf(masterText)) {
    const path = norm(`${dirOf(master)}/${name}`);
    const text = src.text(path);
    if (text == null) missing.push(path);
    else files.push(...(parseFileGroups(text, dirOf(master)).get(ref) ?? []).filter((f) => /\.(dgr|tgr)$/.test(f)));
  }
  return { files, missing };
}

/** Every file needed to import `roots` that `src` does not have yet: graphs, their master.tsi,
 *  the master's .fgp files, and the sub-graphs placeholders name (recursively). */
export function missingFiles(roots, src) {
  const missing = new Set();
  const seen = new Set();
  const queue = [...roots];
  while (queue.length) {
    const path = queue.shift();
    if (seen.has(path)) continue;
    seen.add(path);
    const text = src.text(path);
    if (text == null) {
      missing.add(path);
      continue;
    }
    const graph = parseGraph(text, path);
    if (graph.master && src.text(graph.master) == null) missing.add(graph.master);
    for (const node of graph.nodes.filter(isPlaceholder)) {
      const { files, missing: more } = placeholderFiles(node, graph.master, src);
      more.forEach((m) => missing.add(m));
      queue.push(...files);
    }
  }
  return [...missing];
}

/** Moves a sub-graph point into the parent. The placeholder's cell is the corner of its box with
 *  the lowest x and y; the box is keys[1] × keys[2] parent cells. Turns: R90 a quarter turn
 *  counter-clockwise (x east, y north), F a mirror of x first; the direction is a guess (only two
 *  placeholders, in the Halani Gates, are turned). Most sub-graphs have the box's size in cells and
 *  the parent's RoomUnitSize (scale 1); the others (Freythorn, Halani Gates, Sandswept Marsh) are on
 *  another grid and are scaled to fit the box, keeping their shape. Returns {place, scale}. */
function placer(node, subSize) {
  const origin = [Math.floor(node.x / GRID) * GRID, Math.floor(node.y / GRID) * GRID];
  const [w, h] = [subSize[0] * GRID, subSize[1] * GRID];
  const m = node.turn.match(/^(F?)(I|R(\d+))$/);
  const flip = m?.[1] === 'F';
  const quarter = m ? (Number(m[3] ?? 0) / 90) % 4 : 0;
  const [tw, th] = quarter % 2 ? [subSize[1], subSize[0]] : subSize;
  const [bw, bh] = [Number(node.keys[1]), Number(node.keys[2])];
  const same = tw === bw && th === bh;
  const scale = same || !(bw > 0 && bh > 0) ? 1 : Math.min(bw / tw, bh / th);
  const place = ([x, y]) => {
    let [u, v] = [flip ? w - x : x, y];
    if (quarter === 1) [u, v] = [h - v, u];
    else if (quarter === 2) [u, v] = [w - u, h - v];
    else if (quarter === 3) [u, v] = [v, w - u];
    return [Math.round((origin[0] + u * scale) * 10) / 10, Math.round((origin[1] + v * scale) * 10) / 10];
  };
  return { place, scale: Math.round(scale * 1000) / 1000 };
}

/** The ways a graph file expands: one per choice of files for its placeholders (all choices when
 *  `multiply`, else the first file of each). Each: {nodes, edges, picks, parts, alternatives, guessed}. */
function expansions(path, src, multiply, depth = 0) {
  const graph = parseGraph(src.text(path), path);
  const base = { graph, nodes: graph.nodes, edges: graph.edges, picks: [], parts: [], alternatives: [], guessed: 0 };
  if (depth > 4) return [base];
  // Per placeholder: the options it can expand to (empty: kept as it is).
  const slots = graph.nodes.map((node, index) => {
    if (!isPlaceholder(node)) return null;
    const files = placeholderFiles(node, graph.master, src).files.filter((f) => src.text(f) != null);
    if (!files.length) return null;
    const chosen = multiply ? files : files.slice(0, 1);
    const options = chosen.flatMap((file, k) =>
      expansions(file, src, multiply, depth + 1).map((sub) => ({ file, pick: files.length > 1 ? k + 1 : null, sub })),
    );
    const alternatives = !multiply && files.length > 1 ? [{ name: node.keys[0], files }] : [];
    return { index, node, options, alternatives };
  }).filter(Boolean);
  if (!slots.length) return [base];
  let combos = [[]];
  for (const slot of slots) combos = combos.flatMap((combo) => slot.options.map((option) => [...combo, option]));
  return combos.map((combo) => {
    const dropped = new Set(slots.map((s) => s.index));
    const remap = new Map();
    const nodes = [];
    graph.nodes.forEach((node, i) => {
      if (!dropped.has(i)) {
        remap.set(i, nodes.length);
        nodes.push(node);
      }
    });
    const edges = graph.edges
      .filter((e) => remap.has(e.a) && remap.has(e.b))
      .map((e) => ({ ...e, a: remap.get(e.a), b: remap.get(e.b) }));
    const out = { graph, nodes, edges, picks: [], parts: [], alternatives: [], guessed: 0 };
    combo.forEach((option, k) => {
      const { node, alternatives } = slots[k];
      const { sub } = option;
      const { place, scale } = placer(node, sub.graph.size);
      const offset = nodes.length;
      for (const n of sub.nodes) {
        const [x, y] = place([n.x, n.y]);
        nodes.push({ ...n, x, y });
      }
      for (const e of sub.edges) edges.push({ ...e, a: e.a + offset, b: e.b + offset, via: e.via.map(place) });
      const at = place([0, 0]);
      out.parts.push(
        { file: option.file, at, turn: node.turn, scale },
        ...sub.parts.map((p) => ({ ...p, at: place(p.at), scale: Math.round(p.scale * scale * 1000) / 1000 })),
      );
      if (option.pick) out.picks.push(option.pick);
      out.picks.push(...sub.picks);
      out.alternatives.push(...alternatives.map((a) => ({ ...a, at })), ...sub.alternatives.map((a) => ({ ...a, at: place(a.at) })));
      out.guessed += sub.guessed + (/^\(/.test(node.turn) || node.turn.startsWith('R') || node.turn.startsWith('F') ? 1 : 0);
    });
    return out;
  });
}

/** Areas may multiply their layouts by sub-graph choices up to this many layouts in all. */
const MAX_LAYOUTS = 60;

/** The area's JSON (the "Layout JSON" contract in the build plan; field order kept from before).
 *  `src.text(path)` gives an extracted file's text, or null when it is missing. */
export function importArea(area, topologies, worldAreas, src) {
  const connections = area.Connections?.Ids ?? [];
  const ids = area.Topologies?.Ids ?? [];
  const paths = ids.map((id) => {
    const topology = topologies.get(id);
    if (!topology) throw new Error(`${area.Id}: topology ${id} not in Topologies`);
    return topology.DGRFile.toLowerCase();
  });
  // Every choice of sub-graph files is a layout of its own, while the area stays under MAX_LAYOUTS;
  // above it each placeholder keeps its first file and lists the others in `alternatives`.
  let expanded = paths.map((path) => expansions(path, src, true));
  const multiply = expanded.reduce((n, list) => n + list.length, 0) <= MAX_LAYOUTS;
  if (!multiply) expanded = paths.map((path) => expansions(path, src, false));
  const graphs = [];
  ids.forEach((id, k) => {
    const path = paths[k];
    const format = path.endsWith('.tgr') ? 'tgr' : 'dgr';
    for (const ex of expanded[k]) {
      // "entranceN" leads to the area's N-th connection; a bare "entrance" is where the zone starts.
      // The first such word that names a connection counts (Red Vale has rooms tagged "entrance2"
      // with the key "entrance1" and one connection).
      const outNodes = ex.nodes.map((node) => {
        const exit = node.exitWords.map((w) => connections[Number(w.slice(8)) - 1]).find(Boolean) ?? null;
        return {
          x: node.x,
          y: node.y,
          tag: node.tag,
          exit,
          start: node.start,
          keys: node.keys,
        };
      });
      // A way out opened later ("entranceNdisabled"), where no node leads to that connection:
      // without it Ogham Village knew no way to the Manor Ramparts, and two exits read by name
      // fitted none of its layouts (the owner, 2026-11-13).
      ex.nodes.forEach((node, i) => {
        if (outNodes[i].exit || !node.laterWords?.length) return;
        const later = node.laterWords.map((w) => connections[parseInt(w.slice(8), 10) - 1]).find(Boolean);
        if (later && !outNodes.some((n) => n.exit === later)) outNodes[i].exit = later;
      });
      // Act 4's islands (Shrike Island, Plunder's Point, Brinerot Cove) have no entranceN: the way
      // out is the boat at the landing, keyed "teleport1" (Ngakanu's landing has both "entrance1"
      // and "teleport1", leading to Kingsmarch). With one connection and no other exit, it is that.
      if (connections.length === 1 && !outNodes.some((n) => n.exit)) {
        for (const n of outNodes) if (n.keys.includes('teleport1')) n.exit = connections[0];
      }
      // .dgr only: a grid cell (23 grid units) is a room of RoomUnitSize tiles; rooms share their border tile.
      const master = ex.graph.master;
      const extra = format === 'dgr' ? { roomUnitSize: master ? roomUnitSize(src.text(master)) : null } : {};
      if (ex.parts.length) {
        Object.assign(extra, { expanded: true, parts: ex.parts, guessedTurns: ex.guessed });
        if (ex.alternatives.length) extra.alternatives = ex.alternatives;
      }
      const suffix = expanded[k].length > 1 ? `#${ex.picks.join('-')}` : '';
      graphs.push({ id: id + suffix, file: path, format, size: ex.graph.size, ...extra, nodes: outNodes, edges: ex.edges });
    }
  });
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

/** Reads extracted files: UTF-16 (with or without BOM) or UTF-8 text; null when missing. */
export function fileSource(folder) {
  const cache = new Map();
  return {
    text(path) {
      if (!cache.has(path)) {
        let text = null;
        try {
          const bytes = readFileSync(join(folder, path));
          text = bytes[0] === 0xff || bytes[1] === 0 ? bytes.toString('utf16le') : bytes.toString('utf8');
        } catch {}
        cache.set(path, text);
      }
      return cache.get(path);
    },
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
    if (argv[k] === '--list') options.list = true;
    else if (argv[k].startsWith('--')) options[argv[k].slice(2)] = argv[++k];
    else args.push(argv[k]);
  }
  const [tables, files, ...areaArgs] = args;
  if (!tables || (!files && !options.list)) {
    console.error('usage: node scripts/import-topologies.mjs --list <tables folder> [files folder]');
    console.error('       node scripts/import-topologies.mjs <tables folder> <files folder> [options] [areaId ...]');
    process.exit(2);
  }
  const { rows, worldAreas, topologies } = loadTables(tables);
  const campaign = rows.filter(isCampaignArea);
  const roots = [...new Set(campaign.flatMap((a) => a.Topologies.Ids.map((id) => topologies.get(id).DGRFile.toLowerCase())))];
  if (options.list) {
    // Without a files folder: the graphs the tables name. With one: what is still missing there.
    const list = files ? missingFiles(roots, fileSource(files)) : roots;
    if (list.length) process.stdout.write(list.join('\n') + '\n');
    return;
  }
  const src = fileSource(files);
  const full = areaArgs.length === 0;
  const areas = full ? campaign : areaArgs.map((id) => worldAreas.get(id) ?? (() => { throw new Error(`${id}: not in WorldAreas`); })());
  mkdirSync(OUT_DIR, { recursive: true });
  const index = [];
  for (const area of areas) {
    const out = importArea(area, topologies, worldAreas, src);
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
