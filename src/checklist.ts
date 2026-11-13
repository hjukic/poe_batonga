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
