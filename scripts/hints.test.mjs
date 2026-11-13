// The guide lines' kinds and conditions (src/hints.ts): which lines show for a level and the
// zone's steps, and the level target the header marks.
import { readFile } from 'node:fs/promises';
import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import ts from 'typescript';

const source = await readFile(new URL('../src/hints.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } });
const { hintShown, shownHints, kindOf, asList, unmetLevelTarget } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);

const steps = [{ id: 'kill-draven', state: 'done' }, { id: 'kill-asinia', state: 'open' }, { id: 'memorial-gate', state: 'manual' }];
const at = (level) => ({ level, steps });
const hint = (extra) => ({ text: 'x', src: ['data'], ...extra });

test('a line without conditions always shows; its kind defaults to "way"', () => {
  assert.equal(hintShown(hint(), at(null)), true);
  assert.equal(kindOf(hint()), 'way');
  assert.equal(kindOf(hint({ kind: 'boss' })), 'boss');
  assert.equal(kindOf(hint({ kind: 'nonsense' })), 'way');
});

test('"below": only while the level is under it; an unknown level shows it', () => {
  assert.equal(hintShown(hint({ below: 7 }), at(6)), true);
  assert.equal(hintShown(hint({ below: 7 }), at(7)), false);
  assert.equal(hintShown(hint({ below: 7 }), at(9)), false);
  assert.equal(hintShown(hint({ below: 7 }), at(null)), true);
});

test('"from": only once the level is reached; an unknown level hides it', () => {
  assert.equal(hintShown(hint({ from: 10 }), at(9)), false);
  assert.equal(hintShown(hint({ from: 10 }), at(10)), true);
  assert.equal(hintShown(hint({ from: 10 }), at(null)), false);
  assert.equal(hintShown(hint({ from: 10, below: 16 }), at(12)), true);
  assert.equal(hintShown(hint({ from: 10, below: 16 }), at(16)), false);
});

test('"after" and "until" follow the zone\'s steps; an open or missing step is not done', () => {
  assert.equal(hintShown(hint({ after: 'kill-draven' }), at(5)), true);
  assert.equal(hintShown(hint({ after: 'kill-asinia' }), at(5)), false);
  assert.equal(hintShown(hint({ after: 'memorial-gate' }), at(5)), false);
  assert.equal(hintShown(hint({ after: 'no-such-step' }), at(5)), false);
  assert.equal(hintShown(hint({ until: 'kill-draven' }), at(5)), false);
  assert.equal(hintShown(hint({ until: 'kill-asinia' }), at(5)), true);
  assert.equal(hintShown(hint({ until: 'no-such-step' }), at(5)), true);
  assert.equal(hintShown(hint({ after: 'kill-draven', until: 'kill-asinia', below: 8 }), at(7)), true);
  assert.equal(hintShown(hint({ after: 'kill-draven', until: 'kill-asinia', below: 8 }), at(8)), false);
});

test('a row may have one line or several; the hidden ones are left out, in order', () => {
  assert.deepEqual(asList(undefined), []);
  assert.deepEqual(asList(hint({ text: 'one' })).map((h) => h.text), ['one']);
  const lines = [hint({ text: 'way' }), hint({ text: 'level', kind: 'level', below: 7 }), hint({ text: 'boss', kind: 'boss' })];
  assert.deepEqual(shownHints(lines, at(5)).map((h) => h.text), ['way', 'level', 'boss']);
  assert.deepEqual(shownHints(lines, at(7)).map((h) => h.text), ['way', 'boss']);
});

test('the header marks the level only while a level line on show is unmet', () => {
  const rows = [hint({ kind: 'level', below: 7 }), [hint(), hint({ kind: 'level', below: 8, after: 'kill-draven' })]];
  assert.equal(unmetLevelTarget(rows, [], at(6)), 8);
  assert.equal(unmetLevelTarget(rows, [], at(7)), 8);
  assert.equal(unmetLevelTarget(rows, [], at(8)), null);
  assert.equal(unmetLevelTarget(rows, [], at(null)), null, 'an unknown level is not marked');
  assert.equal(unmetLevelTarget([], [hint({ kind: 'level', below: 5 })], at(4)), 5, 'the zone\'s own lines count');
  assert.equal(unmetLevelTarget([hint({ below: 9 })], [], at(4)), null, 'only level lines are targets');
  assert.equal(unmetLevelTarget([hint({ kind: 'level', below: 9, after: 'kill-asinia' })], [], at(4)), null, 'a hidden line is no target');
});
