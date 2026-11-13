// Shared screen recognition: no I/O, no game input.
export const nameKey = (name) => name.trim().normalize('NFC').toLowerCase();
const isName = (name) => /^[\p{L}\p{N}_]{2,80}$/u.test(name);
const wordAt = (line, image) => {
  const word = line.words?.[0];
  return word ? [word.x / image.width, word.y / image.height] : [-1, -1];
};
const within = ([x, y], [left, top, right, bottom]) => x >= left && x <= right && y >= top && y <= bottom;
const inBox = (image, box) => (image.lines ?? []).filter((line) => within(wordAt(line, image), box));
const anchor = (image, text, box) => inBox(image, box).some((line) => line.text.trim().toUpperCase() === text);

export function classifyScreen(image) {
  if (!(image.width > 0 && image.height > 0)) return 'unknown';
  if (anchor(image, 'PLAY', [.25, .82, .60, .98]) &&
      anchor(image, 'CREATE', [.78, .70, .90, .85]) &&
      anchor(image, 'DELETE', [.88, .70, 1, .85])) return 'selection';
  if (anchor(image, 'CONTINUE', [.40, .82, .65, .98]) &&
      inBox(image, [.39, .78, .68, .84]).length > 0 &&
      inBox(image, [.20, .69, .38, .75]).some((line) =>
        /^(WARRIOR|MERCENARY|HUNTRESS|MONK|SORCERESS|WITCH|DRUID|MARAUDER|DUELIST|RANGER|SHADOW|TEMPLAR)$/i.test(line.text.trim()))) return 'creation';
  return 'unknown';
}

const nameRows = (image) => inBox(image, [.79, .215, .84, .70])
  .flatMap((line) => {
    if (isName(line.text.trim())) return [line];
    // Windows OCR can merge a title and the league tooltip into one line.
    // Keep the spatially separated title word; tooltip detection still checks
    // the original words and requires history if that title may be clipped.
    const [title, ...rest] = line.words ?? [];
    if (!title || !isName(title.text.trim()) || !rest.length ||
        !rest.every((word) => word.x > title.x + title.width + image.width * .005 &&
          Math.abs(word.y - title.y) < title.height)) return [];
    return [{ ...line, text: title.text.trim(), words: [title] }];
  }).sort((a, b) => a.words[0].y - b.words[0].y);

function rowLeague(image, row) {
  const at = row.words[0];
  const next = nameRows(image).find((other) => other.words[0].y > at.y + at.height);
  const end = next?.words[0].y ?? Math.min(image.height * .73, at.y + image.height * .12);
  return (image.lines ?? []).find((line) => line.words?.[0]?.x > image.width * .84 &&
    line.words[0].y > at.y + at.height * 1.5 && line.words[0].y < end && /\sLeague$/i.test(line.text))
    ?.text.replace(/\sLeague$/i, '').trim() ?? null;
}

function occluded(image, row) {
  const at = row.words[0];
  // Position, not the tooltip's spelling: OCR can corrupt "League" too.
  // The character title band has no second text label on its right in these references.
  return (image.lines ?? []).some((line) => (line.words ?? []).some((word) =>
    word.x > at.x + image.width * .06 && Math.abs(word.y - at.y) < at.height && word.text.trim().length >= 4));
}

function selectedRow(image) {
  const rows = nameRows(image);
  if (rows.length === 1) return rows[0];
  const ranked = rows.filter((row) => row.style?.goldPixels >= 30).sort((a, b) => b.style.goldMean - a.style.goldMean);
  if (ranked.length !== rows.length || ranked.length < 2) return null;
  const [first, second] = ranked;
  // Measured on the supplied references: selected ~159–160, others ~103;
  // bright gold covers ~67–69% of selected glyphs, 0% of the others.
  return first.style.goldMean >= 135 && first.style.brightFraction >= .40 &&
    first.style.goldMean - second.style.goldMean >= 30 ? first : null;
}

export function learnKnownCharacters(image, focused = image) {
  if (classifyScreen(image) !== 'selection') return [];
  return nameRows(focused).flatMap((row) => {
    const other = nameRows(image).find((r) => Math.abs(r.words[0].y - row.words[0].y) < row.words[0].height);
    const league = rowLeague(focused, row);
    if (!other || !league || occluded(focused, row) || occluded(image, other) ||
        nameKey(row.text) !== nameKey(other.text)) return [];
    return [{ name: row.text.trim(), league, source: 'complete-screen-read' }];
  });
}

export function resolvePartialName(reads, league, knownCharacters) {
  // Ignore the final OCR glyph only at a validated tooltip edge: a half-hidden
  // glyph can be wrong. Never apply arbitrary edit-distance/fuzzy corrections.
  const prefixes = reads.map((text) => [...nameKey(text)].slice(0, -1).join(''));
  if (!league || prefixes.some((prefix) => [...prefix].length < 6)) return null;
  const matches = new Map();
  for (const known of knownCharacters) {
    if (!known?.name || !known?.league || nameKey(known.league) !== nameKey(league)) continue;
    const key = nameKey(known.name);
    if (prefixes.every((prefix) => key.startsWith(prefix) && [...prefix].length >= [...key].length / 2)) matches.set(key, known);
  }
  return matches.size === 1 ? [...matches.values()][0] : null;
}

export function recognizeScreen(image, focused = image, knownCharacters = []) {
  const screen = classifyScreen(image);
  if (screen === 'unknown') return { screen, status: 'unknown', candidate: null };
  if (screen === 'selection') {
    const row = selectedRow(focused);
    const full = selectedRow(image);
    if (!row || !full || Math.abs(row.words[0].y - full.words[0].y) > row.words[0].height) {
      return { screen, status: 'ambiguous', candidate: null, reason: 'The selected character row is not unambiguous.' };
    }
    const league = rowLeague(focused, row);
    const clipped = occluded(focused, row) || occluded(image, full);
    if (clipped) {
      const known = resolvePartialName([row.text, full.text], league, knownCharacters);
      return known ? { screen, status: 'candidate', candidate: { name: known.name, key: nameKey(known.name), league },
        resolution: 'history-prefix', visibleText: row.text, reason: 'Unique same-league history match for the tooltip-covered selected row; still requires session/entry confirmation.' }
        : { screen, status: 'ambiguous', candidate: null, visibleText: row.text,
          reason: 'Tooltip hides the selected name and history has no unique sufficient prefix match.' };
    }
    if (nameKey(row.text) !== nameKey(full.text)) {
      return { screen, status: 'ambiguous', candidate: null, reason: 'Full-frame and selected-row OCR disagree.' };
    }
    const prior = knownCharacters.find((known) => known.name && known.league && nameKey(known.name) === nameKey(row.text) && nameKey(known.league) === nameKey(league ?? ''));
    return { screen, status: 'candidate', candidate: { name: prior?.name ?? row.text.trim(), key: nameKey(row.text), league },
      resolution: 'complete', reason: 'A screen candidate only; the same login session and zone entry must confirm it.' };
  }
  const box = [.405, .73, .60, .78];
  const names = inBox(focused, box).map((line) => line.text.trim()).filter(isName);
  if (names.length !== 1) {
    return { screen, status: names.length > 1 ? 'ambiguous' : 'no-name', candidate: null,
      reason: names.length > 1 ? 'Several names are visible; the selected-row marker is not validated yet.' : 'No valid name in the name field.' };
  }
  const name = names[0];
  // A focused read must agree with the full-frame read; case differences are cosmetic.
  const fullNames = inBox(image, box).map((line) => line.text.trim()).filter(isName);
  if (fullNames.length !== 1 || nameKey(fullNames[0]) !== nameKey(name)) {
    return { screen, status: 'ambiguous', candidate: null, reason: 'Full-frame and name-crop reads disagree.' };
  }
  const league = inBox(focused, [.40, .78, .68, .84])[0]?.text.trim();
  return { screen, status: 'candidate', candidate: { name, key: nameKey(name), league: league ?? null },
    reason: 'A screen candidate only; zone entry and the same login session must still confirm it.' };
}

