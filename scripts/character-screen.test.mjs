import { test } from 'node:test';
import { strict as assert } from 'node:assert';
import { classifyScreen, recognizeScreen, nameKey, learnKnownCharacters } from './read-character-screen.mjs';

// Positions from the owner's screenshots; names below are synthetic.
const line = (text, x, y) => ({ text, words: [{ text, x, y, width: 100, height: 20 }] });
const image = (lines) => ({ width: 2559, height: 1439, lines });
const selection = image([
  line('Existing_Hero', 2036, 326), line('LEVEL 1 DRUID', 2185, 363),
  line('Forbidden Rites League', 2186, 429), line('CREATE', 2105, 1082),
  line('DELETE', 2382, 1082), line('PLAY', 942, 1281),
]);
const creation = image([
  line('New_Hero', 1061, 1071), line('FORBIDDEN RITES', 1077, 1149), line('CONTINUE', 1192, 1281),
  line('DRUID', 675, 1028),
]);

test('recognizes a single existing character with menu anchors and a league', () => {
  assert.equal(classifyScreen(selection), 'selection');
  assert.deepEqual(recognizeScreen(selection).candidate, { name: 'Existing_Hero', key: 'existing_hero', league: 'Forbidden Rites' });
});
test('creation reads the name field, not the class or league label', () => {
  const result = recognizeScreen(creation);
  assert.equal(result.screen, 'creation');
  assert.equal(result.candidate.name, 'New_Hero');
  assert.equal(result.candidate.league, 'FORBIDDEN RITES');
});
test('a missing or emptied name field never becomes the previous character', () => {
  const empty = { ...creation, lines: creation.lines.slice(1) };
  assert.equal(recognizeScreen(empty).status, 'no-name');
  assert.equal(recognizeScreen(empty).candidate, null);
});
test('multiple character rows are rejected until the selected marker is validated', () => {
  const two = { ...selection, lines: [...selection.lines, line('Another_Hero', 2036, 510)] };
  assert.equal(recognizeScreen(two).status, 'ambiguous');
  assert.equal(recognizeScreen(two).candidate, null);
});
test('full/crop OCR disagreement never binds the closest name', () => {
  const different = { ...creation, lines: [line('Other_Hero', 1061, 1071), ...creation.lines.slice(1)] };
  assert.equal(recognizeScreen(creation, different).status, 'ambiguous');
});
test('loading screens and in-game chat containing names are not identity screens', () => {
  assert.equal(classifyScreen(image([line('Existing_Hero', 2036, 326)])), 'unknown');
  assert.equal(recognizeScreen(image([])).candidate, null);
});
test('a generic Continue dialog without the creation class title is not a creation screen', () => {
  assert.equal(classifyScreen({ ...creation, lines: creation.lines.filter((entry) => entry.text !== 'DRUID') }), 'unknown');
});
test('case differences do not create a new character identity', () => {
  assert.equal(nameKey('  Existing_Hero '), nameKey('EXISTING_HERO'));
  const upper = { ...creation, lines: [line('NEW_HERO', 1061, 1071), ...creation.lines.slice(1)] };
  assert.equal(recognizeScreen(creation, upper).status, 'candidate');
});

const styled = (text, y, selected = false) => ({ ...line(text, 2038, y), style: {
  goldPixels: 1800, goldMean: selected ? 160 : 103, brightFraction: selected ? .68 : 0,
} });
const multiple = (selected = 1) => image([
  styled('First_Hero', 326, selected === 0), line('Runes of Aldur League', 2186, 431),
  styled('RememberedCharacterName', 470, selected === 1), line('Runes of Aldur League', 2186, 575),
  styled('Third_Hero', 614, selected === 2), line('Runes of Aldur League', 2186, 719),
  line('CREATE', 2105, 1082), line('DELETE', 2382, 1082), line('PLAY', 942, 1281),
]);
const tooltip = () => {
  const data = multiple();
  data.lines[2] = styled('REMEMBEREDCHAX', 470, true);
  data.lines.push(line('Runes of Aldur League', 2300, 471));
  return data;
};
const known = [{ name: 'RememberedCharacterName', league: 'Runes of Aldur' }];

test('chooses the highlighted middle row rather than the first visible name', () => {
  assert.equal(recognizeScreen(multiple()).candidate.name, 'RememberedCharacterName');
});
test('selection is determined by row evidence, not a hard-coded middle position', () => {
  assert.equal(recognizeScreen(multiple(0)).candidate.name, 'First_Hero');
  assert.equal(recognizeScreen(multiple(2)).candidate.name, 'Third_Hero');
});
test('all five visible rows can be selected, including merged tooltip text', () => {
  for (let selected = 0; selected < 5; selected++) {
    const data = image([
      ...Array.from({ length: 5 }, (_, i) => [styled(`Saved_${i}_Hero_Name`, 326 + 144 * i, i === selected),
        line('Runes of Aldur League', 2186, 431 + 144 * i)]).flat(),
      line('CREATE', 2105, 1082), line('DELETE', 2382, 1082), line('PLAY', 942, 1281),
    ]);
    const history = learnKnownCharacters(data);
    assert.equal(recognizeScreen(data).candidate.name, `Saved_${selected}_Hero_Name`);
    const row = data.lines[selected * 2];
    row.text += ' Runes of Aldur League';
    row.words.push({ text: 'Runes', x: 2300, y: row.words[0].y, width: 70, height: 20 },
      { text: 'of', x: 2375, y: row.words[0].y, width: 20, height: 20 },
      { text: 'Aldur', x: 2400, y: row.words[0].y, width: 70, height: 20 });
    assert.equal(recognizeScreen(data, data, history).candidate?.name, `Saved_${selected}_Hero_Name`);
    assert.equal(recognizeScreen(data).candidate, null, 'a possibly clipped unknown title stays unresolved');
    assert.ok(!learnKnownCharacters(data).some((known) => known.name === `Saved_${selected}_Hero_Name`));
  }
});
test('two equally highlighted rows are ambiguous', () => {
  const data = multiple();
  data.lines[0] = styled('First_Hero', 326, true);
  assert.equal(recognizeScreen(data).candidate, null);
});
test('restores the covered selected name from unique same-league history', () => {
  const result = recognizeScreen(tooltip(), tooltip(), known);
  assert.equal(result.candidate.name, 'RememberedCharacterName');
  assert.equal(result.resolution, 'history-prefix');
});
test('an unknown covered name is never saved as a complete new character', () => {
  assert.equal(recognizeScreen(tooltip()).candidate, null);
  const learned = learnKnownCharacters(tooltip());
  assert.deepEqual(learned.map((entry) => entry.name), ['First_Hero', 'Third_Hero']);
});
test('corrupt tooltip OCR cannot turn a truncated title into a new complete character', () => {
  const data = tooltip();
  data.lines.at(-1).text = 'Runes of A1dur Leagu';
  assert.equal(recognizeScreen(data).candidate, null);
  assert.deepEqual(learnKnownCharacters(data).map((entry) => entry.name), ['First_Hero', 'Third_Hero']);
});
test('similar saved prefixes remain ambiguous, including a corrupted edge glyph', () => {
  const conflicting = [...known, { name: 'RememberedChaXname', league: 'Runes of Aldur' }];
  assert.equal(recognizeScreen(tooltip(), tooltip(), conflicting).candidate, null);
});
test('wrong-league and substring-only matches cannot complete a name', () => {
  assert.equal(recognizeScreen(tooltip(), tooltip(), [{ ...known[0], league: 'Other league' }]).candidate, null);
  assert.equal(recognizeScreen(tooltip(), tooltip(), [{ name: 'OtherRememberedCharacterName', league: known[0].league }]).candidate, null);
});
test('a short visible fragment cannot identify a saved character', () => {
  const data = tooltip();
  data.lines[2] = styled('REMEM', 470, true);
  assert.equal(recognizeScreen(data, data, known).candidate, null);
});
test('disagreement about which row is selected rejects stale/mismatched frames', () => {
  assert.equal(recognizeScreen(multiple(0), multiple(1)).candidate, null);
});
test('complete reads retain a previously stored display spelling', () => {
  const data = multiple();
  data.lines[2].text = 'REMEMBEREDCHARACTERNAME';
  assert.equal(recognizeScreen(data, data, known).candidate.name, 'RememberedCharacterName');
});
