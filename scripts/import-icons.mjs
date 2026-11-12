#!/usr/bin/env node
// Takes the map icons the app reads out of the game's icon sprite sheet into data/icons/.
//
//   node scripts/import-icons.mjs <tables dir> <player.png> [--game-build N] [--game-updated T] [--tool V]
//
// <tables dir> holds data/balance/{minimapicons,worldareas}.datc64.json (poe_data_tools
// dump-tables --mode json); <player.png> is art/2dart/minimap/player.dds as PNG (dump-art).
// Needs ImageMagick (`magick`) to cut the cells. Run by scripts/update-icons.sh.
//
// The sheet is a grid of 64x64 cells, 14 a row: row i of MinimapIcons is cell (i % 14, i / 14).
// Each icon is stored at 32x32 (box filter, alpha-weighted): the sheet's first mip level,
// which is what the game samples when it draws an icon at about half size (0.48 at 2160 px).
// Output: sprites.rgba (32x32 RGBA, 4096 bytes per icon, in index order) and index.json.

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SIZE = 32;
const CELL = 64;
const COLUMNS = 14;

// Generic icons and what the reader calls them.
const GENERIC = {
  Entrance: "exit",
  EntrancePortal: "exit",
  Portal: "portal",
  Waypoint: "waypoint",
  Checkpoint: "checkpoint",
  CheckpointNotActive: "checkpoint",
  BossCheckpointActive: "checkpoint",
  UniqueMonsterAlive: "boss",
  // The two grey skulls are one picture (1 px apart in their cells): a dead unique and a
  // dead rare cannot be told apart.
  UniqueMonsterDead: "dead",
  RareMonsterAlive: "rare",
  RareMonsterDead: "dead",
  NPC: "npc",
  QuestObject: "quest",
  QuestItem: "quest",
};

// Campaign bosses with their own icon pair (<stem>Active / <stem>Inactive, or …BossActive),
// the area each stands in and the checklist's name for it (data/routes, the boss part).
// `how` says what ties them:
//   id      - the icon id holds the area id (G4_7_Boss);
//   name    - the icon id holds the area's or the boss's name (GrimTangle, Ignagduk);
//   monster - the area's boss in WorldAreas (Bosses_MonsterVarietiesKeys) carries the
//             icon id's word (MudBurrower → MudBurrowerHeadBoss); checked below;
//   place   - the icon's place name (the Active row's Name) is the checklist target's label;
//   assumed - read from the names by hand (Crone ↔ UrchinHag): the area only, `boss` left
//             out unless the zone has that one boss.
// `boss` is set only where the tie names the boss; a zone with one checklist boss needs none.
const BOSSES = [
  { stem: "Miller", area: "G1_1", boss: "The Bloated Miller", how: "monster" },
  { stem: "Crone", area: "G1_2", how: "assumed" }, // UrchinHagBoss: a hag; Clearfell's one boss is Beira
  { stem: "MudBurrower", area: "G1_3", boss: "The Devourer", how: "monster" },
  { stem: "CarnivorousPlant", area: "G1_4", how: "monster", checklist: false }, // not a checklist boss (Gerung, Areagne)
  { stem: "GrimTangleBoss", area: "G1_6", how: "name" },
  { stem: "GraveyardBoss", area: "G1_7", how: "name" }, // Cemetery of the Eternals
  { stem: "Husband", area: "G1_8", boss: "Draven", how: "monster" },
  { stem: "Wife", area: "G1_9", boss: "Asinia", how: "monster" },
  { stem: "Crowbell", area: "G1_11", boss: "The Crowbell", how: "monster" },
  { stem: "Executioner", area: "G1_13_2", boss: "The Executioner", how: "monster" },
  { stem: "Gargoyle", area: "G1_15", boss: "Candlemass", how: "monster" }, // ManorGargoyleBoss
  { stem: "Count", area: "G1_15", how: "assumed" }, // BaronBossHumanForm: Count Geonor
  { stem: "Rathbreaker", area: "G2_1", boss: "Rathbreaker", how: "monster" },
  { stem: "Balbala", area: "G2_2", boss: "Balbala", how: "monster" },
  { stem: "PerennialHuman", area: "G2_3", how: "monster" }, // PerennialKingHumanBoss: Jamanra
  { stem: "Kabala", area: "G2_4_1", boss: "Kabala", how: "name" },
  { stem: "ForsakenSon", area: "G2_4_3", how: "monster" }, // Azarian
  { stem: "Mastodon", area: "G2_5_2", how: "monster" }, // MastodonBoss: Ekbab, Iktab's steed
  { stem: "Titan", area: "G2_7", how: "monster" }, // TitanBoss: Zalmarath
  { stem: "TorGul", area: "G2_9_2", boss: "Tor Gul", how: "name" },
  { stem: "Rudja", area: "G2_10_2", boss: "Rudja", how: "monster" },
  { stem: "PerennialCorrupted", area: "G2_12", how: "assumed" }, // PerennialKing: Jamanra
  { stem: "BeetleBoss" }, // "The Galleria": area not known
  { stem: "RootdredgeBoss", area: "G3_1", boss: "Rootdredge", how: "name" },
  { stem: "SilverbackBlackfistBoss", area: "G3_3", how: "assumed" }, // QuadrillaBoss: Mighty Silverfist
  { stem: "SnakePitBoss", area: "G3_4", how: "assumed" }, // The Venom Crypts; no checklist boss
  { stem: "XyclucianBoss", area: "G3_5", boss: "Xyclucian", how: "name" },
  { stem: "BlackjawBoss", area: "G3_6_1", boss: "Blackjaw", how: "monster" },
  { stem: "ZicoatlBoss", area: "G3_6_2", boss: "Zicoatl", how: "name" },
  { stem: "IgnagdukBoss", area: "G3_7", boss: "Ignagduk", how: "monster" },
  { stem: "GoldcrushBoss" }, // "The Aureaduct": area not known
  { stem: "QueenOfFilthBoss", area: "G3_11", boss: "Queen of Filth", how: "name" },
  { stem: "KaazuliBoss", area: "G3_12", how: "assumed" }, // "Altar of the Sun": Ketzuli
  { stem: "ViperNapuatziBoss", area: "G3_14", boss: "Viper Napuatzi", how: "monster" },
  { stem: "DoryaniBoss", area: "G3_17", boss: "Doryani", how: "monster" },
  { stem: "IsleOfKinBoss", area: "G4_1_1", boss: "Blind Beast", how: "place" }, // Primal Arena
  { stem: "VolcanicWarrensBoss", area: "G4_1_2", how: "name" }, // two bosses there
  { stem: "G4_2_2_Boss", area: "G4_2_2", boss: "Captain Hartlin", how: "id" }, // The Captain's Chair
  { stem: "G4_3_1_Boss", area: "G4_3_1", boss: "Great White One", how: "place" }, // Shark Pit
  { stem: "G4_3_1_Crab", area: "G4_3_1", boss: "Clawcrunch", how: "place" }, // Crabshell Cavern
  { stem: "G4_3_2_Boss", area: "G4_3_2", how: "id" },
  { stem: "G4_4_2Boss", area: "G4_4_2", how: "id" },
  { stem: "SolitaryConfinementBoss", area: "G4_5_2", how: "name" },
  { stem: "G4_7_Boss", area: "G4_7", how: "id" },
  { stem: "G4_8b_Boss", area: "G4_8b", how: "id" },
  { stem: "G4_10_Boss", area: "G4_10", how: "id" },
  { stem: "G4_11_2_Boss", area: "G4_11_2", how: "id" },
];
// The word of the icon id found in the area's boss monster id (case-insensitive).
const MONSTER_WORD = { Miller: "Miller", Crowbell: "CrowBell", Gargoyle: "Gargoyle", PerennialHuman: "PerennialKingHuman", ForsakenSon: "ForsakenSon", Mastodon: "Mastodon", Titan: "Titan", BlackjawBoss: "BlackJaw", IgnagdukBoss: "Ignagduk", ViperNapuatziBoss: "ViperNapuatzi", DoryaniBoss: "Doryani", Husband: "Husband", Wife: "Wife", Executioner: "Executioner", Rathbreaker: "Rathbreaker", Balbala: "Balbala", Rudja: "Rudja", MudBurrower: "MudBurrower", CarnivorousPlant: "CarnivorousPlant" };

function args() {
  const [tables, sheet, ...rest] = process.argv.slice(2);
  if (!tables || !sheet) {
    console.error("usage: import-icons.mjs <tables dir> <player.png> [--game-build N] [--game-updated T] [--tool V]");
    process.exit(2);
  }
  const opt = {};
  for (let i = 0; i < rest.length; i += 2) opt[rest[i].replace(/^--/, "")] = rest[i + 1];
  return { tables, sheet, opt };
}

function table(dir, name) {
  return JSON.parse(fs.readFileSync(path.join(dir, "data/balance", `${name}.datc64.json`), "utf8"));
}

// The checklists' bosses per area: the text of each step's boss part.
function checklistBosses() {
  const byArea = {};
  const dir = path.join(repo, "data/routes");
  for (const file of fs.readdirSync(dir).filter((f) => f.endsWith(".json"))) {
    const route = JSON.parse(fs.readFileSync(path.join(dir, file), "utf8"));
    const walk = (o) => {
      if (Array.isArray(o)) return o.forEach(walk);
      if (!o || typeof o !== "object") return;
      for (const part of Array.isArray(o.parts) ? o.parts : []) {
        if (part.style === "boss") (byArea[route.zone.id] ??= []).push(part.text.trim());
      }
      for (const [k, v] of Object.entries(o)) if (k !== "parts") walk(v);
    };
    walk(route);
  }
  return byArea;
}

// One cell as 32x32 RGBA, and the opaque box (alpha >= 128) at full size.
function cut(sheet, row) {
  const [x, y] = [(row % COLUMNS) * CELL, Math.floor(row / COLUMNS) * CELL];
  const crop = ["-crop", `${CELL}x${CELL}+${x}+${y}`, "+repage"];
  const full = execFileSync("magick", [sheet, ...crop, "-depth", "8", "rgba:-"], { maxBuffer: 1 << 20 });
  const small = execFileSync("magick", [sheet, ...crop, "-filter", "Box", "-resize", `${SIZE}x${SIZE}!`, "-depth", "8", "rgba:-"], { maxBuffer: 1 << 20 });
  if (full.length !== CELL * CELL * 4 || small.length !== SIZE * SIZE * 4) throw new Error(`cell ${row}: unexpected size`);
  let [x0, y0, x1, y1] = [CELL, CELL, -1, -1];
  for (let py = 0; py < CELL; py++) {
    for (let px = 0; px < CELL; px++) {
      if (full[(py * CELL + px) * 4 + 3] >= 128) [x0, y0, x1, y1] = [Math.min(x0, px), Math.min(y0, py), Math.max(x1, px), Math.max(y1, py)];
    }
  }
  if (x1 < 0) throw new Error(`cell ${row}: empty`);
  return { small, box: [x0, y0, x1 - x0 + 1, y1 - y0 + 1] };
}

function main() {
  const { tables, sheet, opt } = args();
  const icons = table(tables, "minimapicons");
  const areas = table(tables, "worldareas");
  const rowOf = new Map(icons.map((r, i) => [r.Id, i]));
  const area = new Map(areas.map((a) => [a.Id, a]));
  const checklist = checklistBosses();
  const problems = [];
  const wanted = Object.entries(GENERIC).map(([id, kind]) => ({ id, kind }));
  for (const b of BOSSES) {
    const pair = [["Active", "boss"], ["Inactive", "boss-dead"]].map(([s, kind]) => ({ id: b.stem + s, kind }));
    for (const p of pair) {
      if (!rowOf.has(p.id)) problems.push(`no icon ${p.id}`);
      const a = b.area && area.get(b.area);
      if (b.area && !a) problems.push(`${p.id}: no area ${b.area}`);
      const entry = { ...p, pair: b.stem, how: b.how ?? null, area: b.area ?? null, areaName: a ? a.Name.trim() : null, boss: b.boss ?? null };
      // A pair known to be no checklist boss's: never counted for one.
      if (b.checklist === false) entry.checklist = false;
      if (p.kind === "boss") entry.place = (icons[rowOf.get(p.id)]?.Name ?? "").trim() || null;
      wanted.push(entry);
    }
    if (b.how === "monster") {
      const word = (MONSTER_WORD[b.stem] ?? b.stem).toLowerCase();
      const monsters = area.get(b.area)?.Bosses_MonsterVarietiesKeys?.Ids ?? [];
      if (!monsters.some((m) => m.toLowerCase().includes(word))) problems.push(`${b.stem}: no boss monster with "${word}" in ${b.area} (${monsters.join(", ")})`);
    }
    if (b.boss && !(checklist[b.area] ?? []).includes(b.boss)) problems.push(`${b.stem}: "${b.boss}" is no checklist boss of ${b.area} (${(checklist[b.area] ?? []).join(", ")})`);
  }
  if (problems.length) {
    console.error(problems.join("\n"));
    process.exit(1);
  }
  const out = path.join(repo, "data/icons");
  fs.mkdirSync(out, { recursive: true });
  const blobs = [];
  const index = [];
  for (const w of wanted) {
    const row = rowOf.get(w.id);
    const r = icons[row];
    const { small, box } = cut(sheet, row);
    blobs.push(small);
    index.push({ id: w.id, row, kind: w.kind, minimapRadius: r.MinimapIconRadius, largemapRadius: r.LargemapIconRadius, box, ...Object.fromEntries(Object.entries(w).filter(([k]) => !["id", "kind"].includes(k))) });
  }
  fs.writeFileSync(path.join(out, "sprites.rgba"), Buffer.concat(blobs));
  const meta = {
    sheet: "art/2dart/minimap/player.dds",
    cell: CELL,
    columns: COLUMNS,
    size: SIZE,
    imported: new Date().toISOString().slice(0, 10),
    ...(opt["game-build"] ? { gameBuild: opt["game-build"] } : {}),
    ...(opt["game-updated"] ? { gameUpdated: Number(opt["game-updated"]) } : {}),
    ...(opt.tool ? { tool: opt.tool } : {}),
  };
  // One icon a line: diffs after a patch stay readable.
  const head = JSON.stringify(meta, null, 1).replace(/\n}$/, "");
  fs.writeFileSync(path.join(out, "index.json"), `${head},\n "icons": [\n${index.map((i) => "  " + JSON.stringify(i)).join(",\n")}\n ]\n}\n`);
  const pairs = BOSSES.length;
  console.log(`${index.length} icons (${Object.keys(GENERIC).length} generic, ${pairs} boss pairs, ${BOSSES.filter((b) => b.area).length} with an area, ${BOSSES.filter((b) => b.boss).length} naming their boss), ${(blobs.length * SIZE * SIZE * 4) / 1024} KiB`);
}

main();
