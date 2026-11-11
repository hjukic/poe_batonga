import { createReadStream } from 'node:fs';
import { mkdir, writeFile } from 'node:fs/promises';
import { dirname } from 'node:path';
import { createInterface } from 'node:readline';

// Offline research utility. Export only campaign area facts, never chat or character data.
const [input, output] = process.argv.slice(2);
if (!input || !output) {
  console.error('Usage: node scripts/collect-zone-observations.mjs <Client.txt> <output.json>');
  process.exit(1);
}
const areas = new Map();
const pending = new Map();
let lines = 0;
let firstDate;
let lastDate;
try {
  for await (const line of createInterface({ input: createReadStream(input), crlfDelay: Infinity })) {
    lines++;
    const record = line.match(/^(\d{4}\/\d{2}\/\d{2}) \d{2}:\d{2}:\d{2} \d+ [\da-f]+ \[(?:DEBUG|INFO) Client (\d+)\] (.*)$/i);
    if (!record) continue;
    const [, date, client, message] = record;
    firstDate ??= date.replaceAll('/', '-');
    lastDate = date.replaceAll('/', '-');
    const generation = message.match(/^Generating level (\d+) area "([^"]+)" with seed \d+$/);
    if (generation) {
      pending.delete(client);
      const [, level, id] = generation;
      if (!/^(G[1-4]|P[1-3])_/i.test(id)) continue;
      if (!areas.has(id)) areas.set(id, { id, names: new Set(), levels: new Set(), generationRecords: 0 });
      const area = areas.get(id);
      area.levels.add(Number(level));
      area.generationRecords++;
      pending.set(client, { area, line: lines });
      continue;
    }
    const source = message.match(/^\[SCENE\] Set Source \[([^\]]+)\]$/);
    const context = pending.get(client);
    if (source && source[1] !== '(null)' && context && lines - context.line <= 50) {
      context.area.names.add(source[1]);
      pending.delete(client);
    }
  }
  const result = {
    formatVersion: 1,
    kind: 'historical-campaign-zone-observations',
    sourceId: 'local-client-log',
    reviewStatus: 'not-validated-for-current-patch',
    observedFrom: firstDate,
    observedThrough: lastDate,
    linesScanned: lines,
    areas: [...areas.values()].sort((a, b) => a.id.localeCompare(b.id, 'en')).map((area) => ({
      id: area.id, names: [...area.names].sort(),
      observedLevels: [...area.levels].sort((a, b) => a - b), generationRecords: area.generationRecords,
    })),
  };
  await mkdir(dirname(output), { recursive: true });
  await writeFile(output, `${JSON.stringify(result, null, 2)}\n`, 'utf8');
  console.log(`Collected ${result.areas.length} campaign area IDs from ${lines} lines. No raw log content exported.`);
} catch (error) {
  console.error(`Zone observation collection failed: ${error.message}`);
  process.exitCode = 1;
}
