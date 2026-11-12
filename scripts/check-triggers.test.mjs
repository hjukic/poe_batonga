// Tests for scripts/check-triggers.mjs and the npcLine triggers in data/routes/:
//   node --test scripts/check-triggers.test.mjs
// With TRIGGER_TABLES=<dump folder> (poe_data_tools dump-tables --mode json) it also checks
// every trigger against those tables, as scripts/check-triggers.sh does.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { check, linesFromTables, loadLines, loadRoutes, shownText, triggerLines, variantStem } from './check-triggers.mjs';

const ROUTES = new URL('../data/routes/', import.meta.url).pathname;
const routes = loadRoutes(ROUTES);

test('shownText follows the tracker: markup out, whitespace collapsed', () => {
  // The same cases as npc_lines_compare_in_their_shown_form in src-tauri/src/tracker.rs.
  assert.equal(shownText('My home died with my <i>{akhara}. Already'), 'My home died with my akhara. Already');
  assert.equal(shownText("Something's wrong. <continue>Something's off.\r\n\r\nGo."), "Something's wrong. Something's off. Go.");
  assert.equal(shownText('  All we wanted...\t'), 'All we wanted...');
  assert.equal(shownText(' Yep.<br/>All good.'), 'Yep. All good.');
  assert.equal(shownText('Well done! Please come inside.'), 'Well done! Please come inside.');
});

test('variantStem groups random and class variants', () => {
  for (const id of ['ClearfellGuardMillerDeathRandom', 'ClearfellGuardMillerDeathRandom2']) assert.equal(variantStem(id), 'ClearfellGuardMillerDeath');
  for (const id of ['Oswin_DeathRandom', 'Oswin_Death7']) assert.equal(variantStem(id), 'Oswin_Death');
  for (const id of ['Rudja_Death', 'Rudja_Death_Sorc']) assert.equal(variantStem(id), 'Rudja_Death');
  for (const id of ['ZarkaA2ReturnFromHyenas', 'ZarkaA2ReturnFromHyenasSorceress']) assert.equal(variantStem(id), 'ZarkaA2ReturnFromHyenas');
  assert.equal(variantStem('IronCountEscapeStrFourb'), 'IronCountEscape');
});

// A small table dump: Mortimer's two death variants, a boss whose Name and ShortName differ,
// and one panel line.
const tables = {
  npcs: [
    { Id: 'Metadata/Monsters/NPC/DogtraderRiverbank', Name: 'Mortimer', ShortName: 'Mortimer' },
    { Id: 'Metadata/Monsters/Four_Act1/RustKing', Name: 'The Rust King', ShortName: 'Rust King' },
    { Id: 'Metadata/NPC/Four_Act1/Renly', Name: 'Renly', ShortName: 'Renly' },
  ],
  npctextaudio: [
    { Id: 'ClearfellGuardMillerDeathRandom', Text: 'Well done! Please come inside.', NPCs: { TableName: 'NPCs', Ids: ['Metadata/Monsters/NPC/DogtraderRiverbank'] } },
    { Id: 'ClearfellGuardMillerDeathRandom2', Text: 'Well done! Come in and speak with the others.', NPCs: { TableName: 'NPCs', Ids: ['Metadata/Monsters/NPC/DogtraderRiverbank'] } },
    { Id: 'RustKingEmerge', Text: 'This is our land.\r\nWe stand united!', NPCs: { TableName: 'NPCs', Ids: ['Metadata/Monsters/Four_Act1/RustKing'] } },
    { Id: 'RenlyRuneSpikes', Text: 'Here are your spikes.', NPCs: { TableName: 'NPCs', Ids: ['Metadata/NPC/Four_Act1/Renly'] } },
  ],
  npctalkdialoguetextaudio: [{ Id: 'Renly_Talk_A1', Text: 'Take care.', NPC1: { TableName: 'NPCs', Ids: ['Metadata/NPC/Four_Act1/Renly'] } }],
  npctalk: [{ NPCKey: { Id: 'Metadata/NPC/Four_Act1/Renly' }, NPCTextAudioKeys: { TableName: 'NPCTextAudio', Ids: ['RenlyRuneSpikes'] } }],
  npctalkdialoguesteps: [],
};

const route = (steps) => [{ file: 'test.json', zone: { id: 'G1_1' }, steps }];
const step = (id, trigger, evidenceKind) => ({ id, trigger, evidenceKind });
const one = (lines, area = 'G1_1') => ({ type: 'npcLine', area, lines });

test('a trigger that matches the tables passes; kinds come from the tables', () => {
  const lines = linesFromTables(tables);
  assert.equal(lines.get('RustKingEmerge').text, 'This is our land. We stand united!');
  assert.equal(lines.get('RustKingEmerge').kind, 'bark');
  assert.equal(lines.get('RenlyRuneSpikes').kind, 'panel');
  assert.equal(lines.get('Renly_Talk_A1').kind, 'panel');
  const result = check(
    route([
      step('a', one([{ id: 'RustKingEmerge', speaker: 'The Rust King', text: 'This is our land. We stand united!' }]), 'bark'),
      step('b', one([{ id: 'Renly_Talk_A1', speaker: 'Renly', text: 'Take care.' }]), 'panel'),
    ]),
    lines,
  );
  assert.deepEqual(result, { triggers: 2, errors: [], warnings: [] });
});

test('a patch that changes or drops a line is an error that says what changed', () => {
  const patched = structuredClone(tables);
  patched.npctextaudio[2].Text = 'This land is ours!';
  patched.npctextaudio.splice(3, 1);
  const { errors } = check(
    route([
      step('a', one([{ id: 'RustKingEmerge', speaker: 'The Rust King', text: 'This is our land. We stand united!' }])),
      step('b', one([{ id: 'RenlyRuneSpikes', speaker: 'Renly', text: 'Here are your spikes.' }])),
    ]),
    linesFromTables(patched),
  );
  assert.equal(errors.length, 2, errors.join('\n'));
  assert.match(errors[0], /RustKingEmerge text changed\n {4}was: "This is our land\. We stand united!"\n {4}now: "This land is ours!"/);
  assert.match(errors[1], /RenlyRuneSpikes is gone from the game data/);
});

test('the speaker must be the Name the log shows, not the ShortName', () => {
  const { errors } = check(
    route([step('a', one([{ id: 'RustKingEmerge', speaker: 'Rust King', text: 'This is our land. We stand united!' }]))]),
    linesFromTables(tables),
  );
  assert.equal(errors.length, 1);
  assert.match(errors[0], /speaker is The Rust King, the trigger says Rust King \(that is its ShortName/);
});

test('a missing variant, another zone and a wrong evidenceKind are warnings', () => {
  const { errors, warnings } = check(
    route([step('a', one([{ id: 'ClearfellGuardMillerDeathRandom', speaker: 'Mortimer', text: 'Well done! Please come inside.' }], 'G1_town'), 'panel')]),
    linesFromTables(tables),
  );
  assert.deepEqual(errors, []);
  assert.equal(warnings.length, 3, warnings.join('\n'));
  assert.match(warnings[0], /bound to G1_town, the checklist is G1_1/);
  assert.match(warnings[1], /variant ClearfellGuardMillerDeathRandom2 is not in the trigger/);
  assert.match(warnings[2], /evidenceKind panel, the tables say bark/);
});

test('a line named by a dialogue step in its NPCTalkDialogueTextAudio column is panel', () => {
  const t = structuredClone(tables);
  t.npcs.push({ Id: 'Metadata/NPC/HoodedOne', Name: 'The Hooded One', ShortName: '' });
  t.npctextaudio.push({ Id: 'HoodedOneNewTravelEast', Text: 'East we go.', NPCs: { TableName: 'NPCs', Ids: ['Metadata/NPC/HoodedOne'] } });
  assert.equal(linesFromTables(t).get('HoodedOneNewTravelEast').kind, 'bark');
  t.npctalkdialoguesteps.push({ Step: 0, NPCTextAudio: [], NPCTalkDialogueTextAudio: { TableName: 'NPCTalkDialogueTextAudio', Ids: ['HoodedOneNewTravelEast'] } });
  const lines = linesFromTables(t);
  assert.equal(lines.get('HoodedOneNewTravelEast').kind, 'panel');
  const { warnings } = check(route([step('a', one([{ id: 'HoodedOneNewTravelEast', speaker: 'The Hooded One', text: 'East we go.' }]), 'bark')]), lines);
  assert.deepEqual(warnings, ['test.json a: evidenceKind bark, the tables say panel']);
});

test('a line said another way is a warning: another NPC, another row, NPCTalk options that differ', () => {
  // Asala's Halani line: the same row is the Desert Map's 'Travel to the Halani Gates'.
  const asala = 'Metadata/NPC/Four_Act2/MarakethMatriarch';
  const map = 'Metadata/Terrain/Gallows/Act2/2_town/Objects/MapTable';
  const flag = (id) => ({ TableName: 'QuestFlags', Id: id });
  const t = structuredClone(tables);
  t.npcs.push({ Id: asala, Name: 'Sekhema Asala', ShortName: 'Asala' }, { Id: map, Name: 'Desert Map', ShortName: '' });
  t.npctextaudio.push(
    { Id: 'AsalaGatesTwo', Text: 'The Gates are closed!', NPCs: { TableName: 'NPCs', Ids: [asala, map] } },
    { Id: 'AsalaGatesAgain', Text: 'The Gates are closed!', NPCs: { TableName: 'NPCs', Ids: [asala] } },
    { Id: 'GuardGates', Text: 'The Gates are closed!', NPCs: { TableName: 'NPCs', Ids: ['Metadata/NPC/Four_Act1/Renly'] } },
  );
  t.npctalk.push(
    { NPCKey: { Id: asala }, DialogueOption: 'The Closed Gates', QuestFlag2: flag('AsalaClosedGatesChatSeen'), NPCTextAudioKeys: { Ids: ['AsalaGatesTwo'] } },
    { NPCKey: { Id: map }, DialogueOption: 'Travel to the Halani Gates', QuestFlag2: flag('NEVER_SET'), NPCTextAudioKeys: { Ids: ['AsalaGatesTwo'] } },
  );
  const { errors, warnings } = check(
    route([step('a', one([{ id: 'AsalaGatesTwo', speaker: 'Sekhema Asala', text: 'The Gates are closed!' }]), 'panel')]),
    linesFromTables(t),
  );
  assert.deepEqual(errors, []);
  assert.equal(warnings.length, 3, warnings.join('\n'));
  assert.match(warnings[0], /AsalaGatesTwo is also spoken by Desert Map/);
  // Renly's GuardGates has the same text but another speaker: the tracker can't confuse them.
  assert.match(warnings[1], /the text of AsalaGatesTwo is also AsalaGatesAgain \(Sekhema Asala\)/);
  assert.match(warnings[2], /AsalaGatesTwo is said by 2 NPCTalk options that differ: Sekhema Asala 'The Closed Gates' .*AsalaClosedGatesChatSeen.*; Desert Map 'Travel to the Halani Gates' .*NEVER_SET/);
  // One option, or two alike, is not a warning.
  t.npctalk.pop();
  t.npctextaudio[t.npctextaudio.length - 3].NPCs.Ids = [asala];
  t.npctextaudio.splice(t.npctextaudio.length - 2, 1);
  const alone = check(route([step('a', one([{ id: 'AsalaGatesTwo', speaker: 'Sekhema Asala', text: 'The Gates are closed!' }]), 'panel')]), linesFromTables(t));
  assert.deepEqual(alone.warnings, []);
});

test('the one-line form without an id is looked up by speaker and text', () => {
  const lines = linesFromTables(tables);
  const found = check(route([step('a', { type: 'npcLine', area: 'G1_1', speaker: 'Mortimer', text: 'Well done! Please come inside.' })]), lines);
  assert.deepEqual(found.errors, []);
  assert.match(found.warnings[0], /line without an id, matches ClearfellGuardMillerDeathRandom/);
  const wrong = check(route([step('a', { type: 'npcLine', area: 'G1_1', speaker: 'Renly', text: 'Well done! Please come inside.' })]), lines);
  assert.match(wrong.errors[0], /no line "Renly: Well done! Please come inside\." in the game data/);
});

test('every npcLine trigger in data/routes is bound to its zone, names its rows and its evidence', () => {
  let count = 0;
  for (const r of routes) {
    for (const s of r.steps) {
      if (s.trigger?.type !== 'npcLine') continue;
      count++;
      const where = `${r.file} ${s.id}`;
      assert.equal(s.trigger.area, r.zone.id, where);
      // Dialogue-panel text is not logged (measured 2026-11-12): no panel triggers.
      assert.ok(['seen', 'bark'].includes(s.evidenceKind) || s.id === 'talk-wounded-man', `${where}: evidenceKind`);
      for (const line of triggerLines(s.trigger)) {
        assert.ok(line.speaker && line.text, where);
        assert.equal(shownText(line.text), line.text, `${where}: text in shown form`);
        if (s.trigger.lines) {
          assert.match(line.id ?? '', /\S/, `${where}: line without an id`);
          assert.ok(s.evidence.includes(line.id), `${where}: evidence names ${line.id}`);
        }
      }
    }
  }
  assert.ok(count >= 22, `${count} npcLine triggers`);
});

test('the triggers match the game tables in TRIGGER_TABLES', { skip: !process.env.TRIGGER_TABLES && 'TRIGGER_TABLES not set' }, () => {
  const { errors } = check(routes, loadLines(process.env.TRIGGER_TABLES));
  assert.deepEqual(errors, []);
});
