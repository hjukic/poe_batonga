import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { performance } from 'node:perf_hooks';
import { PNG } from 'pngjs';
import { extract, analyze, readPng, maskPng, PROFILE, LIMITS } from './baseline.mjs';
import { alpha, beta, scene, cases } from './fixtures.mjs';

const [manifestFile, outputArg] = process.argv.slice(2);
const output = resolve(outputArg ?? 'experiments/map-reading/output');
await mkdir(output, { recursive: true });
let templates, inputs, profile = PROFILE, datasetKind = 'synthetic-development';
if (manifestFile) {
  const manifest = JSON.parse(await readFile(manifestFile, 'utf8'));
  if (!Array.isArray(manifest.templates) || !Array.isArray(manifest.frames) || !manifest.frames.length) throw Error('Manifest needs templates and at least one frame');
  if (new Set(manifest.templates.map((t) => t.id)).size !== manifest.templates.length) throw Error('Template IDs must be unique');
  profile = manifest.profile ?? PROFILE;
  datasetKind = manifest.datasetKind ?? 'unlabelled-local-samples';
  const base = dirname(resolve(manifestFile));
  templates = await Promise.all(manifest.templates.map(async (t) => ({ id: t.id, mask: extract(await readPng(resolve(base, t.file)), { roi: t.roi, profile: manifest.profile ?? PROFILE }) })));
  inputs = await Promise.all(manifest.frames.map(async (f) => ({ id: f.id, image: await readPng(resolve(base, f.file)), roi: f.roi, profile: manifest.profile ?? PROFILE })));
} else {
  templates = [{ id: 'alpha', mask: extract(scene({ segments: alpha, players: [] })) }, { id: 'beta', mask: extract(scene({ segments: beta, players: [] })) }];
  inputs = cases;
}
const records = [];
for (const [index, input] of inputs.entries()) {
  const started = performance.now();
  const mask = extract(input.image, { roi: input.roi, profile: input.profile });
  const result = analyze(mask, templates);
  const elapsedMs = performance.now() - started;
  const image = `frame-${index}.png`, diagnostic = `mask-${index}.png`;
  await writeFile(resolve(output, image), PNG.sync.write(input.image));
  await writeFile(resolve(output, diagnostic), maskPng(mask));
  records.push({ id: input.id, expected: input.expected, image, diagnostic, elapsedMs, ...result });
}
const report = { kind: manifestFile ? 'offline-user-samples' : 'synthetic-development-cases', datasetKind, realGameplayValidated: false, liveNavigationEnabled: false, profile, limits: LIMITS, records };
await writeFile(resolve(output, 'results.json'), `${JSON.stringify(report, null, 2)}\n`);
const escape = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
await writeFile(resolve(output, 'report.html'), `<!doctype html><html lang="en"><meta charset="utf-8"><title>Offline map reading experiment</title><style>body{background:#16191f;color:#eee;font:15px system-ui;max-width:1000px;margin:40px auto;padding:20px}article{border-top:1px solid #454a55;padding:20px 0}img{width:256px;image-rendering:pixelated;margin-right:15px}code{color:#9bd9db}small{color:#aab}pre{white-space:pre-wrap}</style><h1>Offline map reading experiment</h1><p>${manifestFile ? 'Local user samples' : 'SYNTHETIC DEVELOPMENT CASES — not gameplay validation'}. No live capture or game navigation.</p><p>Original frame → extracted map mask / detected marker</p>${records.map((r) => `<article><h2>${escape(r.id)}</h2><img src="${r.image}" alt="Input frame"><img src="${r.diagnostic}" alt="Extracted geometry"><p><code>${escape(r.decision)} · ${escape(r.reason)}</code> <small>${r.elapsedMs.toFixed(1)} ms</small></p><pre>${escape(JSON.stringify({ rankings: r.rankings, player: r.playerInTemplate }, null, 2))}</pre></article>`).join('')}</html>`);
console.log(JSON.stringify({ report: resolve(output, 'report.html'), kind: report.kind, candidates: records.filter((r) => r.decision === 'candidate').length, unknown: records.filter((r) => r.decision === 'unknown').length, maxElapsedMs: Math.max(...records.map((r) => r.elapsedMs)) }, null, 2));
