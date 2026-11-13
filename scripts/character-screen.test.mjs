import { test } from 'node:test';
import { strict as assert } from 'node:assert';
import { classifyScreen, recognizeScreen, nameKey, learnKnownCharacters } from './read-character-screen.mjs';
import { resolvePartialName, leagueText, sameLeague } from '../src/character-screen.mjs';

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

const covered = (row) => { row.words[0].width = 2295 - row.words[0].x; return row; };
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
  data.lines[2] = covered(styled('REMEMBEREDCHAX', 470, true));
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
    const row = covered(data.lines[selected * 2]);
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
  data.lines[2] = covered(styled('REMEM', 470, true));
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
test('first row: a tooltip-edge symbol ("NAME&") is completed from history, never learned', () => {
  // Shape of the owner's live v0.1.92 first-row click: the tooltip opens over
  // the clicked title and OCR turns its last half-hidden glyph into "&".
  for (let selected = 0; selected < 3; selected++) {
    const data = multiple(selected);
    const at = selected * 2;
    const y = data.lines[at].words[0].y;
    const full = ['First_Hero', 'RememberedCharacterName', 'Third_Hero_Name'][selected];
    data.lines[at] = styled(`${full.slice(0, -3).toUpperCase()}&`, y, true);
    data.lines.push(line('orbidden Rites League', 2303, y - 1));
    const history = [{ name: full, league: 'Runes of Aldur' }];
    const result = recognizeScreen(data, data, history);
    assert.equal(result.candidate?.name, full, `row ${selected + 1}`);
    assert.equal(result.resolution, 'history-prefix');
    assert.equal(recognizeScreen(data).candidate, null, 'unknown clipped title stays unresolved');
    assert.ok(!learnKnownCharacters(data).some((known) => nameKey(known.name).startsWith(nameKey(full.slice(0, 5)))));
  }
});

// Shapes from the owner's live v0.1.95 Standard-league list (five rows, slider).
const word = (text, x, y, width) => ({ text, words: [{ text, x, y, width, height: 22 }] });
const gold = (row, selected) => ({ ...row, style: { goldPixels: 1800, goldMean: selected ? 160 : 103, brightFraction: selected ? .68 : 0 } });
const standard = (selectedTitle, extra = []) => image([
  gold(word('EQUIPMENTMANGEMENTWP', 2039, 326, 330), false), line('Standard League', 2185, 430),
  gold(word(selectedTitle, 2036, 470, 180), true), line('Standard League', 2185, 574),
  gold(word('FEMBOYFUTAHOUSE', 2039, 614, 250), false), line('Standard League', 2185, 717),
  ...extra,
  line('CREATE', 2105, 1082), line('DELETE', 2382, 1082), line('PLAY', 942, 1281),
]);

test('icon bullet and stray apostrophe around a title still find the highlighted row', () => {
  const data = standard("•EVOTllZV0Ll'", [line('Standard League', 2365, 469)]);
  const history = [{ name: 'EvoTllZV0Ll', league: 'Standard' }];
  assert.equal(recognizeScreen(data, data, history).candidate?.name, 'EvoTllZV0Ll');
  assert.equal(recognizeScreen(data).candidate, null, 'a symbol-edged unknown title stays unresolved');
});
test('OCR I/l/1 and O/0 variants of one title reach the same saved spelling', () => {
  const data = standard('EvoTllZVOL1');
  const history = [{ name: 'EvoTllZV0Ll', league: 'Standard' }];
  assert.equal(recognizeScreen(data, data, history).candidate.name, 'EvoTllZV0Ll');
  const crop = standard('EVOTIIZVOLI');
  assert.equal(recognizeScreen(data, crop).status, 'candidate', 'full/crop confusable variants agree');
});
test('a tooltip opening well right of a short title leaves it complete', () => {
  const data = standard('EvoTiIzvoli', [line('Standard League', 2365, 469)]);
  assert.equal(recognizeScreen(data).candidate?.name, 'EvoTiIzvoli');
  assert.equal(recognizeScreen(data).resolution, 'complete');
});
test('an accent read on one frame only does not reject the selection', () => {
  const data = standard('EvoTiIzvoli');
  data.lines[4] = gold(word('FÉMBOYFUTAHOUSE', 2037, 614, 250), true);
  data.lines[2] = gold(word('EvoTiIzvoli', 2036, 470, 180), false);
  const crop = standard('EvoTiIzvoli');
  crop.lines[4] = gold(word('FEMBOYFUTAHOUSE', 2039, 614, 250), true);
  crop.lines[2] = gold(word('EvoTiIzvoli', 2036, 470, 180), false);
  assert.equal(recognizeScreen(data, crop).status, 'candidate');
});
test('a half-scrolled row without measurable gold does not hide the highlight', () => {
  const data = standard('EvoTiIzvoli', [{ ...word('KOPFSPRINGEN', 2039, 902, 200), style: { goldPixels: 3, goldMean: 90, brightFraction: 0 } }]);
  assert.equal(recognizeScreen(data).candidate?.name, 'EvoTiIzvoli');
});
test('a tooltip glued into the title word is a clipped prefix, never a complete name', () => {
  const data = standard('FOULWICTHCOVEN&orbidden');
  assert.equal(recognizeScreen(data).candidate, null);
  assert.ok(!learnKnownCharacters(data).some((known) => known.name.startsWith('FOULWICTH')));
  const history = [{ name: 'FOULWICTHCOVENNECRO', league: 'Standard' }];
  assert.equal(recognizeScreen(data, data, history).candidate?.name, 'FOULWICTHCOVENNECRO');
});

test('a long title split into pieces under the tooltip resolves from history', () => {
  // Live v0.1.96 focused crop: "SCH U MACH ERG EWINTWStandard Leagu".
  const pieces = [['SCH', 2037, 49], ['U', 2090, 14], ['MACH', 2108, 70], ['ERG', 2182, 50], ['EWINTWStandard', 2236, 118], ['Leagu', 2362, 60]];
  const split = { text: pieces.map(([t]) => t).join(' '), words: pieces.map(([text, x, width]) => ({ text, x, y: 614, width, height: 22 })),
    style: { goldPixels: 469, goldMean: 149, brightFraction: .59 } };
  const crop = standard('EvoTiIzvoli');
  crop.lines[2] = gold(word('EvoTiIzvoli', 2036, 470, 180), false);
  crop.lines[4] = split;
  const full = standard('EvoTiIzvoli', [line('Standard League', 2365, 613)]);
  full.lines[2] = gold(word('EvoTiIzvoli', 2036, 470, 180), false);
  full.lines[4] = gold(word('SCHUMACHERGEWINTW', 2037, 614, 318), true);
  const history = [{ name: 'SCHUMACHERGEWINTVVREEUU', league: 'Standard' }];
  assert.equal(recognizeScreen(full, crop, history).candidate?.name, 'SCHUMACHERGEWINTVVREEUU');
  assert.equal(recognizeScreen(full, crop).candidate, null);
  assert.ok(!learnKnownCharacters(full, crop).some((known) => known.name.startsWith('SCH')));
});
test('Æ read for E in one frame still agrees, and the plain spelling is kept', () => {
  const full = standard('ODNESENNASINE');
  const crop = standard('ODNÆSENNASINE');
  assert.equal(recognizeScreen(full, crop).candidate?.name, 'ODNESENNASINE');
});

test('a clipped longer new name is not a known shorter name glued to the tooltip', () => {
  const history = [{ name: 'Anna', league: 'Standard' }];
  assert.equal(resolvePartialName(['ANNAB', 'ANNAB'], 'Standard', history), null, 'Annabel, clipped');
  assert.equal(resolvePartialName(['ANNABE', 'ANNABE'], 'Standard', history), null);
  assert.equal(resolvePartialName(['ANNASTANX', 'ANNASTAND'], 'Standard', history)?.name, 'Anna', 'really glued');
});

test('live v0.1.97: a glued "…NINC Standard League" read resolves from history', () => {
  const merged = (end) => ({ text: `MOBILEDEPROVISIONINC Standard ${end}`, style: { goldPixels: 2100, goldMean: 158, brightFraction: .68 },
    words: [{ text: 'MOBILEDEPROVISIONINC', x: 2039, y: 758, width: 313, height: 22 },
      { text: 'Standard', x: 2365, y: 757, width: 80, height: 22 }, { text: end, x: 2450, y: 757, width: 60, height: 22 }] });
  const at = (title) => { const data = standard('EvoTiIzvoli'); data.lines[2] = gold(word('EvoTiIzvoli', 2036, 470, 180), false); data.lines.splice(6, 0, title, line('Standard League', 2185, 862)); return data; };
  const history = [{ name: 'MOBILEDEPROVISIONING', league: 'Standard' }];
  assert.equal(recognizeScreen(at(merged('League')), at(merged('Leagu')), history).candidate?.name, 'MOBILEDEPROVISIONING');
  assert.equal(recognizeScreen(at(merged('League')), at(merged('Leagu'))).candidate, null);
});
test('live v0.1.97: the crop drops the covered first-row title; the full frame resolves it', () => {
  const full = image([
    gold(word('FOULWICTHCOVEN&', 2036, 326, 265), true), line('orbidden Rites League', 2303, 325), line('Forbidden Rites League', 2186, 429),
    gold(word('NOPLACEFORANOTHEROF', 2039, 470, 320), false), line('Forbidden Rites League', 2186, 574),
    line('CREATE', 2105, 1082), line('DELETE', 2382, 1082), line('PLAY', 942, 1281),
  ]);
  const crop = { ...full, lines: full.lines.filter((entry) => !entry.text.startsWith('FOUL')) };
  const history = [{ name: 'FOULWICTHCOVENNECRO', league: 'Forbidden Rites' }];
  assert.equal(recognizeScreen(full, crop, history).candidate?.name, 'FOULWICTHCOVENNECRO');
  assert.equal(recognizeScreen(full, crop).candidate, null, 'never a complete name from one frame');
  const whole = { ...full, lines: full.lines.map((entry) => entry.text === 'FOULWICTHCOVEN&' ? gold(word('FOULWICTHCOVENNECRO', 2036, 326, 325), true) : entry)
    .filter((entry) => !entry.text.startsWith('orbidden')) };
  assert.equal(recognizeScreen(whole, crop).candidate, null, 'an unclipped title still needs both reads');
});

test('live v0.1.98: two disagreeing reads resolve only to the one already known', () => {
  const at = (title) => { const data = standard('EvoTiIzvoli'); data.lines[2] = gold(word('EvoTiIzvoli', 2036, 470, 180), false);
    data.lines[0] = gold(word(title, 2037, 326, 182), true); return data; };
  const history = [{ name: 'AAAABATONGA', league: 'Standard' }];
  assert.equal(recognizeScreen(at('ANAABATONGA'), at('AAAABATONGA'), history).candidate?.name, 'AAAABATONGA');
  assert.equal(recognizeScreen(at('ANAABATONGA'), at('AAAABATONGA')).candidate, null, 'unknown: stays ambiguous');
  const both = [...history, { name: 'ANAABATONGA', league: 'Standard' }];
  assert.equal(recognizeScreen(at('ANAABATONGA'), at('AAAABATONGA'), both).candidate, null, 'both known: ambiguous');
});
test('live v0.1.98: an open league dropdown does not hide the highlighted character', () => {
  const data = image([
    { ...line('Standard', 2040, 332), style: { goldPixels: 40, goldMean: 89, brightFraction: 0 } },
    { text: 'Runes of Aldur', words: [{ text: 'Runes', x: 2041, y: 365, width: 40, height: 18 }, { text: 'of', x: 2090, y: 365, width: 12, height: 18 }, { text: 'Aldur', x: 2110, y: 365, width: 40, height: 18 }],
      style: { goldPixels: 139, goldMean: 144, brightFraction: .26 } },
    gold(word('REDEFININGMASCULINITY', 2039, 470, 324), false), line('Runes of Aldur League', 2186, 574),
    gold(word('STONAMENIVIDISCURICE', 2037, 614, 317), true), line('Runes of Aldur League', 2186, 718),
    line('CREATE', 2105, 1082), line('DELETE', 2382, 1082), line('PLAY', 942, 1281),
  ]);
  assert.equal(recognizeScreen(data).candidate?.name, 'STONAMENIVIDISCURICE');
});

// ---- v0.1.109: a turning screen, row double-clicks, and leagues the code does not know ----

// A still screen as live v0.1.108 read it: title, level, location and league per row.
const characterRow = (title, y, { selected = false, league = 'Standard League', location = 'Hideout', level = 'LEVEL 95 TACTICIAN' } = {}) => [
  gold(word(title, 2037, y, 250), selected), line(level, 2185, y + 36), line(location, 2185, y + 69), ...(league ? [line(league, 2185, y + 103)] : []),
];
const buttons = () => [line('CREATE', 2105, 1082), line('DELETE', 2382, 1082), line('PLAY', 942, 1281)];
// The whole screen turned by `degrees` about (1280, 720) and shifted, as on the live frames.
const turned = (data, degrees, [dx, dy] = [0, 0]) => {
  const [c, s] = [Math.cos(degrees * Math.PI / 180), Math.sin(degrees * Math.PI / 180)];
  return { ...data, lines: data.lines.map((l) => ({ ...l, words: l.words.map((w) => ({ ...w,
    x: c * (w.x - 1280) - s * (w.y - 720) + 1280 + dx, y: s * (w.x - 1280) + c * (w.y - 720) + 720 + dy })) })) };
};
const remembered = [{ name: 'AAAABATONGA', league: 'Standard' }, { name: 'DEADEYEOPLOOOOL', league: 'Standard' }];

test('live v0.1.108: a screen turned by -3.9° (a double-click) is read back on its still layout', () => {
  const still = image([...characterRow('AAAABATONGA', 328, { selected: true }), ...characterRow('DEADEYEOPLOOOOL', 470), ...buttons()]);
  const moved = turned(still, -3.9, [12, -8]);
  assert.equal(classifyScreen(moved), 'selection');
  const result = recognizeScreen(moved, moved, remembered);
  assert.equal(result.candidate?.name, 'AAAABATONGA');
  assert.equal(result.resolution, 'complete-moved');
  assert.equal(recognizeScreen(turned(still, 10.4, [-90, 30]), undefined, remembered).candidate?.name, 'AAAABATONGA', 'the dropdown frame turned +10°');
});

test('a turning screen never yields a name not seen on a still one', () => {
  const moved = turned(image([...characterRow('NEWHERO', 328, { selected: true }), ...characterRow('DEADEYEOPLOOOOL', 470), ...buttons()]), -3.9);
  const result = recognizeScreen(moved, moved, remembered);
  assert.equal(result.status, 'ambiguous');
  assert.deepEqual(learnKnownCharacters(moved), [], 'nothing is learned from it either');
});

test('a row double-click enters the clicked row when its highlight cannot be measured', () => {
  // Live frame 32: the lit title's gold was not measurable on the turned screen.
  const dim = (row) => ({ ...row, style: { goldPixels: 15, goldMean: 90, brightFraction: 0 } });
  const rows = [...characterRow('AAAABATONGA', 328), ...characterRow('DEADEYEOPLOOOOL', 470)];
  rows[0] = dim(rows[0]); rows[4] = dim(rows[4]);
  const moved = turned(image([...rows, ...buttons()]), -3.9);
  assert.equal(recognizeScreen(moved, moved, remembered).status, 'ambiguous', 'without the click');
  // The click as the hook saw it: on the turned screen, on DEADEYE's row.
  const [c, s] = [Math.cos(-3.9 * Math.PI / 180), Math.sin(-3.9 * Math.PI / 180)];
  const [x, y] = [2150, 500];
  const click = [(c * (x - 1280) - s * (y - 720) + 1280) / 2559, (s * (x - 1280) + c * (y - 720) + 720) / 1439];
  const result = recognizeScreen(moved, moved, remembered, click);
  assert.equal(result.candidate?.name, 'DEADEYEOPLOOOOL');
  assert.equal(result.resolution, 'complete-moved-clicked-row');
});

test('a double-click on an open league dropdown item is no character', () => {
  const item = (text, y) => ({ ...line(text, 2040, y), style: { goldPixels: 900, goldMean: 100, brightFraction: 0 } });
  const rows = [...characterRow('AAAABATONGA', 328), ...characterRow('DEADEYEOPLOOOOL', 470)];
  rows[0] = { ...rows[0], style: { goldPixels: 15, goldMean: 90, brightFraction: 0 } };
  const data = image([...rows, item('Standard', 520), item('Forbidden', 552), ...buttons()]);
  assert.equal(recognizeScreen(data, data, remembered, [2060 / 2559, 556 / 1439]).candidate, null);
});

test('live v0.1.108: a league cut short ("HC SSF Forbidden Rites Lea...") still gives the name', () => {
  const data = image([
    ...characterRow('OGLEDALOGOTOVOOOOO', 326, { selected: true, league: 'HC SSF Forbidden Rites Lea...', location: 'Hideout', level: 'LEVEL 64 DISCIPLE OF VARASHTA' }),
    ...buttons(),
  ]);
  const result = recognizeScreen(data);
  assert.equal(result.candidate?.name, 'OGLEDALOGOTOVOOOOO');
  assert.equal(result.candidate?.league, 'HC SSF Forbidden Rites');
  assert.deepEqual(learnKnownCharacters(data).map((k) => [k.name, k.league]), [['OGLEDALOGOTOVOOOOO', 'HC SSF Forbidden Rites']]);
});

test('any league name works, also one cut before its "League" or not read at all', () => {
  assert.equal(leagueText('Standard League'), 'Standard');
  assert.equal(leagueText('Standard i_eague'), 'Standard');
  assert.equal(leagueText('Fate of the Vaal 1.0 League'), 'Fate of the Vaal 1.0');
  assert.equal(leagueText('HC SSF Some Very Long Ne...', true), 'HC SSF Some Very Long Ne', 'cut: the last line by position');
  assert.equal(leagueText('Act 2: The Ardura Car...', false), null, 'a location is not the last line');
  assert.equal(leagueText('Hideout'), null);
  // Not read at all: the level line still shows this is a character row.
  const data = image([...characterRow('BRANDNEWLEAGUER', 326, { selected: true, league: null }), ...buttons()]);
  const result = recognizeScreen(data);
  assert.equal(result.candidate?.name, 'BRANDNEWLEAGUER');
  assert.equal(result.candidate?.league, null);
});

test('leagues compare loosely: cut short, capitals, unknown', () => {
  assert.ok(sameLeague('HC SSF Forbidden Rites', 'HC SSF Forbidden Ri'));
  assert.ok(sameLeague('FORBIDDEN RITES', 'Forbidden Rites'));
  assert.ok(sameLeague('', 'Standard'));
  assert.ok(sameLeague(null, 'Standard'));
  assert.ok(!sameLeague('Standard', 'Runes of Aldur'));
  // A covered title completes from a name remembered under the cut league.
  const history = [{ name: 'OGLEDALOGOTOVOOOOO', league: 'HC SSF Forbidden Rites' }];
  assert.equal(resolvePartialName(['OGLEDALOGOTO', 'OGLEDALOGOTO'], 'HC SSF Forbidden Ri', history)?.name, 'OGLEDALOGOTOVOOOOO');
  assert.equal(resolvePartialName(['OGLEDALOGOTO', 'OGLEDALOGOTO'], 'Standard', history), null, 'another league');
});