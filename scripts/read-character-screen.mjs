import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { readFile, writeFile, mkdir, rename } from 'node:fs/promises';

// Offline prototype: consumes supplied screenshots, never polls/captures a game window.
import { classifyScreen, recognizeScreen, learnKnownCharacters, nameKey } from '../src/character-screen.mjs';
export { classifyScreen, recognizeScreen, learnKnownCharacters, nameKey, resolvePartialName } from '../src/character-screen.mjs';

async function ocr(imagePath, region) {
  const script = fileURLToPath(new URL('./probe-character-screen.ps1', import.meta.url));
  const { stdout } = await promisify(execFile)('powershell.exe', [
    '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', script, '-ImagePath', path.resolve(imagePath), '-Region', region,
  ], { timeout: 45000, maxBuffer: 2 * 1024 * 1024, windowsHide: true });
  return JSON.parse(stdout.replace(/^\uFEFF/, '').trim());
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const imagePath = process.argv[2];
  if (!imagePath) throw new Error('Usage: node scripts/read-character-screen.mjs <screenshot.png> [--history <local-cache.log>]');
  const historyIndex = process.argv.indexOf('--history');
  const historyPath = historyIndex >= 0 ? process.argv[historyIndex + 1] : null;
  if (historyIndex >= 0 && !historyPath) throw new Error('--history requires a file path');
  let history = { version: 1, characters: [] };
  if (historyPath) {
    try {
      history = JSON.parse(await readFile(historyPath, 'utf8'));
      if (history.version !== 1 || !Array.isArray(history.characters)) throw new Error('Unsupported character history format');
    } catch (error) { if (error.code !== 'ENOENT') throw error; }
  }
  const full = await ocr(imagePath, 'full');
  const screen = classifyScreen(full);
  const focused = screen === 'unknown' ? full : await ocr(imagePath, screen);
  const result = recognizeScreen(full, focused, history.characters);
  if (historyPath) {
    for (const known of learnKnownCharacters(full, focused)) {
      if (!history.characters.some((old) => nameKey(old.name) === nameKey(known.name) && nameKey(old.league) === nameKey(known.league))) history.characters.push(known);
    }
    await mkdir(path.dirname(path.resolve(historyPath)), { recursive: true });
    const temporary = `${path.resolve(historyPath)}.${process.pid}.tmp`;
    await writeFile(temporary, JSON.stringify(history, null, 2), 'utf8');
    await rename(temporary, path.resolve(historyPath));
  }
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}
