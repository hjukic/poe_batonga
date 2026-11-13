// Zone guides: how to cross each zone (Act 1 for now): lines under the checklist rows and for
// the whole zone (data/guides/act1.json, kinds and conditions in hints.ts), and a schematic of
// the zone's usual shape drawn from where you come in (data/guides/act1-drawings.json).
// Community patterns checked against the game's layout graphs; see docs/act1-zone-tips.md.
import act1 from '../data/guides/act1.json';
import act1Drawings from '../data/guides/act1-drawings.json';
import { type Drawing } from './guide-svg';
import { type Hint } from './hints';

export type { Hint } from './hints';
interface Guide { hints?: Record<string, Hint | Hint[]>; general?: Hint[] }
interface GuideFile { zones: Record<string, Guide>; sources: Record<string, string> }
interface DrawingFile { zones: Record<string, Drawing> }

const files: GuideFile[] = [act1 as unknown as GuideFile];
const drawings: DrawingFile[] = [act1Drawings as unknown as DrawingFile];
/** The browser preview's own sample lines (`previewGuide`), laid over the files'. */
const overrides: Record<string, Guide> = {};

export interface ZoneGuide { drawing: Drawing | null; hints: Record<string, Hint | Hint[]>; general: Hint[] }

export function guideFor(zone: string | null | undefined): ZoneGuide | null {
  if (!zone) return null;
  const file = files.map((entry) => entry.zones[zone]).find(Boolean);
  const own = overrides[zone];
  const drawing = drawings.map((entry) => entry.zones[zone]).find(Boolean) ?? null;
  if (!file && !own && !drawing) return null;
  return {
    drawing,
    hints: { ...file?.hints, ...own?.hints },
    general: own?.general ?? file?.general ?? [],
  };
}

/** Browser preview only: sample lines for a zone (they replace the file's lines by step). */
export function setPreviewGuide(zone: string, guide: Guide | null) {
  if (guide) overrides[zone] = guide;
  else delete overrides[zone];
}

/** Where a hint comes from. */
export function sourcesOf(hint: Hint): string {
  return hint.src.map((key) => files.map((file) => file.sources[key]).find(Boolean) ?? key).join('\n');
}
