import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { questStates } from './import-quest-states.mjs';
import { setScreen } from './set-screen-conditions.mjs';

test('quest states keep the tracker lines, campaign quests only', () => {
  const quests = [
    { Id: 'TheMiller', Act: 1, Name: 'Reaching Clearfell' },
    { Id: 'GoToAct2', Act: 1, Name: '[DNT-UNUSED] The Mysterious Boat' },
    { Id: 'EndgameMapBoss', Act: 10, Name: 'Powerful Foe' },
  ];
  const states = [
    { Quest: { Id: 'TheMiller' }, Order: 4, MapPinsText: 'Enter the logging encampment', Message: 'Enter town' },
    { Quest: { Id: 'TheMiller' }, Order: 0, MapPinsText: '', Message: 'Quest Complete', unknown_0: true },
    { Quest: { Id: 'TheMiller' }, Order: 3, MapPinsText: 'Open the Passive Skill Screen\r\nSpend your Passive Skill Point' },
    { Quest: { Id: 'EndgameMapBoss' }, Order: 1, MapPinsText: 'Defeat a Map Boss' },
  ];
  assert.deepEqual(questStates(quests, states), {
    TheMiller: {
      name: 'Reaching Clearfell',
      act: 1,
      states: [
        { order: 0, lines: ['Quest Complete'], complete: true },
        { order: 3, lines: ['Open the Passive Skill Screen', 'Spend your Passive Skill Point'] },
        { order: 4, lines: ['Enter the logging encampment'] },
      ],
    },
  });
});

test('screen conditions go after the kind line and keep the file as it was', () => {
  const route = '{\r\n  "steps": [\r\n    {\r\n      "id": "a",\r\n      "kind": "objective",\r\n      "parts": []\r\n    }\r\n  ]\r\n}';
  const set = setScreen(route, 'a', [{ quest: 'Q', past: 'Line' }]);
  assert.equal(set.split('\r\n')[5], '      "screen": [{ "quest": "Q", "past": "Line" }],');
  assert.deepEqual(JSON.parse(set).steps[0].screen, [{ quest: 'Q', past: 'Line' }]);
  // Replaced, not added twice; an empty list removes it.
  const again = setScreen(set, 'a', [{ quest: 'Q', lines: ['x', 'y'] }, { quest: 'R' }]);
  assert.deepEqual(JSON.parse(again).steps[0].screen, [{ quest: 'Q', lines: ['x', 'y'] }, { quest: 'R' }]);
  assert.equal(setScreen(again, 'a', []), route);
  assert.equal(setScreen(route, 'missing', []), null);
});

test('every route screen condition names a quest in data/quests/states.json', () => {
  const { quests } = JSON.parse(readFileSync(new URL('../data/quests/states.json', import.meta.url), 'utf8'));
  assert.ok(quests.TheMiller.states.some((s) => s.lines.includes('Enter the logging encampment')));
});
