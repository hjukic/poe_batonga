export interface KnownCharacter { name: string; league: string; source?: string }
export interface OcrFrame { width: number; height: number; lines: unknown[] }
export interface ScreenResult {
  screen: string; status: string;
  candidate: { name: string; key: string; league: string | null } | null;
  resolution?: string; reason?: string;
}
export function recognizeScreen(image: OcrFrame, focused?: OcrFrame, known?: KnownCharacter[]): ScreenResult;
export function learnKnownCharacters(image: OcrFrame, focused?: OcrFrame): KnownCharacter[];
export function classifyScreen(image: OcrFrame): string;
export function nameKey(name: string): string;
