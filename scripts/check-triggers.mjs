#!/usr/bin/env node
// Checks every `npcLine` trigger in data/routes/ against the game's dialogue tables, so a
// patch that changes or drops a line shows up before play does. Run it after each patch:
//
//   scripts/check-triggers.sh [game folder]        dumps the tables and runs this check
//   node scripts/check-triggers.mjs <tables> [--routes <dir>] [--json]
//
// <tables> holds data/balance/{npctextaudio,npcs}.datc64.json (poe_data_tools dump-tables
// --mode json), and for panel dialogue npctalkdialoguetextaudio.datc64.json; npctalk and
// npctalkdialoguesteps, when present, tell world speech (bark) from panel text.
//
// For each line of a trigger: the row (`id`) still exists in NPCTextAudio or
// NPCTalkDialogueTextAudio, its text in shown form is the trigger's text, and the speaker is
// the `Name` of one of the row's NPCs (NPCTextAudio.NPCs, NPCTalkDialogueTextAudio.NPC1).
// A line without an id (the two first triggers) is looked up by speaker and text.
// Warnings: random or class variants of a line that the trigger lacks, a trigger bound to
// another zone than its checklist's, an evidenceKind that does not match the table, and a
// line said in more than one way: its text in another row of the same speaker, its row
// spoken by another NPC too, or its row named by NPCTalk options that differ in NPC, quest
// flags or zone.
// Exit code 1 when any line no longer matches; the report says what changed.
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

/** A line's text as the log shows it; the same rules as `shown_text` in src-tauri/src/tracker.rs:
 *  `<i>{word}` becomes `word`, other tags and every run of whitespace become one space. */
export function shownText(text) {
  return text
    .replace(/<[^>]*>\{([^}]*)\}/g, '$1')
    .replace(/<[^>]*>/g, ' ')
    .split(/\s+/u)
    .filter(Boolean)
    .join(' ');
}

const CLASS = /_?(Sorc|Sorceress|Warrior|Witch|Ranger|Monk|Merc|Mercenary|Huntress|Druid|Gemling|Infernalist|Templar|Marauder|Karui|Ezomyte|Male|Female|Son|(Str|Dex|Int|StrDex|StrInt|DexInt)Four[bB]?)$/;

/** The id without its variant suffix: random (`…Random`, `…_2`, `…Random2`) and class or
 *  gender variants (`…Sorc`, `…StrFourb`, `…Female`). Siblings share it. */
export function variantStem(id) {
  return id.replace(/_?\d+$/, '').replace(/_?Random$/, '').replace(CLASS, '');
}

const ids = (ref) => (ref == null ? [] : Array.isArray(ref) ? ref.flatMap(ids) : ref.Ids ?? (ref.Id ? [ref.Id] : []));

function readTable(dir, name, required) {
  for (const path of [join(dir, 'data/balance', `${name}.datc64.json`), join(dir, `${name}.datc64.json`)]) {
    if (existsSync(path)) return JSON.parse(readFileSync(path, 'utf8'));
  }
  if (required) throw new Error(`${name}.datc64.json not found in ${dir} (or its data/balance/)`);
  return null;
}

/** The dialogue lines of a table dump in `dir`, by id (see `linesFromTables`). */
export function loadLines(dir) {
  const table = (name, required) => readTable(dir, name, required);
  return linesFromTables({
    npcs: table('npcs', true),
    npctextaudio: table('npctextaudio', true),
    npctalkdialoguetextaudio: table('npctalkdialoguetextaudio', false),
    npctalk: table('npctalk', false),
    npctalkdialoguesteps: table('npctalkdialoguesteps', false),
  });
}

/** The dialogue lines by id: {id, table, text (shown form), speakers, shortNames, kind, talks}.
 *  `kind` is "panel" for lines a dialogue panel shows (named by an NPCTalk option or by a
 *  dialogue's steps, in either of its columns, and every NPCTalkDialogueTextAudio line),
 *  "bark" for the rest, null when NPCTalk or NPCTalkDialogueSteps is missing. `talks` lists
 *  the NPCTalk options that name the line: {npc, option, flags, areas}. */
export function linesFromTables({ npcs, npctextaudio, npctalkdialoguetextaudio, npctalk, npctalkdialoguesteps }) {
  const byId = new Map(npcs.map((n) => [n.Id, n]));
  const names = (refs) => [...new Set(ids(refs).map((i) => byId.get(i)?.Name).filter(Boolean))];
  const shortNames = (refs) => [...new Set(ids(refs).map((i) => byId.get(i)?.ShortName).filter(Boolean))];
  const panel =
    npctalk && npctalkdialoguesteps
      ? new Set([
          ...npctalk.flatMap((r) => ids(r.NPCTextAudioKeys)),
          ...npctalkdialoguesteps.flatMap((s) => [...ids(s.NPCTextAudio), ...ids(s.NPCTalkDialogueTextAudio)]),
        ])
      : null;
  // The NPCTalk options by line id. `flags` joins every quest-flag column of the option (the
  // flags it needs, hides on and sets); two options that differ there come at different times.
  const talks = new Map();
  for (const r of npctalk ?? []) {
    const talk = {
      npc: byId.get(r.NPCKey?.Id)?.Name ?? r.NPCKey?.Id ?? '?',
      option: r.DialogueOption ?? '',
      flags: ['unknown_1', 'unknown_2', 'unknown_3', 'QuestFlags1', 'QuestFlag2', 'QuestFlag3'].map((c) => ids(r[c]).join('+')).join('/'),
      areas: [...ids(r.WorldArea1), ...ids(r.WorldArea2)].join('+'),
    };
    for (const id of ids(r.NPCTextAudioKeys)) {
      if (!talks.has(id)) talks.set(id, []);
      talks.get(id).push(talk);
    }
  }
  const lines = new Map();
  for (const r of npctextaudio) {
    lines.set(r.Id, {
      id: r.Id,
      table: 'NPCTextAudio',
      text: shownText(r.Text ?? ''),
      speakers: names(r.NPCs),
      shortNames: shortNames(r.NPCs),
      kind: panel ? (panel.has(r.Id) ? 'panel' : 'bark') : null,
      talks: talks.get(r.Id) ?? [],
    });
  }
  for (const r of npctalkdialoguetextaudio ?? []) {
    // An id in both tables keeps its NPCTextAudio row (same text); its kind comes from the
    // dialogue steps above.
    if (lines.has(r.Id)) continue;
    lines.set(r.Id, {
      id: r.Id,
      table: 'NPCTalkDialogueTextAudio',
      text: shownText(r.Text ?? ''),
      speakers: names(r.NPC1),
      shortNames: shortNames(r.NPC1),
      kind: 'panel',
      talks: talks.get(r.Id) ?? [],
    });
  }
  return lines;
}

/** The checklists in `dir`, each with its file name. */
export function loadRoutes(dir) {
  return readdirSync(dir)
    .filter((n) => n.endsWith('.json'))
    .sort()
    .map((file) => ({ file, ...JSON.parse(readFileSync(join(dir, file), 'utf8')) }));
}

/** The lines of an npcLine trigger: the `lines` form or the one-line speaker + text form. */
export function triggerLines(trigger) {
  return trigger.lines ?? [{ speaker: trigger.speaker, text: trigger.text }];
}

/** Checks every npcLine trigger. Returns {triggers, errors, warnings}; each finding names the step. */
export function check(routes, lines) {
  const errors = [];
  const warnings = [];
  const byText = new Map();
  for (const line of lines.values()) {
    if (!byText.has(line.text)) byText.set(line.text, []);
    byText.get(line.text).push(line);
  }
  const byStem = new Map();
  for (const line of lines.values()) {
    const stem = variantStem(line.id);
    if (!byStem.has(stem)) byStem.set(stem, []);
    byStem.get(stem).push(line);
  }
  let triggers = 0;
  for (const route of routes) {
    for (const step of route.steps) {
      const trigger = step.trigger;
      if (trigger?.type !== 'npcLine') continue;
      triggers++;
      const where = `${route.file} ${step.id}`;
      if (trigger.area !== route.zone.id) warnings.push(`${where}: bound to ${trigger.area}, the checklist is ${route.zone.id}`);
      const found = [];
      for (const want of triggerLines(trigger)) {
        if (shownText(want.text) !== want.text) errors.push(`${where}: text not in shown form: ${JSON.stringify(want.text)}`);
        let row = want.id ? lines.get(want.id) : null;
        if (want.id && !row) {
          errors.push(`${where}: ${want.id} is gone from the game data (was ${want.speaker}: ${JSON.stringify(want.text)})`);
          continue;
        }
        if (!want.id) {
          const candidates = (byText.get(want.text) ?? []).filter((l) => l.speakers.includes(want.speaker));
          if (!candidates.length) {
            errors.push(`${where}: no line "${want.speaker}: ${want.text}" in the game data`);
            continue;
          }
          row = candidates[0];
          warnings.push(`${where}: line without an id, matches ${candidates.map((l) => l.id).join(', ')}`);
        }
        found.push(row);
        if (!row.text || /^\[DNT/.test(row.text)) errors.push(`${where}: ${row.id} has no usable text (${JSON.stringify(row.text)})`);
        if (row.text !== want.text) {
          errors.push(`${where}: ${row.id} text changed\n    was: ${JSON.stringify(want.text)}\n    now: ${JSON.stringify(row.text)}`);
        }
        if (!row.speakers.includes(want.speaker)) {
          const short = row.shortNames.includes(want.speaker) ? ' (that is its ShortName; the log shows Name)' : '';
          errors.push(`${where}: ${row.id} speaker is ${row.speakers.join(' / ') || 'nobody'}, the trigger says ${want.speaker}${short}`);
        }
      }
      // Variants the trigger lacks: same stem, same speaker. Only NPCTextAudio ids carry
      // variant suffixes; in NPCTalkDialogueTextAudio a number is the line's place in a talk.
      const have = new Set(found.map((l) => l.id));
      for (const row of found.filter((l) => l.table === 'NPCTextAudio')) {
        for (const sibling of byStem.get(variantStem(row.id)) ?? []) {
          if (have.has(sibling.id) || !sibling.text || /^\[DNT/.test(sibling.text)) continue;
          if (!sibling.speakers.some((s) => row.speakers.includes(s))) continue;
          have.add(sibling.id);
          warnings.push(`${where}: variant ${sibling.id} is not in the trigger: ${JSON.stringify(sibling.text)}`);
        }
      }
      // The same line said another way: the trigger may tick at another time or not at all.
      const speakers = new Set(triggerLines(trigger).map((l) => l.speaker));
      for (const row of found) {
        const others = row.speakers.filter((s) => !speakers.has(s));
        if (others.length) warnings.push(`${where}: ${row.id} is also spoken by ${others.join(' / ')}`);
        // Another row with this text only matters when the log could name the same speaker:
        // the tracker matches speaker and text ("As you wish." is also many vendors' line).
        for (const other of byText.get(row.text) ?? []) {
          if (have.has(other.id) || !other.speakers.some((s) => speakers.has(s))) continue;
          have.add(other.id);
          warnings.push(`${where}: the text of ${row.id} is also ${other.id} (${other.speakers.join(' / ')})`);
        }
        const kinds = new Set(row.talks.map((t) => `${t.npc}|${t.flags}|${t.areas}`));
        if (kinds.size > 1) {
          const list = row.talks.map((t) => `${t.npc} '${t.option}' (flags ${t.flags.replace(/\/+$/, '') || 'none'}${t.areas ? `, ${t.areas}` : ''})`);
          warnings.push(`${where}: ${row.id} is said by ${row.talks.length} NPCTalk options that differ: ${list.join('; ')}`);
        }
      }
      const kinds = found.map((l) => l.kind).filter(Boolean);
      const kind = step.evidenceKind;
      if (kinds.length && kind && kind !== 'seen') {
        const expected = kinds.includes('bark') ? 'bark' : 'panel';
        if (kind !== expected) warnings.push(`${where}: evidenceKind ${kind}, the tables say ${expected}`);
      }
    }
  }
  return { triggers, errors, warnings };
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const args = process.argv.slice(2);
  const option = (name, fallback) => {
    const at = args.indexOf(name);
    if (at < 0) return fallback;
    const [, value] = args.splice(at, 2);
    return value;
  };
  const json = args.includes('--json');
  const routesDir = option('--routes', new URL('../data/routes/', import.meta.url).pathname);
  const tables = args.filter((a) => a !== '--json')[0];
  if (!tables) {
    console.error('usage: node scripts/check-triggers.mjs <tables> [--routes <dir>] [--json]');
    process.exit(2);
  }
  const result = check(loadRoutes(routesDir), loadLines(tables));
  if (json) console.log(JSON.stringify(result, null, 2));
  else {
    for (const w of result.warnings) console.log(`warning: ${w}`);
    for (const e of result.errors) console.log(`error: ${e}`);
    console.log(`${result.triggers} npcLine triggers, ${result.errors.length} errors, ${result.warnings.length} warnings`);
  }
  process.exit(result.errors.length ? 1 : 0);
}
