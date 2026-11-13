// Zone guides: how to cross each zone (data/guides/act1.json, Act 1 for now): a schematic
// of the zone's usual shape drawn from where you come in, and a short hint under each
// checklist row. Community patterns checked against the game's layout graphs; see
// docs/act1-zone-tips.md.
import act1 from '../data/guides/act1.json';
import { type Drawing, guideSvg } from './guide-svg';

export interface Hint { text: string; src: string[] }
interface Guide { drawing: Drawing; hints: Record<string, Hint>; general?: Hint[] }
interface GuideFile { zones: Record<string, Guide>; sources: Record<string, string> }

const files: GuideFile[] = [act1 as unknown as GuideFile];

export interface ZoneGuide { svg: string; hints: Record<string, Hint>; general: Hint[] }

export function guideFor(zone: string | null | undefined): ZoneGuide | null {
  const guide = zone ? files.map((file) => file.zones[zone]).find(Boolean) : undefined;
  return guide ? { svg: guideSvg(guide.drawing), hints: guide.hints ?? {}, general: guide.general ?? [] } : null;
}

/** Where a hint comes from, for its tooltip. */
export function sourcesOf(hint: Hint): string {
  return hint.src.map((key) => files.map((file) => file.sources[key]).find(Boolean) ?? key).join('\n');
}
