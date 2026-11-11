#!/usr/bin/env node
// Fetch poe2way's collected zone terrain (walkable-area outline + points of
// interest) for campaign areas and store it compactly under data/layouts/poe2way.
//
//   node scripts/fetch-poe2way-terrain.mjs [areaPrefix ...]   (default: G1_ G2_ G3_)
//
// Area IDs come from data/observations/campaign-zones.json.

import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const outDir = join(root, "data/layouts/poe2way");
const api = "https://www.poe2way.com/api/map-layouts/";
const prefixes = process.argv.slice(2).length ? process.argv.slice(2) : ["G1_", "G2_", "G3_"];

const zones = JSON.parse(await readFile(join(root, "data/observations/campaign-zones.json"), "utf8"));
const ids = zones.areas.map((a) => a.id).filter((id) => prefixes.some((p) => id.startsWith(p)));

const en = (names) => (names && typeof names === "object" ? names.en ?? null : null);

// The outline is either inlined as a data URL or a site path (e.g. /map-layouts/v1/terrain/<sha>.svg).
async function loadSvg(ref) {
  if (ref.startsWith("data:")) return Buffer.from(ref.split(",", 2)[1], "base64").toString("utf8");
  const res = await fetch(new URL(ref, "https://www.poe2way.com"), { headers: { "user-agent": "Mozilla/5.0" } });
  if (!res.ok) throw new Error(`${ref}: HTTP ${res.status}`);
  return res.text();
}

function compact(map) {
  return {
    areaId: map.areaId,
    name: en(map.name),
    releaseId: map.releaseId,
    // Game-space rectangle [minX, minY, maxX, maxY] the outline covers.
    terrain: map.terrain,
    image: { bounds: map.image.bounds, width: map.image.width, height: map.image.height },
    objects: map.objects.map((o) => ({
      id: o.id,
      kind: o.kind,
      x: Number(o.x),
      y: Number(o.y),
      label: en(o.labels),
      destination: o.destination
        ? { areaId: o.destination.areaId ?? null, name: en(o.destination.names), evidence: o.destination.evidence ?? null }
        : null,
      evidence: o.evidence,
    })),
  };
}

await mkdir(outDir, { recursive: true });
const index = { source: "https://www.poe2way.com/atlas", api, fetchedOn: new Date().toISOString().slice(0, 10), areas: {}, missing: [] };

for (const id of ids) {
  const res = await fetch(api + encodeURIComponent(id), { headers: { "user-agent": "Mozilla/5.0" } });
  if (res.status === 404) {
    index.missing.push(id);
    console.log(`${id}: no terrain`);
  } else if (!res.ok) {
    throw new Error(`${id}: HTTP ${res.status}`);
  } else {
    const { map } = await res.json();
    const variants = [map, ...(map.variants ?? [])];
    index.areas[id] = { name: en(map.name), variants: [] };
    for (const [i, v] of variants.entries()) {
      const base = i === 0 ? id : `${id}.v${i}`;
      await writeFile(join(outDir, `${base}.json`), JSON.stringify(compact(v), null, 1) + "\n");
      await writeFile(join(outDir, `${base}.svg`), await loadSvg(v.image.svg));
      index.areas[id].variants.push(base);
    }
    console.log(`${id}: ${variants.length} layout(s), ${map.objects.length} objects`);
  }
  await new Promise((r) => setTimeout(r, 300));
}

await writeFile(join(outDir, "index.json"), JSON.stringify(index, null, 1) + "\n");
console.log(`${Object.keys(index.areas).length} areas, missing: ${index.missing.join(" ") || "none"}`);
