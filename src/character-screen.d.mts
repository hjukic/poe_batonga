export interface KnownCharacter { name: string; league: string; source?: string }
export interface OcrFrame { width: number; height: number; lines: unknown[] }
export interface ScreenResult {
  screen: string; status: string;
  candidate: { name: string; key: string; league: string | null } | null;
  resolution?: string; reason?: string;
}
export function recognizeScreen(image: OcrFrame, focused?: OcrFrame, known?: KnownCharacter[], click?: [number, number] | null): ScreenResult;
export function learnKnownCharacters(image: OcrFrame, focused?: OcrFrame): KnownCharacter[];
export function classifyScreen(image: OcrFrame): string;
export function steadyView(image: OcrFrame): { image: OcrFrame; moved: boolean; back: ((point: [number, number]) => [number, number]) | null };
export function leagueText(text: string, last?: boolean): string | null;
export function sameLeague(a: string | null | undefined, b: string | null | undefined): boolean;
export function nameKey(name: string): string;
export function matchKey(name: string): string;
