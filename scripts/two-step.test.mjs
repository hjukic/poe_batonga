// "Reset progress" asks twice (src/two-step.ts): one click or a double click never resets; a
// deliberate second click does; waiting or closing the menu disarms.
import { readFile } from 'node:fs/promises';
import { strict as assert } from 'node:assert';
import { test, mock } from 'node:test';
import ts from 'typescript';

const source = await readFile(new URL('../src/two-step.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } });
const { twoStep, confirmedClick } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);

/** The reset button as main.ts wires it: its handler, the resets it sent, its label. */
function resetButton() {
  const button = { resets: 0, label: 'Reset progress' };
  const step = twoStep(4000, (armed) => { button.label = armed ? 'Click again to reset' : 'Reset progress'; });
  button.click = confirmedClick(step, () => { button.resets += 1; });
  button.closeMenu = () => step.disarm();
  return button;
}

test('one click arms; a deliberate second click resets', () => {
  mock.timers.enable({ apis: ['setTimeout', 'Date'] });
  try {
    const button = resetButton();
    button.click();
    assert.equal(button.resets, 0, 'one click does not reset');
    assert.equal(button.label, 'Click again to reset');
    mock.timers.tick(1500);
    button.click();
    assert.equal(button.resets, 1);
    assert.equal(button.label, 'Reset progress');
  } finally {
    mock.timers.reset();
  }
});

test('a double click (and a triple one) never resets; the button stays armed for a real second click', () => {
  mock.timers.enable({ apis: ['setTimeout', 'Date'] });
  try {
    const button = resetButton();
    button.click();
    mock.timers.tick(150);
    button.click();
    mock.timers.tick(150);
    button.click();
    assert.equal(button.resets, 0);
    assert.equal(button.label, 'Click again to reset');
    mock.timers.tick(1000);
    button.click();
    assert.equal(button.resets, 1);
  } finally {
    mock.timers.reset();
  }
});

test('waiting 4 s or closing the menu disarms: the next click arms again', () => {
  mock.timers.enable({ apis: ['setTimeout', 'Date'] });
  try {
    const button = resetButton();
    button.click();
    mock.timers.tick(3999);
    assert.equal(button.label, 'Click again to reset');
    mock.timers.tick(1);
    assert.equal(button.label, 'Reset progress');
    button.click();
    assert.equal(button.resets, 0, 'after the timeout a click arms, it does not reset');
    button.closeMenu();
    assert.equal(button.label, 'Reset progress');
    mock.timers.tick(1000);
    button.click();
    assert.equal(button.resets, 0, 'after the menu closed a click arms, it does not reset');
  } finally {
    mock.timers.reset();
  }
});
