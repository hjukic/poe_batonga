// Checks data/icons/ as the reader expects it: node --test scripts/import-icons.test.mjs
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../data/icons");
const index = JSON.parse(fs.readFileSync(path.join(dir, "index.json"), "utf8"));
const sprites = fs.readFileSync(path.join(dir, "sprites.rgba"));
const bytes = index.size * index.size * 4;

test("one 32x32 RGBA sprite per icon, none empty", () => {
  assert.equal(index.size, 32);
  assert.equal(sprites.length, index.icons.length * bytes);
  index.icons.forEach((icon, i) => {
    const sprite = sprites.subarray(i * bytes, (i + 1) * bytes);
    const opaque = [...Array(index.size * index.size).keys()].filter((p) => sprite[p * 4 + 3] >= 230).length;
    assert.ok(opaque > 100, `${icon.id}: ${opaque} opaque pixels`);
    // Row i of MinimapIcons is cell (i % 14, i / 14).
    assert.ok(Number.isInteger(icon.row) && icon.row >= 0);
  });
});

test("the icons the reader needs are there", () => {
  const kinds = Object.fromEntries(index.icons.map((i) => [i.id, i.kind]));
  for (const [id, kind] of [
    ["Entrance", "exit"], ["EntrancePortal", "exit"], ["Waypoint", "waypoint"], ["Checkpoint", "checkpoint"],
    ["CheckpointNotActive", "checkpoint"], ["BossCheckpointActive", "checkpoint"], ["UniqueMonsterAlive", "boss"],
    ["UniqueMonsterDead", "dead"], ["RareMonsterAlive", "rare"], ["RareMonsterDead", "dead"],
    ["NPC", "npc"], ["QuestObject", "quest"], ["QuestItem", "quest"], ["Portal", "portal"],
    ["GrimTangleBossActive", "boss"], ["GraveyardBossInactive", "boss-dead"], ["MillerActive", "boss"],
  ]) {
    assert.equal(kinds[id], kind, id);
  }
  // Measured on the sheet: the Entrance's opaque box (the exit icon is 0.48 of it on screen).
  assert.deepEqual(index.icons.find((i) => i.id === "Entrance").box, [13, 4, 39, 56]);
});

test("boss pairs are complete and agree", () => {
  const pairs = new Map();
  for (const icon of index.icons.filter((i) => i.pair)) {
    const list = pairs.get(icon.pair) ?? [];
    list.push(icon);
    pairs.set(icon.pair, list);
  }
  for (const [stem, list] of pairs) {
    assert.deepEqual(list.map((i) => i.kind).sort(), ["boss", "boss-dead"], stem);
    assert.equal(list[0].area, list[1].area, stem);
    assert.equal(list[0].boss, list[1].boss, stem);
    // An assumed tie never names the boss.
    if (list[0].how === "assumed") assert.equal(list[0].boss, null, stem);
  }
  assert.ok(pairs.size >= 40, `${pairs.size} pairs`);
});
