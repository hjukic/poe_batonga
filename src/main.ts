import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
import './style.css';

interface Status {
  visible: boolean;
  interactive: boolean;
  shortcutErrors: string[];
  lastError: string | null;
}

interface StepPart { text: string; style?: string }
interface StepView { id: string; kind: string; parts: StepPart[]; state: 'done' | 'open' | 'manual' | 'note'; source: string | null }
interface TrackerView {
  zone: { id: string; name: string; level: number };
  currentArea: string | null;
  steps: StepView[];
  log: { connected: boolean; path: string | null; message: string };
}

const overlay = new URLSearchParams(location.search).get('view') === 'overlay';
document.body.className = overlay ? 'overlay' : 'controls';
const root = document.querySelector<HTMLElement>('#app')!;
root.innerHTML = overlay ? `
  <section class="panel" aria-label="Campaign guide preview">
    <header id="drag">
      <span class="zone">The Riverbank - Zone Level: 1</span>
      <div class="header-actions">
        <button id="settings-toggle" class="icon" type="button" title="Settings" aria-label="Settings" aria-expanded="false" aria-controls="settings">
          <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>
        </button>
        <button id="exit" class="icon" type="button" title="Exit Batonga" aria-label="Exit Batonga">
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M18 6 6 18M6 6l12 12"/></svg>
        </button>
      </div>
    </header>
    <div class="route" id="route"></div>
    <div class="edit-tools">
      <div id="settings" hidden>
        <div class="opacity-control"><label for="opacity">Opacity</label><input id="opacity" type="range" min="10" max="90" value="25" /><output id="opacity-value" for="opacity">25%</output></div>
        <span class="hint">Click a step to tick or untick it by hand.</span>
        <span id="log-status" class="hint">Connecting to the game log…</span>
        <div class="settings-actions"><button id="hide" type="button">Hide overlay</button><button id="open-controls" type="button">Controls window</button></div>
        <dl class="keys"><dt>Ctrl + Alt + O</dt><dd>Show / hide</dd><dt>Ctrl + Alt + I</dt><dd>Lock / unlock</dd></dl>
      </div>
      <footer><span id="mode">CONNECTING</span><kbd>Ctrl + Alt + I to lock</kbd></footer>
    </div>
  </section>` : `
  <div class="control-content"><div class="eyebrow">PHASE 2 · WINDOW EXPERIMENT</div>
    <h1>Batonga overlay</h1>
    <p>A minimal campaign panel, inspired by Qutm. Preview content is shown until game tracking is connected.</p>
    <div class="status-card"><strong id="mode">Connecting…</strong><p id="visibility"></p></div>
    <div class="buttons"><button id="interaction">Enable interaction</button><button id="visibility-toggle">Hide overlay</button><button id="reset">Recenter & lock</button><button id="quit" class="secondary">Quit</button><button id="reset-progress" class="secondary">Reset Riverbank progress</button></div>
    <dl><dt>Ctrl + Alt + I</dt><dd>Toggle interaction</dd><dt>Ctrl + Alt + O</dt><dd>Show / hide (returns to click-through)</dd></dl>
    <p class="hint">In interaction mode, drag the panel header. In click-through mode, mouse input reaches the window underneath. Close this controls window to quit both windows.</p>
    <div id="error" role="alert" hidden></div>
    <div id="shortcuts" role="status"></div>
    <p class="hint">Progress comes from Path of Exile 2's Client.txt, read locally and never changed or uploaded. No inventory scan, screen capture or game input.</p>
    <p id="log-status" class="hint"></p>
  </div>`;

let status: Status | undefined;
let busy = false;
function render(next: Status) {
  status = next;
  document.body.dataset.interactive = String(next.interactive);
  document.querySelector('#mode')!.textContent = next.interactive ? 'INTERACTION ON' : 'CLICK-THROUGH';
  if (overlay) {
    // Returning to click-through closes settings so the play view stays minimal.
    if (!next.interactive) setSettingsOpen(false);
  } else {
    document.querySelector('#visibility')!.textContent = next.visible ? 'Overlay visible' : 'Overlay hidden';
    document.querySelector('#interaction')!.textContent = next.interactive ? 'Lock to click-through' : 'Enable interaction';
    document.querySelector('#visibility-toggle')!.textContent = next.visible ? 'Hide overlay' : 'Show overlay';
    document.querySelector('#shortcuts')!.textContent = next.shortcutErrors.length
      ? `Global shortcuts unavailable: ${next.shortcutErrors.join('; ')}. Use these buttons instead.` : 'Global shortcuts registered. Verify they work while the game is focused.';
    showError(next.lastError);
  }
}
function renderTracker(view: TrackerView) {
  const logStatus = document.querySelector('#log-status');
  if (logStatus) logStatus.textContent = view.log.connected ? `${view.log.message}: ${view.log.path}` : view.log.message;
  if (!overlay) return;
  document.querySelector('.zone')!.textContent = `${view.zone.name} - Zone Level: ${view.zone.level}`;
  const route = document.querySelector<HTMLElement>('#route')!;
  route.replaceChildren(...view.steps.map((step) => {
    const row = document.createElement('div');
    row.className = `step ${step.kind} ${step.state}`;
    row.dataset.id = step.id;
    for (const part of step.parts) {
      const span = document.createElement('span');
      span.textContent = part.text;
      if (part.style) span.className = part.style;
      row.append(span);
    }
    if (step.kind === 'objective') {
      row.title = step.state === 'done'
        ? `Done (${step.source === 'manual' ? 'ticked by hand' : 'from game log'})`
        : step.state === 'manual' ? 'Not tracked from the log yet; tick by hand' : 'Ticks automatically from the game log';
    }
    return row;
  }));
}
function setSettingsOpen(open: boolean) {
  const settings = document.querySelector<HTMLElement>('#settings');
  if (!settings) return;
  settings.hidden = !open;
  document.querySelector('#settings-toggle')!.setAttribute('aria-expanded', String(open));
}
function showError(message: string | null) {
  const element = document.querySelector<HTMLElement>('#error');
  if (element) { element.textContent = message; element.hidden = !message; }
}
async function action(command: string, args?: Record<string, unknown>) {
  if (busy) return;
  busy = true;
  try { render(await invoke<Status>(command, args)); }
  catch (error) { showError(String(error)); }
  finally { busy = false; }
}

if (overlay) {
  const slider = document.querySelector<HTMLInputElement>('#opacity')!;
  function setOpacity(value: number) {
    const opacity = Number.isFinite(value) ? Math.min(90, Math.max(10, value)) : 25;
    document.documentElement.style.setProperty('--panel-opacity', String(opacity / 100));
    slider.value = String(opacity);
    document.querySelector('#opacity-value')!.textContent = `${opacity}%`;
  }
  try { setOpacity(Number(localStorage.getItem('panel-opacity') ?? 25)); }
  catch { setOpacity(25); }
  slider.addEventListener('input', () => {
    setOpacity(slider.valueAsNumber);
    try { localStorage.setItem('panel-opacity', slider.value); } catch { /* Optional preference. */ }
  });
  // The window follows the panel, so it never leaves empty transparent space around the text.
  const panel = document.querySelector<HTMLElement>('.panel')!;
  let lastSize = '';
  new ResizeObserver(() => {
    if (!isTauri()) return;
    const box = panel.getBoundingClientRect();
    const width = Math.ceil(box.width) + 2, height = Math.ceil(box.height) + 2; // body padding
    if (`${width}x${height}` === lastSize) return;
    lastSize = `${width}x${height}`;
    void getCurrentWindow().setSize(new LogicalSize(width, height)).catch(console.error);
  }).observe(panel);
  document.querySelector('#drag')!.addEventListener('mousedown', (event) => {
    if ((event.target as Element).closest('button')) return;
    if (status?.interactive && (event as MouseEvent).button === 0 && isTauri()) {
      void getCurrentWindow().startDragging().catch(console.error);
    }
  });
  document.querySelector('#route')!.addEventListener('click', (event) => {
    const row = (event.target as Element).closest<HTMLElement>('.step.objective');
    if (!row || !status?.interactive || !isTauri()) return;
    void invoke<TrackerView>('toggle_step', { id: row.dataset.id }).then(renderTracker).catch(console.error);
  });
  document.querySelector('#settings-toggle')!.addEventListener('click', () => {
    setSettingsOpen(document.querySelector<HTMLElement>('#settings')!.hidden);
  });
  document.querySelector('#exit')!.addEventListener('click', () => void invoke('quit').catch(console.error));
  document.querySelector('#hide')!.addEventListener('click', () => void action('set_visibility', { visible: false }));
  document.querySelector('#open-controls')!.addEventListener('click', () => void invoke('open_controls').catch(console.error));
} else {
  document.querySelector('#interaction')!.addEventListener('click', () => void action('set_interaction', { interactive: !status?.interactive }));
  document.querySelector('#visibility-toggle')!.addEventListener('click', () => void action('set_visibility', { visible: !status?.visible }));
  document.querySelector('#reset')!.addEventListener('click', () => void action('reset_overlay'));
  document.querySelector('#quit')!.addEventListener('click', () => void invoke('quit').catch((error) => showError(String(error))));
  document.querySelector('#reset-progress')!.addEventListener('click', () => {
    void invoke<TrackerView>('reset_progress').then(renderTracker).catch((error) => showError(String(error)));
  });
}

// Shown only in a plain browser (npm run dev without Tauri), where no log can be read.
const browserPreview: TrackerView = {
  zone: { id: 'G1_1', name: 'The Riverbank', level: 1 },
  currentArea: null,
  log: { connected: false, path: null, message: 'Browser preview: no game log.' },
  steps: [
    { id: 'talk-wounded-man', kind: 'objective', parts: [{ text: 'Talk to the Wounded Man' }], state: 'done', source: 'log' },
    { id: 'directions-river', kind: 'note', parts: [{ text: 'Directions: Follow the river upstream', style: 'direction' }], state: 'note', source: null },
    { id: 'kill-bloated-miller', kind: 'objective', parts: [{ text: 'Kill ' }, { text: 'The Bloated Miller', style: 'boss' }], state: 'manual', source: null },
    { id: 'enter-clearfell-encampment', kind: 'objective', parts: [{ text: 'Enter Clearfell Encampment' }], state: 'open', source: null },
  ],
};

async function start() {
  if (!isTauri()) {
    render({ visible: true, interactive: true, shortcutErrors: ['Browser preview only; native window behavior is unavailable.'], lastError: null });
    renderTracker(browserPreview);
    document.querySelectorAll<HTMLButtonElement>('button').forEach((button) => button.disabled = true);
    return;
  }
  await listen<Status>('overlay-status', (event) => render(event.payload));
  await listen<TrackerView>('tracker-state', (event) => renderTracker(event.payload));
  render(await invoke<Status>('overlay_status'));
  renderTracker(await invoke<TrackerView>('tracker_state'));
}
void start().catch((error) => showError(String(error)));
