// Renders the panel for each view of a replayed run, with headless Chromium, to PNG files:
// what the player would see at each step. The views come from the tracker itself:
//   BATONGA_VIEWS_OUT=<views.jsonl> cargo test dump_act_one_views -- --ignored   (in src-tauri)
// The panel runs in a plain browser (`npm run dev`, or vite on another port); then
//   node scripts/render-views.mjs <views.jsonl> <outdir> [url] [backdrop image] [setting=value ...]
// A setting is a localStorage key of the panel, e.g. optional-content=true, panel-opacity=25.
import { readFileSync, writeFileSync, mkdirSync, rmSync } from 'node:fs';
import { spawn } from 'node:child_process';

const [viewsFile, out, url = 'http://127.0.0.1:1420/', backdropFile, ...pairs] = process.argv.slice(2);
if (!viewsFile || !out) throw new Error('usage: render-views.mjs <views.jsonl> <outdir> [url] [backdrop] [setting=value ...]');
const views = readFileSync(viewsFile, 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l));
const settings = Object.fromEntries(pairs.map((p) => p.split('=')));
const backdrop = backdropFile && backdropFile !== '-' ? `data:image/jpeg;base64,${readFileSync(backdropFile).toString('base64')}` : null;
mkdirSync(out, { recursive: true });

const port = 9351;
const profile = `${out}/.profile`;
const chrome = spawn('chromium', ['--headless=new', `--remote-debugging-port=${port}`, `--user-data-dir=${profile}`, '--hide-scrollbars', 'about:blank'], { stdio: 'ignore' });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let target;
for (let i = 0; i < 50 && !target; i++) {
  await sleep(200);
  try { target = (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).find((t) => t.type === 'page'); } catch { /* not up yet */ }
}
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener('open', r));
let seq = 0;
const waiting = new Map();
const problems = [];
ws.addEventListener('message', (m) => {
  const d = JSON.parse(m.data);
  if (d.id && waiting.has(d.id)) { waiting.get(d.id)(d); waiting.delete(d.id); return; }
  if (d.method === 'Runtime.exceptionThrown') problems.push(`exception: ${d.params.exceptionDetails.exception?.description ?? d.params.exceptionDetails.text}`);
  if (d.method === 'Runtime.consoleAPICalled' && ['error', 'assert'].includes(d.params.type)) problems.push(`console.${d.params.type}: ${d.params.args.map((a) => a.value ?? a.description).join(' ')}`);
});
const send = (method, params = {}) => new Promise((res, rej) => {
  const id = ++seq;
  waiting.set(id, (d) => (d.error ? rej(new Error(`${method}: ${d.error.message}`)) : res(d.result)));
  ws.send(JSON.stringify({ id, method, params }));
});
const evaluate = async (expression) => {
  const r = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? 'evaluation failed');
  return r.result.value;
};
await send('Page.enable');
await send('Runtime.enable');
await send('Emulation.setDeviceMetricsOverride', { width: 420, height: 1300, deviceScaleFactor: 2, mobile: false });
if (!backdrop) await send('Emulation.setDefaultBackgroundColorOverride', { color: { r: 0, g: 0, b: 0, a: 0 } });
const locked = { visible: true, interactive: false, toggleShortcut: 'Ctrl+Space', shortcutError: null, lastError: null, layerShell: false };
await send('Page.navigate', { url }); await sleep(900);
await evaluate(`localStorage.clear(); ${Object.entries(settings).map(([k, v]) => `localStorage.setItem(${JSON.stringify(k)}, ${JSON.stringify(v)});`).join(' ')} 1`);
await send('Page.navigate', { url }); await sleep(1200);
const pad = 16;
await evaluate(`(() => { const s = document.body.style; s.padding = '${pad}px'; s.minHeight = '100vh'; ${backdrop ? `s.background = 'url(${backdrop}) 0 0 / 900px auto';` : ''} return 1; })()`);
let bad = 0;
for (const { name, view } of views) {
  problems.length = 0;
  await evaluate(`previewStatus(${JSON.stringify(locked)}); previewTracker(${JSON.stringify(view)}); 1`);
  await sleep(250);
  const box = await evaluate(`(() => { const r = document.querySelector('.panel').getBoundingClientRect(); return { w: Math.ceil(r.right), h: Math.ceil(r.bottom) }; })()`);
  const shot = await send('Page.captureScreenshot', { format: 'png', clip: { x: 0, y: 0, width: box.w + pad, height: box.h + pad, scale: 1 } });
  writeFileSync(`${out}/${name}.png`, Buffer.from(shot.data, 'base64'));
  if (problems.length) bad += 1;
  console.log(name, `${box.w + pad}x${box.h + pad}`, problems.length ? `PROBLEMS: ${problems.join(' | ')}` : 'ok');
}
ws.close();
chrome.kill();
await sleep(300);
rmSync(profile, { recursive: true, force: true });
process.exit(bad ? 1 : 0);
