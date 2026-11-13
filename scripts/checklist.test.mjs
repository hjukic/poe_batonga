import { readFile } from 'node:fs/promises';
import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import ts from 'typescript';

const source = await readFile(new URL('../src/checklist.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } });
const { panelMode, headerLevels, headingStep, litSteps } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);
const task = (state, optional = false) => ({ kind: 'objective', state, optional });
const view = (steps) => ({ inZone: true, character: 'First', zone: { id: 'G1_1' }, steps });

test('saved zone and completed ticks never display outside live gameplay', () => {
  assert.equal(panelMode({ ...view([task('done')]), inZone: false }, false), 'idle');
});
test('completion requires every selected objective, including later visits', () => {
  assert.equal(panelMode(view([task('done'), task('open')]), false), 'checklist');
  assert.equal(panelMode(view([task('done'), task('done'), { kind: 'note', state: 'note' }]), false), 'completed');
});
test('optional setting determines whether unfinished optional objectives keep the checklist', () => {
  assert.equal(panelMode(view([task('done'), task('open', true)]), false), 'completed');
  assert.equal(panelMode(view([task('done'), task('open', true)]), true), 'checklist');
});
test('unknown character, unknown route and empty lists are not completed quests', () => {
  assert.equal(panelMode({ ...view([task('done')]), character: null }, false), 'choose-character');
  assert.equal(panelMode({ ...view([task('done')]), zone: null }, false), 'unknown-zone');
  assert.equal(panelMode(view([]), false), 'checklist');
});

test('the header names both levels; the character\'s is marked only under a target', () => {
  assert.deepEqual(headerLevels(4, 6, null), { area: 'area 4', you: 'you 6', under: false });
  assert.deepEqual(headerLevels(7, 6, 7), { area: 'area 7', you: 'you 6', under: true });
  assert.deepEqual(headerLevels(7, 7, 7), { area: 'area 7', you: 'you 7', under: false });
  assert.deepEqual(headerLevels(7, null, 7), { area: 'area 7', you: null, under: false }, 'unknown level: area only');
});
test('in a town the header shows only the character\'s level', () => {
  assert.deepEqual(headerLevels(15, 1, null, true), { area: null, you: 'you 1', under: false });
  assert.deepEqual(headerLevels(15, null, null, true), { area: null, you: null, under: false });
});

const row = (id, state = 'open', extra = {}) => ({ id, kind: 'objective', state, optional: false, ...extra });
test('the arrow goes to the first open required row the app ticks by itself', () => {
  const grelwood = [
    row('grelwood-talk-una', 'open', { tracked: false }),
    row('grelwood-waypoint', 'manual', { tracked: false }),
    row('kill-areagne', 'open', { tracked: true }),
    row('grelwood-grim-tangle-wp', 'open', { tracked: true }),
  ];
  assert.equal(headingStep(grelwood), 'kill-areagne', 'click-only rows do not hold the arrow');
  assert.equal(headingStep([row('a', 'open', { optional: true }), row('b')]), 'b', 'optional rows are passed over');
  assert.equal(headingStep([{ kind: 'note', id: 'n', state: 'note', optional: false }, row('b')]), 'b', 'notes are not steps');
  assert.equal(headingStep([row('a', 'open', { tracked: undefined })]), 'a', 'a view without "tracked" counts as tracked');
});
test('a "probably done" boss is passed over; the arrow falls back when nothing tracked is open', () => {
  const likely = [row('kill-areagne', 'done', { source: 'likely', tracked: true }), row('enter-red-vale', 'open', { tracked: true })];
  assert.equal(headingStep(likely), 'enter-red-vale');
  assert.equal(headingStep([row('a', 'open', { tracked: false, optional: true }), row('b', 'open', { tracked: false })]), 'b', 'then the first open required row');
  assert.equal(headingStep([row('a', 'done'), row('b', 'open', { optional: true, tracked: false })]), 'b', 'then the first open row');
  assert.equal(headingStep([row('a', 'done')]), null);
  assert.equal(headingStep([]), null);
});
test('the parked compass still wins when it points at an open row', () => {
  const rows = [row('a', 'open', { tracked: true, target: 'A' }), row('b', 'open', { tracked: true, target: 'B' })];
  assert.equal(headingStep(rows, 'B'), 'b');
  assert.equal(headingStep(rows, 'Z'), 'a');
});

test('the schematic lights the arrow row and the open rows the arrow passed over', () => {
  const rows = [row('una', 'done', { tracked: false }), row('waypoint', 'open', { tracked: false }), row('areagne', 'open', { tracked: true }), row('red-vale', 'open', { tracked: true })];
  assert.deepEqual(litSteps(rows, 'areagne'), ['waypoint', 'areagne']);
  assert.deepEqual(litSteps(rows, 'red-vale'), ['waypoint', 'areagne', 'red-vale']);
  assert.deepEqual(litSteps(rows, null), []);
  assert.deepEqual(litSteps(rows, 'missing'), []);
});

test('the browser preview shows a real tracker view: the Grelwood\'s rows as the route has them, with level and tracked', async () => {
  const main = await readFile(new URL('../src/main.ts', import.meta.url), 'utf8');
  const preview = main.slice(main.indexOf('const browserPreview: TrackerView = {'));
  const steps = [...preview.slice(0, preview.indexOf('\n};')).matchAll(/^ {4}(\{"id".*\}),$/gm)].map((m) => JSON.parse(m[1]));
  const route = JSON.parse(await readFile(new URL('../data/routes/grelwood.json', import.meta.url), 'utf8'));
  assert.deepEqual(steps.map((step) => step.id), route.steps.map((step) => step.id));
  assert.ok(steps.every((step) => typeof step.tracked === 'boolean'));
  assert.match(preview, /^ {2}level: \d+,$/m);
});
