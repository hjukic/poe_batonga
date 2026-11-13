#!/usr/bin/env node
// Writes the quest-tracker conditions (`screen`) of route steps, from a JSON file
// { "<step id>": [ { "quest": "...", "past": "..." } | { "quest": "...", "lines": [...] } ] }.
// Each step's `screen` goes right after its "kind" line; an existing one is replaced, an
// empty list removes it. The rest of each route file keeps its formatting.
//
//   node scripts/set-screen-conditions.mjs <mapping.json>
import { readFileSync, writeFileSync, readdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = join(dirname(fileURLToPath(import.meta.url)), '..');
const dir = join(repo, 'data/routes');

/** The `"screen": [...]` lines for a step, at the step's field indent. */
export function screenLines(conditions, indent = '      ') {
  if (conditions.length === 1 && JSON.stringify(conditions[0]).length < 100) {
    return [`${indent}"screen": [${format(conditions[0])}],`];
  }
  return [`${indent}"screen": [`, ...conditions.map((c, i) => `${indent}  ${format(c)}${i + 1 < conditions.length ? ',' : ''}`), `${indent}],`];
}

function format(condition) {
  const fields = Object.entries(condition).map(([k, v]) => `${JSON.stringify(k)}: ${JSON.stringify(v).replace(/","/g, '", "')}`);
  return `{ ${fields.join(', ')} }`;
}

/** The route text with `step`'s screen lines set; null when the step is not in it. */
export function setScreen(text, step, conditions) {
  const eol = text.includes('\r\n') ? '\r\n' : '\n';
  const lines = text.split(/\r?\n/);
  const at = lines.findIndex((l) => l.trim() === `"id": ${JSON.stringify(step)},`);
  if (at < 0) return null;
  let end = at + 1;
  while (end < lines.length && !/^\s*}/.test(lines[end])) end++;
  // Drop an existing screen block inside this step.
  for (let i = at + 1; i < end; i++) {
    if (lines[i].trim().startsWith('"screen":')) {
      let j = i;
      if (!lines[i].trim().endsWith('],')) while (!lines[j].trim().startsWith('],')) j++;
      lines.splice(i, j - i + 1);
      end -= j - i + 1;
      break;
    }
  }
  if (conditions.length) {
    const kind = lines.slice(at, end).findIndex((l) => l.trim().startsWith('"kind":'));
    const after = kind >= 0 ? at + kind : at;
    const indent = lines[at].match(/^\s*/)[0];
    lines.splice(after + 1, 0, ...screenLines(conditions, indent));
  }
  return lines.join(eol);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const mapping = JSON.parse(readFileSync(process.argv[2], 'utf8'));
  const files = readdirSync(dir).filter((f) => f.endsWith('.json'));
  const texts = Object.fromEntries(files.map((f) => [f, readFileSync(join(dir, f), 'utf8')]));
  let missing = 0;
  for (const [step, conditions] of Object.entries(mapping)) {
    const file = files.find((f) => setScreen(texts[f], step, []) !== null);
    if (!file) {
      console.error(`no step ${step}`);
      missing++;
      continue;
    }
    texts[file] = setScreen(texts[file], step, conditions);
    JSON.parse(texts[file]);
  }
  for (const f of files) writeFileSync(join(dir, f), texts[f]);
  console.log(`${Object.keys(mapping).length - missing} steps set`);
  if (missing) process.exit(1);
}
