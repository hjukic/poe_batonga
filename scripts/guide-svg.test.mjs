// The zone schematics follow the checklist (src/guide-svg.ts, data/guides/act1-drawings.json):
// every step tag names a row of its zone, the rows to do now are lit, hidden rows' places are
// left out, labels are at least 11 px, bosses have one colour.
import { readFile, readdir } from 'node:fs/promises';
import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import ts from 'typescript';

const load = async (path) => {
  const source = await readFile(new URL(path, import.meta.url), 'utf8');
  const { outputText } = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } });
  return import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);
};
const { guideSvg, strengthOf, drawable, tagsOf, BOSS_COLOR } = await load('../src/guide-svg.ts');
const drawings = JSON.parse(await readFile(new URL('../data/guides/act1-drawings.json', import.meta.url), 'utf8')).zones;
const css = await readFile(new URL('../src/style.css', import.meta.url), 'utf8');

// The Act 1 rows of contract 3 (lvl/plan.md, 2026-11-13), until package R's route files carry them.
const CONTRACT3 = {
  G1_1: ['talk-wounded-man', 'kill-bloated-miller', 'enter-clearfell-encampment'],
  G1_town: ['talk-renly', 'enter-clearfell', 'renly-support-gem', 'renly-runed-spikes', 'town-to-grelwood', 'town-una-roots',
    'town-to-grim-tangle', 'finn-charm', 'town-to-hunting-grounds', 'deliver-unas-lute', 'leitis-gem', 'town-to-ogham-village',
    'town-beasts-trail', 'enter-vastiri-outskirts'],
  G1_2: ['abandoned-stash', 'kill-beira', 'enter-mud-burrow', 'enter-grelwood'],
  G1_3: ['kill-devourer', 'mud-burrow-back'],
  G1_4: ['grelwood-talk-una', 'grelwood-waypoint', 'kill-areagne', 'grelwood-grim-tangle-wp', 'kill-gerung', 'enter-red-vale',
    'runic-seals', 'grelwood-una-seals', 'grelwood-portal-town'],
  G1_5: ['obelisks-of-rust', 'kill-rust-king', 'red-vale-portal-town'],
  G1_6: ['grim-tangle-waypoint', 'kill-ervig', 'enter-cemetery'],
  G1_7: ['talk-lachlann', 'ancient-ruins', 'enter-mausoleum', 'enter-tomb', 'memorial-gate', 'kill-lachlann', 'enter-hunting-grounds'],
  G1_8: ['kill-draven', 'forgotten-riches', 'mausoleum-back'],
  G1_9: ['haunted-treasure', 'kill-asinia', 'tomb-back'],
  G1_10: ['kill-root-hollow-druid'],
  G1_11: ['hunting-grounds-waypoint', 'talk-delwyn', 'kill-crowbell', 'small-ritual', 'hunting-grounds-to-freythorn', 'enter-ogham-farmlands'],
  G1_12: ['ritual-altars', 'kill-king-in-the-mists', 'freythorn-portal-town', 'ominous-altars'],
  G1_13_1: ['unas-lute', 'crop-circle', 'enter-ogham-village'],
  G1_13_2: ['smithing-tools', 'blacksmiths-chest', 'kill-executioner', 'lever-leitis', 'village-portal-town', 'enter-manor-ramparts'],
  G1_14: ['gallows-secret', 'enter-ogham-manor'],
  G1_15: ['kill-candlemass', 'kill-geonor', 'manor-portal-town'],
};
const elements = (drawing) => [...drawing.marks, ...(drawing.route ?? []), ...(drawing.alt ?? []), ...(drawing.clues ?? [])];
const tagsIn = (zone) => [...new Set(elements(drawings[zone]).flatMap(tagsOf))];

test('every step tag of a drawing is a row of its zone in contract 3', () => {
  for (const zone of Object.keys(drawings)) {
    assert.ok(CONTRACT3[zone], `${zone}: no contract 3 rows`);
    for (const id of tagsIn(zone)) assert.ok(CONTRACT3[zone].includes(id), `${zone}: "${id}" is not a contract 3 row`);
  }
});

const routes = {};
for (const file of await readdir(new URL('../data/routes/', import.meta.url))) {
  const route = JSON.parse(await readFile(new URL(`../data/routes/${file}`, import.meta.url), 'utf8'));
  routes[route.zone.id] = new Set(route.steps.map((step) => step.id));
}
// Runs once package R's Act 1 rows are in data/routes (the assembled candidate).
const routeHasContract3 = routes.G1_6?.has('grim-tangle-waypoint') && routes.G1_13_2?.has('village-portal-town');
test('every step tag is a step id of its zone\'s route file', { skip: routeHasContract3 ? false : 'the route files do not carry contract 3 yet (package R)' }, () => {
  for (const zone of Object.keys(drawings)) {
    for (const id of tagsIn(zone)) assert.ok(routes[zone]?.has(id), `${zone}: "${id}" is not in its route file`);
  }
});

test('every place a row stands for is tagged; the changed routes are drawn per place', () => {
  for (const [zone, drawing] of Object.entries(drawings)) {
    for (const mark of drawing.marks) {
      if (['exit', 'boss'].includes(mark.kind) && mark.label !== 'Brambleghast') assert.ok(tagsOf(mark).length, `${zone}: ${mark.label} has no step`);
    }
    for (const line of drawing.route ?? []) assert.ok(tagsOf(line).length, `${zone}: a route line has no step`);
  }
  assert.deepEqual(tagsIn('G1_4').sort(), ['enter-red-vale', 'grelwood-grim-tangle-wp', 'grelwood-talk-una', 'grelwood-una-seals', 'grelwood-waypoint', 'kill-areagne', 'kill-gerung', 'runic-seals']);
  assert.ok(tagsIn('G1_11').includes('enter-ogham-farmlands') && tagsIn('G1_11').includes('kill-crowbell'));
  assert.ok(tagsIn('G1_7').includes('enter-hunting-grounds') && tagsIn('G1_7').includes('ancient-ruins'));
  const all = JSON.stringify(drawings);
  assert.ok(!all.includes('Rotten Druid') && drawings.G1_6.marks.some((mark) => mark.label === 'Ervig'), 'the Grim Tangle\'s boss is Ervig, as the row');
});

const state = (shown, lit) => ({ shown: (id) => shown.includes(id), lit: (id) => lit.includes(id) });
test('a place is lit for a row to do now, dim otherwise, left out when all its rows are hidden', () => {
  const s = state(['a', 'b'], ['a']);
  assert.equal(strengthOf({ step: 'a' }, s), 'lit');
  assert.equal(strengthOf({ steps: ['b', 'a'] }, s), 'lit');
  assert.equal(strengthOf({ step: 'b' }, s), 'dim');
  assert.equal(strengthOf({ steps: ['b', 'c'] }, s), 'dim', 'one row listed is enough to draw it');
  assert.equal(strengthOf({ step: 'c' }, s), 'hidden');
  assert.equal(strengthOf({}, s), 'dim', 'untagged context is dim');
  assert.equal(strengthOf({ step: 'c' }), 'lit', 'without a state the whole pattern is drawn');
  assert.equal(drawable({ marks: [{ at: [0, 0], kind: 'boss', label: 'x', step: 'c' }, { at: [0, 0], kind: 'in', label: 'in' }] }, s), false);
  assert.equal(drawable({ marks: [{ at: [0, 0], kind: 'boss', label: 'x', step: 'b' }] }, s), false, 'all dim: no drawing');
  assert.equal(drawable({ marks: [{ at: [0, 0], kind: 'boss', label: 'x', step: 'a' }] }, s), true);
  assert.equal(drawable({ marks: [], route: [{ points: [[0, 0], [1, 1]], step: 'a' }] }, s), true, 'a lit way is enough');
});

test('the Clearfell drawing: optional content off leaves out the Mud Burrow, the row to do now is lit', () => {
  const listed = CONTRACT3.G1_2.filter((id) => id !== 'enter-mud-burrow');
  const svg = guideSvg(drawings.G1_2, state(listed, ['abandoned-stash', 'kill-beira']));
  assert.ok(!svg.includes('Mud Burrow'), 'the hidden row\'s place is not drawn');
  assert.match(svg, /<text[^>]*>Campsite<\/text>/);
  assert.match(svg, /<g opacity="0\.6"><text[^>]*>The Grelwood<\/text><\/g>/, 'a later row\'s place is dimmed, label kept');
  assert.doesNotMatch(svg, /<g opacity="[0-9.]+"><text[^>]*>Beira<\/text>/, 'the row to do now is not dimmed');
  const glow = (svg.match(/filter="url\(#g-glow\)"/g) ?? []).length;
  assert.equal(glow, 2, 'only the ways to the stash and Beira glow');
  assert.ok(guideSvg(drawings.G1_2, state(CONTRACT3.G1_2, [])).includes('Mud Burrow'), 'optional content on draws it');
});

test('labels are at least 11 px on screen; bosses have one colour in the rows and the drawing', () => {
  const width = Number(css.match(/\.guide-drawing svg \{[^}]*width: (\d+(?:\.\d+)?)px/)[1]);
  for (const name of ['g-label', 'g-clue']) {
    const size = Number(css.match(new RegExp(`\\.guide-svg \\.${name} \\{ font: [^;]*?(\\d+(?:\\.\\d+)?)px`))[1]);
    assert.ok((size * width) / 100 >= 11, `${name}: ${((size * width) / 100).toFixed(1)} px`);
  }
  assert.equal(css.match(/\n\.boss \{ color: (#[0-9a-f]{6}); \}/)[1], BOSS_COLOR);
});

test('text holds over bright ground at 25%: a tight dark halo on the panel, dim labels not too faint', () => {
  assert.match(css, /--halo: 0 0 1px #000, 0 0 2px #000d,/);
  assert.match(css, /\.panel \{[^}]*text-shadow: var\(--halo\)/);
  assert.match(css, /\.panel \{[^}]*background: rgb\(13 12 18 \/ var\(--panel-opacity\)\)/, 'the panel stays translucent');
  const svg = guideSvg(drawings.G1_2, state(CONTRACT3.G1_2, ['kill-beira']));
  assert.match(svg, /<g opacity="0\.6"><text[^>]*>Campsite<\/text>/);
});

// A visit's view: its rows listed now (all lit: the arrow row and the click-only rows before it).
const visitState = (zone, rows, lit = rows) => ({
  shown: (id) => CONTRACT3[zone].includes(id), current: (id) => rows.includes(id), lit: (id) => lit.includes(id),
});
const glows = (svg) => (svg.match(/filter="url\(#g-glow\)"/g) ?? []).length;
const ways = (svg) => (svg.match(/<polyline[^>]*stroke="#ffd27a"/g) ?? []).length;
const litLabel = (svg, text) => new RegExp(`(^|[^"])<text[^>]*>${text}</text>`).test(svg.replace(/<g opacity="[0-9.]+"><text[^>]*>[^<]*<\/text><\/g>/g, ''));
test('a return visit by waypoint or portal draws no first entrance and no way from it: only its target', () => {
  const returns = [
    ['G1_4', ['runic-seals', 'grelwood-una-seals', 'grelwood-portal-town'], 'From Clearfell', 'Tree of Souls'],
    ['G1_11', ['enter-ogham-farmlands'], 'From the Cemetery', 'Ogham Farmlands'],
    ['G1_13_2', ['enter-manor-ramparts'], 'From the Farmlands', 'Manor Ramparts'],
    ['G1_6', ['kill-ervig', 'enter-cemetery'], 'From the Grelwood', 'Ervig'],
  ];
  for (const [zone, rows, entrance, target] of returns) {
    const svg = guideSvg(drawings[zone], visitState(zone, rows));
    assert.ok(!svg.includes(entrance), `${zone}: no "${entrance}" on the return`);
    assert.equal(ways(svg), 0, `${zone}: no way from an entrance the player did not use`);
    assert.equal(glows(svg), 0, `${zone}: nothing glows as a path`);
    assert.ok(litLabel(svg, target), `${zone}: ${target} is lit`);
  }
  const first = guideSvg(drawings.G1_4, visitState('G1_4', CONTRACT3.G1_4.slice(0, 6), ['grelwood-talk-una', 'grelwood-waypoint', 'kill-areagne']));
  assert.ok(first.includes('From Clearfell') && glows(first) === 2, 'the first visit keeps its entrance and lit ways');
  const village = guideSvg(drawings.G1_13_2, visitState('G1_13_2', ['smithing-tools', 'blacksmiths-chest', 'kill-executioner', 'lever-leitis', 'village-portal-town']));
  assert.ok(village.includes('From the Farmlands') && glows(village) === 2);
});

test('the Cemetery\'s side exits swap by layout: one neutral label on both, both lit for either row', () => {
  const sides = drawings.G1_7.marks.filter((mark) => tagsOf(mark).includes('enter-mausoleum'));
  assert.equal(sides.length, 2);
  assert.equal(sides[0].label, sides[1].label, 'neither side is claimed for one of them');
  assert.deepEqual(tagsOf(sides[0]).sort(), ['enter-mausoleum', 'enter-tomb']);
  assert.deepEqual(tagsOf(sides[1]).sort(), ['enter-mausoleum', 'enter-tomb']);
  const rows = CONTRACT3.G1_7;
  for (const lit of [['talk-lachlann', 'ancient-ruins', 'enter-mausoleum'], ['enter-tomb']]) {
    const svg = guideSvg(drawings.G1_7, visitState('G1_7', rows, lit));
    const bright = svg.replace(/<g opacity="[0-9.]+">.*?<\/g>/g, '');
    assert.equal((bright.match(new RegExp(`>${sides[0].label}</text>`, 'g')) ?? []).length, 2, `both sides lit for ${lit.at(-1)}`);
  }
});

test('no schematic when nothing in it is lit for the rows on show', () => {
  const riverbank = CONTRACT3.G1_1;
  assert.equal(drawable(drawings.G1_1, visitState('G1_1', riverbank, ['talk-wounded-man'])), false, 'a talk at the start: no place');
  assert.equal(drawable(drawings.G1_1, visitState('G1_1', riverbank, ['kill-bloated-miller'])), true);
  assert.equal(drawable(drawings.G1_6, visitState('G1_6', ['grim-tangle-waypoint'])), false, 'the waypoint grab: no place known');
  assert.equal(drawable(drawings.G1_6, visitState('G1_6', ['kill-ervig', 'enter-cemetery'], ['kill-ervig'])), true);
  assert.equal(drawable(drawings.G1_4, visitState('G1_4', ['runic-seals', 'grelwood-una-seals', 'grelwood-portal-town'])), true);
  assert.equal(drawable(drawings.G1_2), true, 'without a state the whole pattern is drawn');
});
