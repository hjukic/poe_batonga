// Zone tips: how to cross each zone fast (data/tips/act1.json, Act 1 for now). Community
// tips checked against the game's layout graphs; see docs/act1-zone-tips.md.
import act1 from '../data/tips/act1.json';

interface Tip { text: string; src: string[] }
interface TipFile { zones: Record<string, Tip[]>; sources: Record<string, string> }

const files: TipFile[] = [act1 as TipFile];

/** The tips for a zone id ("G1_4"), in the order to read them. */
export function tipsFor(zone: string | null | undefined): Tip[] {
  if (!zone) return [];
  return files.flatMap((file) => file.zones[zone] ?? []);
}

/** Where a tip comes from, for its tooltip. */
export function sourcesOf(tip: Tip): string {
  return tip.src.map((key) => files.map((file) => file.sources[key]).find(Boolean) ?? key).join('\n');
}
