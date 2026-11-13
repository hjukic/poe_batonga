// The guide's lines (data/guides/act1.json): under a checklist row while it is open, or for
// the whole zone above its schematic (in town: the errands). Each line has a kind, which sets
// its look, and may show only below or from a character level, or after or until a step of
// the zone is done. Pure: scripts/hints.test.mjs.

/** "way": how to get there (the default); "boss": how to fight it (read before the fight, the
 *  panel hides during it); "level": a level target; "tip": a time saver. */
export type HintKind = 'way' | 'boss' | 'level' | 'tip';
export interface Hint {
  text: string;
  src: string[];
  kind?: HintKind;
  /** Only while the character's level is below this (unknown level: shown). */
  below?: number;
  /** Only once the character's level is at least this (unknown level: hidden). */
  from?: number;
  /** Only once this step of the zone is done. */
  after?: string;
  /** Only until this step of the zone is done. */
  until?: string;
}
/** What the conditions are evaluated against: the character's level (null until a level-up
 *  line was seen) and the zone's steps, every visit. */
export interface HintContext {
  level: number | null;
  steps: { id: string; state: string }[];
}

const KINDS: ReadonlySet<string> = new Set<HintKind>(['way', 'boss', 'level', 'tip']);
export const kindOf = (hint: Hint): HintKind => (hint.kind && KINDS.has(hint.kind) ? hint.kind : 'way');

/** A row's hints: one line or several. */
export const asList = (entry: Hint | Hint[] | undefined): Hint[] =>
  entry === undefined ? [] : Array.isArray(entry) ? entry : [entry];

export function hintShown(hint: Hint, context: HintContext): boolean {
  const done = (id: string) => context.steps.some((step) => step.id === id && step.state === 'done');
  if (hint.below !== undefined && context.level !== null && context.level >= hint.below) return false;
  if (hint.from !== undefined && (context.level === null || context.level < hint.from)) return false;
  if (hint.after !== undefined && !done(hint.after)) return false;
  if (hint.until !== undefined && done(hint.until)) return false;
  return true;
}

export const shownHints = (entry: Hint | Hint[] | undefined, context: HintContext): Hint[] =>
  asList(entry).filter((hint) => hintShown(hint, context));

/**
 * The level target the character is under, from the level lines on show: under the open rows
 * listed now and the zone's own lines. Null when the level is unknown or no target is unmet.
 */
export function unmetLevelTarget(
  rowHints: (Hint | Hint[] | undefined)[], general: Hint[], context: HintContext,
): number | null {
  if (context.level === null) return null;
  const level = context.level;
  const targets = [...rowHints.flatMap(asList), ...general]
    .filter((hint) => kindOf(hint) === 'level' && hint.below !== undefined && level < hint.below && hintShown(hint, context))
    .map((hint) => hint.below as number);
  return targets.length ? Math.max(...targets) : null;
}
