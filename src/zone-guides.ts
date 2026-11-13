// Zone guides: how to cross each zone (data/guides/act1.json, Act 1 for now): a schematic
// of the zone's usual shape drawn from where you come in, and short steps. Community patterns
// checked against the game's layout graphs; see docs/act1-zone-tips.md.
import act1 from '../data/guides/act1.json';
import { type Drawing, guideSvg } from './guide-svg';

export interface Step { text: string; src: string[] }
interface Guide { steps: Step[]; drawing: Drawing }
interface GuideFile { zones: Record<string, Guide>; sources: Record<string, string> }

const files: GuideFile[] = [act1 as unknown as GuideFile];

export function guideFor(zone: string | null | undefined): { steps: Step[]; svg: string } | null {
  const guide = zone ? files.map((file) => file.zones[zone]).find(Boolean) : undefined;
  return guide ? { steps: guide.steps, svg: guideSvg(guide.drawing) } : null;
}

/** Where a step comes from, for its tooltip. */
export function sourcesOf(step: Step): string {
  return step.src.map((key) => files.map((file) => file.sources[key]).find(Boolean) ?? key).join('\n');
}
