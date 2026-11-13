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

// Where the selection screen's buttons are when it stands still (2560x1440,
// the owner's screenshots), and how far from there each may be looked for.
const BUTTONS = [
  { text: 'PLAY', at: [942 / 2560, 1281 / 1440], box: [.15, .65, .70, 1] },
  { text: 'CREATE', at: [2105 / 2560, 1082 / 1440], box: [.60, .50, .97, .97] },
  { text: 'DELETE', at: [2382 / 2560, 1082 / 1440], box: [.70, .50, 1, .97] },
];

/**
 * The selection screen sometimes turns as a whole (live v0.1.108: by -3.9° on a
 * double-click, +10° with the league dropdown open; the three buttons fit one
 * rotation and shift within a few pixels). Its three buttons give that motion;
 * every word is moved back to where it stands on a still screen. `moved`: the
 * screen was not still. Null transform: no such buttons (another screen).
 */
export function steadyView(image) {
  if (!(image?.width > 0 && image?.height > 0)) return { image, moved: false, back: null };
  const found = BUTTONS.map(({ text, at, box }) => {
    const lines = inBox(image, box).filter((line) => line.text.trim().toUpperCase() === text);
    return lines.length === 1 ? { ref: [at[0] * image.width, at[1] * image.height], got: [lines[0].words[0].x, lines[0].words[0].y] } : null;
  });
  if (found.some((point) => !point)) return { image, moved: false, back: null };
  // Least-squares rotation, scale and shift (complex numbers: got = z * ref + t).
  const mean = (key) => [0, 1].map((i) => found.reduce((sum, p) => sum + p[key][i], 0) / found.length);
  const [rx, ry] = mean('ref');
  const [gx, gy] = mean('got');
  let re = 0, im = 0, norm = 0;
  for (const { ref, got } of found) {
    const [ax, ay] = [ref[0] - rx, ref[1] - ry];
    const [bx, by] = [got[0] - gx, got[1] - gy];
    re += bx * ax + by * ay; im += by * ax - bx * ay; norm += ax * ax + ay * ay;
  }
  const [zr, zi] = [re / norm, im / norm];
  const scale = Math.hypot(zr, zi);
  const angle = Math.atan2(zi, zr) * 180 / Math.PI;
  const forward = ([x, y]) => [zr * (x - rx) - zi * (y - ry) + gx, zi * (x - rx) + zr * (y - ry) + gy];
  const fits = found.every(({ ref, got }) => {
    const [x, y] = forward(ref);
    return Math.hypot(x - got[0], y - got[1]) <= image.width * .012;
  });
  if (!fits || Math.abs(angle) > 15 || Math.abs(scale - 1) > .08) return { image, moved: false, back: null };
  const back = ([x, y]) => {
    const [dx, dy] = [x - gx, y - gy];
    const d = scale * scale;
    return [(zr * dx + zi * dy) / d + rx, (zr * dy - zi * dx) / d + ry];
  };
  const shift = Math.hypot(gx - rx, gy - ry);
  if (Math.abs(angle) < .4 && Math.abs(scale - 1) < .01 && shift < image.width * .004) return { image, moved: false, back: null };
  return { image: moveBack(image, back), moved: true, back };
}

const moveBack = (image, back) => ({ ...image, lines: (image.lines ?? []).map((line) => ({ ...line,
  words: (line.words ?? []).map((word) => { const [x, y] = back([word.x, word.y]); return { ...word, x, y }; }) })) });

export function classifyScreen(image) {
  if (!(image.width > 0 && image.height > 0)) return 'unknown';
  const still = steadyView(image).image;
  if (anchor(still, 'PLAY', [.25, .82, .60, .98]) &&
      anchor(still, 'CREATE', [.78, .70, .90, .85]) &&
      anchor(still, 'DELETE', [.88, .70, 1, .85])) return 'selection';
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

// "LEVEL 17 MONK", also as OCR misreads it ("I_EVEL 97 SIORMWEAVER", "[EVEL 95 …").
// Every character row has it, whatever its league; a league dropdown item has not.
const LEVEL_LINE = /^\S{2,7}\s+(\d{1,3})\s+\S/u;
const isLevelLine = (text) => {
  const level = Number(LEVEL_LINE.exec(text.trim())?.[1]);
  return level >= 1 && level <= 100;
};
const ELLIPSIS = /\s*(?:\.{2,}|…)$/u;

/**
 * The league in a row's last line: "<league> League", or cut short by the game
 * ("HC SSF Forbidden Rites Lea...") or read wrong ("Standard i_eague"). Null
 * when it is not there. League names change every few months: nothing here
 * knows one, only the word "League" and where the line is.
 */
export function leagueText(text, last = false) {
  const trimmed = text.trim();
  const cut = ELLIPSIS.test(trimmed);
  const words = trimmed.replace(ELLIPSIS, '').split(/\s+/);
  if (words.length < 2 && !(cut && last)) return null;
  const tail = words.at(-1).toLowerCase().replace(/^(?:i_|i\.|\[|\||1|i)(?=e)/, 'l');
  const leagueWord = tail === 'league' || /^l.ag.e$/.test(tail) ||
    ('league'.startsWith(tail) && (tail.length >= 3 || (cut && tail.length >= 1)));
  if (leagueWord) return words.slice(0, -1).join(' ').trim() || null;
  // A long league is cut before its "League": the row's last line, by position.
  return cut && last ? words.join(' ').trim() || null : null;
}

/** What is listed under a row's title: its level line and its league (or null). */
function rowDetails(image, row) {
  const at = row.words[0];
  const next = nameRows(image).find((other) => other.words[0].y > at.y + at.height);
  const end = next?.words[0].y ?? Math.min(image.height * .73, at.y + image.height * .12);
  const lines = (image.lines ?? []).filter((line) => line.words?.[0]?.x > image.width * .84 &&
    line.words[0].y > at.y + at.height * 1.5 && line.words[0].y < end).sort((a, b) => a.words[0].y - b.words[0].y);
  const level = lines.findIndex((line) => isLevelLine(line.text));
  const after = level >= 0 ? lines.slice(level + 1) : lines;
  const league = after.map((line, index) => leagueText(line.text, level >= 0 && index === after.length - 1 && index >= 1))
    .find((text) => text) ?? null;
  return { level: level >= 0, league };
}

// League names compare loosely: the game cuts long ones short ("HC SSF Forbidden
// Ri..."), the creation screen writes them in capitals, and an unread league is
// no evidence against a name. Character names are unique across leagues; the
// league only narrows which remembered names a covered title may complete to.
const leagueKey = (league) => matchKey(league ?? '').replace(/[^\p{L}\p{N}]/gu, '');
export const sameLeague = (a, b) => {
  const [x, y] = [leagueKey(a), leagueKey(b)];
  return !x || !y || x.startsWith(y) || y.startsWith(x);
};

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

/**
 * The character row under a click (`click`: fractions of the frame, on the still
 * screen). A row double-click enters that row's character, whether or not its
 * highlight could be measured. Only a real character row (its level line under
 * it) counts: an open league dropdown's items lie over the list and have none.
 */
function clickedRow(image, click) {
  if (!click) return null;
  const [x, y] = [click[0] * image.width, click[1] * image.height];
  if (x < image.width * .79 || x > image.width * .99) return null;
  const rows = nameRows(image);
  const index = rows.findLastIndex((row) => row.words[0].y - row.words[0].height * .5 <= y);
  if (index < 0) return null;
  const row = rows[index];
  const next = rows[index + 1];
  const bottom = next ? next.words[0].y - next.words[0].height * .5 : row.words[0].y + image.height * .095;
  return y < bottom && rowDetails(image, row).level ? row : null;
}

/** Both frames on the still screen (one motion, read from the whole frame). */
function stillFrames(image, focused) {
  const view = steadyView(image);
  return { image: view.image, focused: view.back && focused !== image ? moveBack(focused, view.back) : (focused === image ? view.image : focused),
    moved: view.moved, back: view.back };
}

export function learnKnownCharacters(image, focused = image) {
  if (classifyScreen(image) !== 'selection') return [];
  const still = stillFrames(image, focused);
  // A turning screen's titles are read worse (live: "I_EVEL", "DEADEYEOPLOOÃ–OÃœ"):
  // nothing is learned from it.
  if (still.moved) return [];
  return nameRows(still.focused).flatMap((row) => {
    const other = nameRows(still.image).find((r) => Math.abs(r.words[0].y - row.words[0].y) < row.words[0].height);
    const details = rowDetails(still.focused, row);
    if (!other || !(details.level || details.league) || row.clipped || other.clipped ||
        occluded(still.focused, row) || occluded(still.image, other) || matchKey(row.text) !== matchKey(other.text)) return [];
    return [{ name: row.text.trim(), league: details.league ?? '', source: 'complete-screen-read' }];
  });
}

export function resolvePartialName(reads, league, knownCharacters) {
  // Ignore the final OCR glyph only at a validated tooltip edge: a half-hidden
  // glyph can be wrong. Never apply arbitrary edit-distance/fuzzy corrections.
  // The reads agree up to where the tooltip begins; past it each frame sees a
  // different mix of title and tooltip. Use their common visible prefix.
  // An unread league narrows nothing (`sameLeague`); the match must still be unique.
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
  // of this row's tooltip text, at least three glyphs of it before the last
  // (which may be misread). Less is a longer new name clipped by the tooltip
  // ("ANNAB" is not the known "Anna" glued to anything).
  const tooltip = league ? matchKey(`${league}League`.replace(/\s+/g, '')) : null;
  const glued = (read, key) => {
    const rest = read.slice(key.length, -1);
    return tooltip !== null && read.startsWith(key) && [...rest].length >= 3 && tooltip.startsWith(rest);
  };
  const matches = new Map();
  for (const known of knownCharacters) {
    if (!known?.name || !sameLeague(known.league, league)) continue;
    const key = matchKey(known.name);
    const byPrefix = [...prefix].length >= 6 && key.startsWith(prefix) && [...prefix].length >= [...key].length / 2;
    const byTooltip = keys.every((read) => glued(read.join(''), key));
    if ((byPrefix || byTooltip) && !matches.has(key)) matches.set(key, known);
  }
  return matches.size === 1 ? [...matches.values()][0] : null;
}

/**
 * `click`: where the input that took this frame clicked, as fractions of the
 * frame (a row click or double-click), or null. Used only when the highlighted
 * row cannot be told.
 */
export function recognizeScreen(rawImage, rawFocused = rawImage, knownCharacters = [], click = null) {
  const screen = classifyScreen(rawImage);
  if (screen === 'unknown') return { screen, status: 'unknown', candidate: null };
  if (screen === 'selection') {
    const { image, focused, moved, back } = stillFrames(rawImage, rawFocused);
    const at = click && back ? (([x, y]) => [x / image.width, y / image.height])(back([click[0] * image.width, click[1] * image.height])) : click;
    let byClick = false;
    const pick = (frame) => {
      const lit = selectedRow(frame);
      if (lit) return lit;
      const row = clickedRow(frame, at);
      if (row) byClick = true;
      return row;
    };
    const full = pick(image);
    // Under the tooltip the focused crop sometimes drops the covered title line
    // altogether. The full frame then reads it alone, for the history-only path.
    const cropped = pick(focused);
    const row = cropped ?? (full && (full.clipped || occluded(image, full)) &&
      !nameRows(focused).some((other) => Math.abs(other.words[0].y - full.words[0].y) < full.words[0].height) ? full : null);
    if (!row || !full || Math.abs(row.words[0].y - full.words[0].y) > row.words[0].height) {
      return { screen, status: 'ambiguous', candidate: null, reason: 'The selected character row is not unambiguous.' };
    }
    const details = rowDetails(focused, row);
    const fullDetails = rowDetails(image, full);
    // A title is a character's only with its level or league line under it.
    if (!(details.level || details.league || fullDetails.level || fullDetails.league)) {
      return { screen, status: 'ambiguous', candidate: null, reason: 'The selected line has no character level or league under it.' };
    }
    const league = details.league ?? fullDetails.league;
    const how = (resolution) => byClick ? `${resolution}-clicked-row` : resolution;
    const clipped = row.clipped || full.clipped || occluded(focused, row) || occluded(image, full);
    if (clipped) {
      // A glued-on piece can itself be misread ("…NINC Standard"): fall back to
      // the first OCR piece of each read.
      const known = resolvePartialName([row.text, full.text], league, knownCharacters) ??
        resolvePartialName([row.first ?? row.text, full.first ?? full.text], league, knownCharacters);
      return known ? { screen, status: 'candidate', candidate: { name: known.name, key: nameKey(known.name), league },
        resolution: how('history-prefix'), visibleText: row.text, reason: 'Unique same-league history match for the tooltip-covered selected row; still requires session/entry confirmation.' }
        : { screen, status: 'ambiguous', candidate: null, visibleText: row.text,
          reason: 'Tooltip hides the selected name and history has no unique sufficient prefix match.' };
    }
    const knownAs = (text) => knownCharacters.filter((known) => known.name &&
      matchKey(known.name) === matchKey(text) && sameLeague(known.league, league)).slice(0, 1);
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
    // A turning screen is read worse, and its crop is no second read: only a
    // name already seen whole on a still screen is taken from it.
    if (moved && !prior) {
      return { screen, status: 'ambiguous', candidate: null, visibleText: read,
        reason: 'The screen is turning; only a remembered name is taken from it.' };
    }
    // Without a remembered spelling, prefer the plainer read (E over Æ).
    const plain = (text) => text.replace(/[A-Za-z0-9_]/g, '').length;
    const name = prior?.name ?? (plain(full.text) < plain(read) ? full.text : read).trim();
    return { screen, status: 'candidate', candidate: { name, key: nameKey(name), league },
      resolution: how(moved ? 'complete-moved' : 'complete'), reason: 'A screen candidate only; the same login session and zone entry must confirm it.' };
  }
  const [image, focused] = [rawImage, rawFocused];
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
