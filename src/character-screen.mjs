// Shared screen recognition: no I/O, no game input.
export const nameKey = (name) => name.trim().normalize('NFC').toLowerCase();
// Matching only (never a display name): Windows OCR reads the game's small-caps
// title font inconsistently between frames (I/l/1, O/0, É/Æ/E). Two reads of
// one title must compare equal, so they reach the same saved profile.
export const matchKey = (name) => nameKey(name).normalize('NFD').replace(/\p{M}/gu, '')
  .replace(/[æœ]/g, 'e').replace(/0/g, 'o').replace(/[1l|]/g, 'i');
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

// A title as OCR returns it. A leading symbol comes from the row's icon
// ("•NAME") and is dropped. A trailing symbol is usually the half-hidden glyph
// at the league tooltip's edge ("NAME&"), and a symbol inside means the tooltip
// text was glued on ("NAME&orbidden"): keep the leading name run, marked
// clipped. A clipped title is only ever completed from history, never learned.
const titleText = (text) => {
  const trimmed = text.trim().replace(/^[^\p{L}\p{N}_]+/u, '');
  if (isName(trimmed)) return { text: trimmed, clipped: false };
  const edge = /^([\p{L}\p{N}_]{2,80})[^\p{L}\p{N}_\s]/u.exec(trimmed);
  return edge ? { text: edge[1], clipped: true } : null;
};

const nameRows = (image) => inBox(image, [.79, .215, .84, .70])
  .flatMap((line) => {
    const words = line.words ?? [];
    if (words.length <= 1) {
      const whole = titleText(line.text);
      return whole ? [{ ...line, text: whole.text, first: whole.text, clipped: whole.clipped }] : [];
    }
    // Several words: an icon fragment ("i'"), the title, and possibly the
    // league tooltip merged into the same OCR line. Take the first real title
    // word; anything after it must be clearly separated on the same baseline.
    const at = words.findIndex((word) => titleText(word.text));
    if (at < 0 || !words.slice(0, at).every((word) => word.text.trim().length <= 2)) return [];
    const first = titleText(words[at].text);
    // Under the tooltip OCR can also split one long title into pieces
    // ("SCH U MACH ERG EWINTWStandard"): pieces closer than a glyph height are
    // one title. Such a title is clipped: the tooltip may be glued on its end.
    let text = first.text;
    let clipped = first.clipped;
    let end = at;
    let symbol = first.clipped; // a symbol ends the title: nothing after it belongs
    while (!symbol && end + 1 < words.length) {
      const prev = words[end];
      const next = words[end + 1];
      const piece = next.text.trim();
      const run = /^[\p{L}\p{N}_]+/u.exec(piece)?.[0];
      if (!run || next.x - (prev.x + prev.width) > prev.height * .5 || Math.abs(next.y - prev.y) >= prev.height) break;
      text += run;
      clipped = true;
      symbol = run.length < piece.length;
      end += 1;
    }
    const head = words[at];
    const tail = words[end];
    const title = { ...head, text, width: tail.x + tail.width - head.x };
    const rest = words.slice(end + 1);
    if (!rest.every((word) => word.x >= title.x + title.width &&
        Math.abs(word.y - title.y) < title.height)) return [];
    return [{ ...line, text, first: first.text, clipped, words: [title] }];
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
  // Position, not the tooltip's spelling: OCR can corrupt "League" too. Only
  // text that begins where the visible title ends can hide part of it; a
  // tooltip that opens well to the right of a short title leaves it whole.
  const edge = at.x + at.width + image.width * .015;
  return (image.lines ?? []).some((line) => (line.words ?? []).some((word) =>
    word !== at && word.x > at.x + image.width * .06 && word.x <= edge &&
    Math.abs(word.y - at.y) < at.height && word.text.trim().length >= 4));
}

function selectedRow(image) {
  const rows = nameRows(image);
  // A lone row is the selection, unless its gold shows it is not highlighted
  // (the highlighted title was dropped from this read).
  if (rows.length === 1 && !(rows[0].style?.goldPixels >= 30 && rows[0].style.brightFraction < .2)) return rows[0];
  // A row without measurable gold (scrolled half out, misread) cannot be the
  // highlighted one, but must not hide the highlight of the others.
  // Character titles have 950+ gold pixels; the league dropdown's small items
  // (one lights up under the mouse) have under 250 and are not rows.
  const ranked = rows.filter((row) => row.style?.goldPixels >= 300).sort((a, b) => b.style.goldMean - a.style.goldMean);
  if (!ranked.length) return null;
  const [first, second] = ranked;
  // Measured on the supplied references: selected ~159–160, others ~103;
  // bright gold covers ~67–69% of selected glyphs, 0% of the others.
  return first.style.goldMean >= 135 && first.style.brightFraction >= .40 &&
    (!second || first.style.goldMean - second.style.goldMean >= 30) ? first : null;
}

export function learnKnownCharacters(image, focused = image) {
  if (classifyScreen(image) !== 'selection') return [];
  return nameRows(focused).flatMap((row) => {
    const other = nameRows(image).find((r) => Math.abs(r.words[0].y - row.words[0].y) < row.words[0].height);
    const league = rowLeague(focused, row);
    if (!other || !league || row.clipped || other.clipped || occluded(focused, row) || occluded(image, other) ||
        matchKey(row.text) !== matchKey(other.text)) return [];
    return [{ name: row.text.trim(), league, source: 'complete-screen-read' }];
  });
}

export function resolvePartialName(reads, league, knownCharacters) {
  // Ignore the final OCR glyph only at a validated tooltip edge: a half-hidden
  // glyph can be wrong. Never apply arbitrary edit-distance/fuzzy corrections.
  // The reads agree up to where the tooltip begins; past it each frame sees a
  // different mix of title and tooltip. Use their common visible prefix.
  if (!league) return null;
  const keys = reads.map((text) => [...matchKey(text)]);
  let common = keys[0] ?? [];
  for (const key of keys.slice(1)) {
    let n = 0;
    while (n < common.length && n < key.length && common[n] === key[n]) n += 1;
    common = common.slice(0, n);
  }
  const prefix = common.slice(0, -1).join('');
  // OCR may also glue the tooltip's own text ("<league> League") onto a title
  // that is still whole: accept that only when the rest is exactly the start
  // of this row's tooltip text.
  const tooltip = matchKey(`${league}League`.replace(/\s+/g, ''));
  const glued = (read, key) => read.startsWith(key) && tooltip.startsWith(read.slice(key.length, -1) || '');
  const matches = new Map();
  for (const known of knownCharacters) {
    if (!known?.name || !known?.league || nameKey(known.league) !== nameKey(league)) continue;
    const key = matchKey(known.name);
    const byPrefix = [...prefix].length >= 6 && key.startsWith(prefix) && [...prefix].length >= [...key].length / 2;
    const byTooltip = keys.every((read) => glued(read.join(''), key));
    if ((byPrefix || byTooltip) && !matches.has(key)) matches.set(key, known);
  }
  return matches.size === 1 ? [...matches.values()][0] : null;
}

export function recognizeScreen(image, focused = image, knownCharacters = []) {
  const screen = classifyScreen(image);
  if (screen === 'unknown') return { screen, status: 'unknown', candidate: null };
  if (screen === 'selection') {
    const full = selectedRow(image);
    // Under the tooltip the focused crop sometimes drops the covered title line
    // altogether. The full frame then reads it alone, for the history-only path.
    const cropped = selectedRow(focused);
    const row = cropped ?? (full && (full.clipped || occluded(image, full)) &&
      !nameRows(focused).some((other) => Math.abs(other.words[0].y - full.words[0].y) < full.words[0].height) ? full : null);
    if (!row || !full || Math.abs(row.words[0].y - full.words[0].y) > row.words[0].height) {
      return { screen, status: 'ambiguous', candidate: null, reason: 'The selected character row is not unambiguous.' };
    }
    const league = rowLeague(focused, row) ?? rowLeague(image, full);
    const clipped = row.clipped || full.clipped || occluded(focused, row) || occluded(image, full);
    if (clipped) {
      // A glued-on piece can itself be misread ("…NINC Standard"): fall back to
      // the first OCR piece of each read.
      const known = resolvePartialName([row.text, full.text], league, knownCharacters) ??
        resolvePartialName([row.first ?? row.text, full.first ?? full.text], league, knownCharacters);
      return known ? { screen, status: 'candidate', candidate: { name: known.name, key: nameKey(known.name), league },
        resolution: 'history-prefix', visibleText: row.text, reason: 'Unique same-league history match for the tooltip-covered selected row; still requires session/entry confirmation.' }
        : { screen, status: 'ambiguous', candidate: null, visibleText: row.text,
          reason: 'Tooltip hides the selected name and history has no unique sufficient prefix match.' };
    }
    const knownAs = (text) => knownCharacters.filter((known) => known.name && known.league &&
      matchKey(known.name) === matchKey(text) && nameKey(known.league) === nameKey(league ?? '')).slice(0, 1);
    let read = row.text;
    if (matchKey(row.text) !== matchKey(full.text)) {
      // Two reads of one unclipped title disagree ("AAAABATONGA"/"ANAABATONGA").
      // Accept only the read that equals a name already seen whole on an
      // earlier screen, and only when the other read is no known name.
      const crop = knownAs(row.text);
      const whole = knownAs(full.text);
      if (crop.length + whole.length !== 1 || [...matchKey(row.text)].length !== [...matchKey(full.text)].length) {
        return { screen, status: 'ambiguous', candidate: null, reason: 'Full-frame and selected-row OCR disagree.' };
      }
      read = crop.length ? row.text : full.text;
    }
    const prior = knownAs(read)[0];
    // Without a remembered spelling, prefer the plainer read (E over Æ).
    const plain = (text) => text.replace(/[A-Za-z0-9_]/g, '').length;
    const name = prior?.name ?? (plain(full.text) < plain(read) ? full.text : read).trim();
    return { screen, status: 'candidate', candidate: { name, key: nameKey(name), league },
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
