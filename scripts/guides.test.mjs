// The Act 1 guide text (data/guides/act1.json) against the route files and the text rules
// (docs/act1-zone-tips.md). `problems` is the whole check; the tests below run it on the real
// files and on planted violations, one per rule, so a rule cannot go quiet unnoticed.
import { readFile, readdir } from 'node:fs/promises';
import { strict as assert } from 'node:assert';
import { test } from 'node:test';

const root = new URL('../', import.meta.url);
const guide = JSON.parse(await readFile(new URL('data/guides/act1.json', root), 'utf8'));
const routes = {};
for (const name of await readdir(new URL('data/routes/', root))) {
  if (!name.endsWith('.json')) continue;
  const route = JSON.parse(await readFile(new URL(`data/routes/${name}`, root), 'utf8'));
  routes[route.zone.id] = route;
}

const TOWN = 'G1_town';
const KINDS = new Set(['way', 'boss', 'level', 'tip']);
// What fits on one line of the panel at the hints' size (measured on the renders).
const MAX = 46;
// "2 of 3", "2/3" and "two of three" alike.
const NUMBER = '(?:\\d+|one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve)';
const COUNT = new RegExp(`\\b${NUMBER}\\s*(?:of|/)\\s*${NUMBER}\\b`, 'i');

/** Every hint of the file as [zone, where, hint]; `where` is a step id or "general". */
export function hintsOf(file) {
  const out = [];
  for (const [zone, g] of Object.entries(file.zones ?? {})) {
    for (const [step, h] of Object.entries(g.hints ?? {})) for (const one of [h].flat()) out.push([zone, step, one]);
    for (const one of g.general ?? []) out.push([zone, 'general', one]);
  }
  return out;
}

/** Text rules: what a line may say, checked mechanically. Judgment (one thing, a second of
 *  reading, build-neutral) is the author's; see docs/act1-zone-tips.md. */
export function textProblems(file) {
  const out = [];
  for (const [zone, where, h] of hintsOf(file)) {
    const at = `${zone}/${where}: "${h.text}"`;
    const kind = h.kind ?? 'way';
    if (typeof h.text !== 'string' || !h.text.trim()) { out.push(`${at}: no text`); continue; }
    if (/[\r\n]/.test(h.text)) out.push(`${at}: more than one line`);
    if (h.text.length > MAX) out.push(`${at}: ${h.text.length} characters, over ${MAX}`);
    if (h.text.includes('%')) out.push(`${at}: a percentage`);
    if (COUNT.test(h.text)) out.push(`${at}: an "N of M" count`);
    const errand = zone === TOWN && where === 'general';
    if (/\d/.test(h.text) && kind !== 'level' && !errand) out.push(`${at}: a number outside a level line or an errand`);
    if (!KINDS.has(kind)) out.push(`${at}: unknown kind "${kind}"`);
    if (kind === 'level') {
      if (!Number.isInteger(h.below)) out.push(`${at}: a level line without "below"`);
      else if (h.text !== `Level ${h.below} first.`) out.push(`${at}: a level line reads "Level ${h.below} first."`);
    }
    for (const key of ['below', 'from']) {
      if (key in h && !(Number.isInteger(h[key]) && h[key] > 0)) out.push(`${at}: "${key}" is not a level`);
    }
    if (!Array.isArray(h.src) || h.src.length === 0) out.push(`${at}: no source`);
    else for (const s of h.src) if (!(s in (file.sources ?? {}))) out.push(`${at}: unknown source "${s}"`);
  }
  return out;
}

/** Ids: every hint key and every after/until is a step of its zone's route file. */
export function idProblems(file, routeFiles) {
  const out = [];
  for (const [zone, g] of Object.entries(file.zones ?? {})) {
    const steps = new Set((routeFiles[zone]?.steps ?? []).map((s) => s.id));
    if (!routeFiles[zone]) out.push(`${zone}: no route file`);
    for (const step of Object.keys(g.hints ?? {})) if (!steps.has(step)) out.push(`${zone}: hint key "${step}" is not a step`);
    for (const [, where, h] of hintsOf({ zones: { [zone]: g } })) {
      for (const key of ['after', 'until']) {
        if (key in h && !steps.has(h[key])) out.push(`${zone}/${where}: ${key} "${h[key]}" is not a step`);
      }
    }
  }
  return out;
}

const clone = (x) => JSON.parse(JSON.stringify(x));
const line = (text, extra = {}) => ({ text, src: ['data'], ...extra });
const planted = (zone, where, hint) => {
  const file = clone(guide);
  file.zones[zone] ??= { hints: {} };
  if (where === 'general') (file.zones[zone].general ??= []).push(hint);
  else file.zones[zone].hints[where] = hint;
  return file;
};

test('the guide text keeps the mechanical text rules', () => {
  assert.deepEqual(textProblems(guide), []);
});

test('every hint key and every after/until is a step id of its zone', () => {
  // Fails on a branch whose route files predate contract 3's new ids; green on the candidate.
  assert.deepEqual(idProblems(guide, routes), []);
});

test('a planted violation of each text rule is caught', () => {
  const cases = [
    ['G1_2', 'kill-beira', line('Away from town,\nfar side.'), 'more than one line'],
    ['G1_2', 'kill-beira', line('x'.repeat(MAX + 1)), `over ${MAX}`],
    ['G1_2', 'kill-beira', line('Far side in most layouts (67%).'), 'a percentage'],
    ['G1_5', 'obelisks-of-rust', line('Two of three are close.'), 'an "N of M" count'],
    ['G1_2', 'enter-mud-burrow', line('To one side of the start.'), null],
    ['G1_5', 'obelisks-of-rust', line('Found 2 of 3 already.'), 'an "N of M" count'],
    ['G1_5', 'obelisks-of-rust', line('Found 2/3 already.'), 'an "N of M" count'],
    ['G1_5', 'kill-rust-king', line('Spawns at the 3rd obelisk.'), 'a number outside'],
    ['G1_1', 'general', line('Buy 2 flasks.'), 'a number outside'],
    ['G1_5', 'kill-rust-king', line('Hug him.', { kind: 'fight' }), 'unknown kind'],
    ['G1_8', 'kill-draven', line('Level 7 first.', { kind: 'level' }), 'without "below"'],
    ['G1_8', 'kill-draven', line('Be level 7 first.', { kind: 'level', below: 7 }), 'reads "Level 7 first."'],
    ['G1_8', 'kill-draven', line('Level 8 first.', { kind: 'level', below: 7 }), 'reads "Level 7 first."'],
    ['G1_town', 'general', line('Boots.', { from: 'ten' }), '"from" is not a level'],
    ['G1_town', 'general', line('Boots.', { below: 0 }), '"below" is not a level'],
    ['G1_2', 'kill-beira', { text: 'Far side.', src: [] }, 'no source'],
    ['G1_2', 'kill-beira', { text: 'Far side.', src: ['someone'] }, 'unknown source'],
    ['G1_2', 'kill-beira', { text: ' ', src: ['data'] }, 'no text'],
  ];
  for (const [zone, where, hint, expected] of cases) {
    const found = textProblems(planted(zone, where, hint)).filter((p) => p.includes(`"${hint.text}"`));
    if (expected === null) assert.deepEqual(found, [], `"${hint.text}" is allowed`);
    else assert.ok(found.some((p) => p.includes(expected)), `"${hint.text}" → ${expected}; got ${JSON.stringify(found)}`);
  }
  // Numbers are fine on level lines and town errands.
  assert.deepEqual(textProblems(planted('G1_town', 'general', line('Greater flasks at 10.', { from: 10 }))), textProblems(guide));
});

test('a planted id violation of each kind is caught', () => {
  const zones = { G1_2: { steps: [{ id: 'kill-beira' }, { id: 'enter-grelwood' }] } };
  const file = (g) => ({ sources: {}, zones: { G1_2: g } });
  assert.deepEqual(idProblems(file({ hints: { 'kill-beira': line('Far side.') } }), zones), []);
  assert.match(idProblems(file({ hints: { 'kill-biera': line('Far side.') } }), zones).join(), /hint key "kill-biera"/);
  assert.match(idProblems(file({ hints: { 'kill-beira': [line('a'), line('b', { after: 'nope' })] } }), zones).join(), /after "nope"/);
  assert.match(idProblems(file({ hints: {}, general: [line('a', { until: 'gone' })] }), zones).join(), /until "gone"/);
  assert.match(idProblems({ sources: {}, zones: { G9_9: { hints: {} } } }, zones).join(), /G9_9: no route file/);
});

// GTD-video's Act 1 level targets (the benchmark, kind 3). Level 10 ("the next weapon base in
// town") is a town errand that names its level, shown under it too, pinned with the errands.
const LEVEL_TARGETS = [
  ['G1_6', 'enter-cemetery', 5],
  ['G1_7', 'enter-mausoleum', 7],
  ['G1_11', 'hunting-grounds-to-freythorn', 8],
  ['G1_14', 'enter-ogham-manor', 13],
  ['G1_15', 'kill-geonor', 14],
];

test("every Act 1 level target is a level line under the row it gates, read first", () => {
  for (const [zone, step, level] of LEVEL_TARGETS) {
    const lines = [guide.zones[zone]?.hints?.[step] ?? []].flat();
    assert.equal(lines[0]?.kind, 'level', `${zone}/${step}: the level line comes first`);
    assert.equal(lines[0].below, level, `${zone}/${step}: below ${level}`);
  }
  const all = hintsOf(guide).filter(([, , h]) => h.kind === 'level');
  assert.equal(all.length, LEVEL_TARGETS.length, 'no level line without a source target');
});

// Contract 3's town visits (docs: lvl/plan.md), in walking order; the last id of each is its
// leaving step (a log-ticked zone entry). Errands are pinned to visits by these ids only.
const TOWN_VISITS = [
  ['talk-renly', 'enter-clearfell'],
  ['renly-support-gem', 'renly-runed-spikes', 'town-to-grelwood'],
  ['town-una-roots', 'town-to-grim-tangle'],
  ['finn-charm', 'town-to-hunting-grounds'],
  ['deliver-unas-lute', 'leitis-gem', 'town-to-ogham-village'],
  ['town-beasts-trail', 'enter-vastiri-outskirts'],
];

/** Contract 2's conditions: is the line shown at this character level (null: unknown) with these steps done? */
export function visible(h, level, done) {
  if (h.below !== undefined && level !== null && level >= h.below) return false;
  if (h.from !== undefined && (level === null || level < h.from)) return false;
  if (h.after !== undefined && !done.has(h.after)) return false;
  if (h.until !== undefined && done.has(h.until)) return false;
  return true;
}

/** The most errand lines any town visit shows, at any level, whatever of the visit is already done. */
function mostErrands(errands) {
  let most = { count: 0 };
  TOWN_VISITS.forEach((steps, v) => {
    const before = TOWN_VISITS.slice(0, v).flat();
    const inside = steps.slice(0, -1);
    for (let mask = 0; mask < 1 << inside.length; mask++) {
      const done = new Set([...before, ...inside.filter((_, i) => mask & (1 << i))]);
      for (const level of [null, ...Array.from({ length: 60 }, (_, i) => i + 1)]) {
        const shown = errands.filter((h) => visible(h, level, done));
        if (shown.length > most.count) most = { count: shown.length, visit: v + 1, level, lines: shown.map((h) => h.text) };
      }
    }
  });
  return most;
}

test('at most two errand lines in any town visit, at any level', () => {
  const errands = guide.zones.G1_town?.general ?? [];
  assert.ok(errands.length > 0, 'the town has errands');
  const most = mostErrands(errands);
  assert.ok(most.count <= 2, `visit ${most.visit} at level ${most.level}: ${JSON.stringify(most.lines)}`);
  // Each visit up to the one after Ogham Village has at least one errand at its usual level.
  // The ring for Geonor shows from level 12 on any trip after the Grim Tangle's, also when
  // the trip after Freythorn was skipped (council round 2).
  const skipped = new Set(TOWN_VISITS.slice(0, 3).flat());
  assert.ok(errands.some((h) => /ring for Geonor/.test(h.text) && visible(h, 13, skipped)), 'the ring shows though the Freythorn trip was skipped');
  for (const [v, level] of [[1, 2], [2, 6], [3, 8], [4, 11], [5, 13]]) {
    const done = new Set(TOWN_VISITS.slice(0, v - 1).flat());
    assert.ok(errands.some((h) => visible(h, level, done)), `visit ${v} at level ${level} has an errand`);
  }
  // The level-10 errands of the visit after Freythorn show under 10 and with the level unknown too.
  const fourth = new Set(TOWN_VISITS.slice(0, 3).flat());
  for (const level of [null, 9, 10]) {
    assert.ok(errands.some((h) => /Level 10/.test(h.text) && visible(h, level, fourth)), `visit 4 at level ${level} names level 10`);
  }
  // A planted third line in the second visit is caught.
  const extra = { text: 'Planted.', src: ['gtd'], after: 'enter-clearfell', until: 'town-to-grelwood' };
  assert.equal(mostErrands([...errands, extra]).count, 3);
});

test('errands are pinned by leaving steps, so a log line moves them on', () => {
  const leaving = new Set(TOWN_VISITS.map((steps) => steps.at(-1)));
  for (const h of guide.zones.G1_town?.general ?? []) {
    for (const key of ['after', 'until']) if (key in h) assert.ok(leaving.has(h[key]), `"${h.text}": ${key} ${h[key]}`);
  }
});

test("the town's visits in the route file are contract 3's", () => {
  // Fails on a branch whose route files predate contract 3; green on the candidate.
  const route = routes.G1_town;
  const visits = [[]];
  for (const s of route.steps) {
    visits.at(-1).push(s.id);
    const t = s.trigger;
    if (t?.type === 'zoneEntered' && t.area !== route.zone.id && !s.optional) visits.push([]);
  }
  assert.deepEqual(visits.filter((v) => v.length), TOWN_VISITS);
});

// Contract 3: rows dropped from the Act 1 route keep no hints; the rows that replace them do.
const DROPPED = {
  G1_town: ['town-talk-finn', 'town-una-cemetery', 'town-hooded-one', 'town-talk-all'],
  G1_4: ['grelwood-red-vale-wp'],
  G1_11: ['hunting-grounds-town', 'freythorn-wp', 'farmlands-wp'],
  G1_12: ['freythorn-to-farmlands'],
};

test("hints follow contract 3's rows", () => {
  for (const [zone, steps] of Object.entries(DROPPED)) {
    for (const step of steps) assert.equal(guide.zones[zone]?.hints?.[step], undefined, `${zone}/${step} was dropped`);
  }
  const lines = (zone, step) => [guide.zones[zone]?.hints?.[step] ?? []].flat().map((h) => h.text);
  assert.ok(lines('G1_11', 'hunting-grounds-to-freythorn').some((t) => /locusts/.test(t)));
  assert.ok(lines('G1_11', 'enter-ogham-farmlands').some((t) => /signposts/.test(t)));
  assert.ok(lines('G1_7', 'kill-lachlann').some((t) => /cutscene/.test(t)), "Lachlann's row says why the ring stays");
  assert.equal(guide.zones.G1_4.general[0].until, 'enter-red-vale', 'the "exit first" trick ends with the first visit');
  // Lines that must not show at the wrong moment (review of 2026-11-13).
  assert.ok(lines('G1_7', 'kill-lachlann').some((t) => /comes later/.test(t)), 'the ring line says the Hooded One comes later');
  assert.ok(!lines('G1_4', 'kill-areagne').some((t) => /cauldron/.test(t)), 'the cauldron line outlives the kill row');
  assert.ok(guide.zones.G1_4.general.some((h) => /cauldron/.test(h.text) && h.until === 'enter-red-vale'));
  assert.ok(!(guide.zones.G1_6.general ?? []).some((h) => /Una/.test(h.text)), "no Una line on the Grim Tangle's waypoint grab");
  assert.ok(lines('G1_6', 'enter-cemetery').some((t) => /Una/.test(t)));
});
