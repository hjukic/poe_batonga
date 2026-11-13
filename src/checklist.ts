interface ChecklistContext {
  inZone: boolean;
  character: string | null;
  zone: unknown | null;
  steps: { kind: string; optional: boolean; state: string }[];
}

/** Completion covers the zone's selected objectives across all visits; notes are not tasks. */
export function panelMode(view: ChecklistContext, includeOptional: boolean) {
  if (!view.inZone) return 'idle';
  if (!view.character) return 'choose-character';
  if (!view.zone) return 'unknown-zone';
  const objectives = view.steps.filter((s) => s.kind === 'objective' && (includeOptional || !s.optional));
  return objectives.length > 0 && objectives.every((s) => s.state === 'done') ? 'completed' : 'checklist';
}

/** The header's levels: the zone's ("area 4"; none in a town, which has no monsters) and the
 *  character's ("you 6", none while unknown), the latter marked while under the level target on show. */
export function headerLevels(zoneLevel: number, level: number | null, target: number | null, town = false) {
  return {
    area: town ? null : `area ${zoneLevel}`,
    you: level === null ? null : `you ${level}`,
    under: level !== null && target !== null && level < target,
  };
}

interface HeadingCandidate {
  id: string; kind: string; state: string; optional: boolean;
  /** false: the app cannot tick it by itself (a click or an implied tick only). Missing: tracked. */
  tracked?: boolean;
  target?: string | null;
}
/**
 * The row with the bright arrow, among the rows listed now (this visit's, after the optional
 * filter), in walking order: the one the compass points to (parked), else the first open
 * required row the app can tick by itself, else the first open required row, else the first
 * open row. A "probably done" boss is done and passed over. Null when nothing is open.
 */
export function headingStep(listed: HeadingCandidate[], compassTarget: string | null = null): string | null {
  const open = listed.filter((step) => step.kind === 'objective' && step.state !== 'done');
  const pick = (compassTarget !== null ? open.find((step) => step.target === compassTarget) : undefined)
    ?? open.find((step) => !step.optional && step.tracked !== false)
    ?? open.find((step) => !step.optional)
    ?? open[0];
  return pick?.id ?? null;
}

/**
 * The rows to do now, for the schematic: the row with the bright arrow and the open rows listed
 * before it (rows the arrow passed over because only a click ticks them; they come first on the
 * way). Empty when nothing has the arrow.
 */
export function litSteps(listed: HeadingCandidate[], heading: string | null): string[] {
  if (heading === null) return [];
  const lit: string[] = [];
  for (const step of listed) {
    if (step.kind !== 'objective') continue;
    if (step.id === heading) return [...lit, step.id];
    if (step.state !== 'done') lit.push(step.id);
  }
  return [];
}
