#!/usr/bin/env node
// Writes data/quests/states.json: the campaign quests and the lines the game's quest
// tracker shows for each of their states, from the game's own tables (read-only).
//
//   poe_data_tools --patch 2 --steam "<game folder>" dump-tables --mode json <dump> \
//     data/balance/quest.datc64 data/balance/queststates.datc64
//   node scripts/import-quest-states.mjs <dump>
//
// The tracker (right of the screen, under the minimap) shows each active quest's
// name and the `MapPinsText` of its current state, one line per `\n` (measured
// 2026-11-13: The Miller's state 4 shows "Enter the logging encampment"). The game
// shows the first state, by `Order`, whose flags hold, and a quest's flags only grow,
// so a lower `Order` is further on in the quest. See docs/quest-tracker.md.
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = join(dirname(fileURLToPath(import.meta.url)), '..');

/** Campaign quests only: acts 1–6 (6 is the interlude), named, not unused. */
export function campaignQuests(quests) {
  return quests.filter((q) => q.Act >= 1 && q.Act <= 6 && q.Name && !q.Name.startsWith('[DNT'));
}

/** { id: { name, act, states: [{ order, lines, complete? }] } }, states by order. */
export function questStates(quests, states) {
  const out = {};
  for (const quest of campaignQuests(quests)) {
    const rows = states
      .filter((s) => s.Quest?.Id === quest.Id)
      .sort((a, b) => a.Order - b.Order)
      .map((s) => {
        // A completed quest has no MapPinsText; its Message ("Quest Complete") is kept in
        // case the tracker shows it for a moment before the quest leaves the list.
        const text = s.MapPinsText || (s.unknown_0 ? s.Message : '') || '';
        const lines = String(text)
          .split('\n')
          .map((l) => l.replace(/\s+/g, ' ').trim())
          .filter(Boolean);
        return { order: s.Order, lines, ...(s.unknown_0 ? { complete: true } : {}) };
      });
    if (rows.length) out[quest.Id] = { name: quest.Name, act: quest.Act, states: rows };
  }
  return out;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const dump = process.argv[2];
  if (!dump) {
    console.error('usage: node scripts/import-quest-states.mjs <dump-tables folder>');
    process.exit(2);
  }
  const table = (name) => JSON.parse(readFileSync(join(dump, 'data/balance', `${name}.datc64.json`), 'utf8'));
  const quests = questStates(table('quest'), table('queststates'));
  const file = join(repo, 'data/quests/states.json');
  mkdirSync(dirname(file), { recursive: true });
  const body = Object.entries(quests)
    .map(([id, q]) => {
      const states = q.states.map((s) => `      ${JSON.stringify(s)}`).join(',\n');
      return `    ${JSON.stringify(id)}: {\n      "name": ${JSON.stringify(q.name)}, "act": ${q.act},\n      "states": [\n  ${states.replace(/\n {6}/g, '\n        ')}\n      ]\n    }`;
    })
    .join(',\n');
  writeFileSync(file, `{\n  "formatVersion": 1,\n  "quests": {\n${body}\n  }\n}\n`);
  console.log(`${Object.keys(quests).length} quests -> ${file}`);
}
