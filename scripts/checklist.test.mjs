import { readFile } from 'node:fs/promises';
import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import ts from 'typescript';

const source = await readFile(new URL('../src/checklist.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } });
const { panelMode } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);
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
