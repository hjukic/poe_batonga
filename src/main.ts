import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
import './style.css';

interface Status {
  visible: boolean;
  /** false = locked: click-through except the header buttons. */
  interactive: boolean;
  toggleShortcut: string;
  shortcutError: string | null;
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

const icon = (paths: string) => `<svg viewBox="0 0 24 24" aria-hidden="true">${paths}</svg>`;
const LOCKED = icon('<rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 8 0v4"/>');
const UNLOCKED = icon('<rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 7.75-1.4"/>');
const GEAR = icon('<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>');
const CLOSE = icon('<path d="M18 6 6 18M6 6l12 12"/>');

document.body.className = 'overlay';
document.querySelector<HTMLElement>('#app')!.innerHTML = `
  <section class="panel" aria-label="Campaign guide">
    <header id="drag">
      <span class="zone"></span>
      <div class="header-actions">
        <button id="lock" class="icon" type="button"></button>
        <button id="settings-toggle" class="icon" type="button" title="Settings" aria-label="Settings" aria-expanded="false" aria-controls="settings">${GEAR}</button>
        <button id="exit" class="icon" type="button" title="Quit" aria-label="Quit">${CLOSE}</button>
      </div>
    </header>
    <div class="route" id="route"></div>
  </section>
  <div id="settings" class="dropdown" aria-label="Settings" hidden>
    <label class="row"><span>Opacity</span><input id="opacity" type="range" min="10" max="90" value="25" /><output id="opacity-value" for="opacity">25%</output></label>
    <div class="row"><span>Show / hide</span><button id="shortcut" class="keycap" type="button" title="Click, then press a new shortcut"></button></div>
    <p id="shortcut-error" class="warning" hidden></p>
    <hr />
    <button id="recenter" class="item" type="button">Recenter overlay</button>
    <button id="reset-progress" class="item" type="button">Reset progress</button>
    <p id="log-warning" class="warning" hidden></p>
    <p id="error" class="warning" role="alert" hidden></p>
  </div>`;

const $ = <T extends HTMLElement = HTMLElement>(selector: string) => document.querySelector<T>(selector)!;
const settings = $('#settings');
const settingsToggle = $('#settings-toggle');
const lockButton = $<HTMLButtonElement>('#lock');
const shortcutButton = $<HTMLButtonElement>('#shortcut');
let status: Status | undefined;
let recording = false;

const shortcutLabel = (accelerator: string) =>
  accelerator.split('+').map((key) => key.replace(/^Key(?=.)/, '').replace(/^Digit(?=.)/, '')).join(' + ');

function showMessage(selector: string, message: string | null) {
  const element = $(selector);
  element.textContent = message ?? '';
  element.hidden = !message;
}

function render(next: Status) {
  status = next;
  document.body.dataset.interactive = String(next.interactive);
  lockButton.innerHTML = next.interactive ? UNLOCKED : LOCKED;
  lockButton.title = next.interactive ? 'Lock' : 'Unlock';
  lockButton.setAttribute('aria-label', lockButton.title);
  lockButton.setAttribute('aria-pressed', String(!next.interactive));
  if (!next.interactive) setSettingsOpen(false);
  if (!recording) shortcutButton.textContent = shortcutLabel(next.toggleShortcut);
  showMessage('#shortcut-error', next.shortcutError);
  showMessage('#error', next.lastError);
}

function renderTracker(view: TrackerView) {
  showMessage('#log-warning', view.log.connected ? null : view.log.message);
  $('.zone').textContent = `${view.zone.name} - Zone Level: ${view.zone.level}`;
  $('#route').replaceChildren(...view.steps.map((step) => {
    const row = document.createElement('div');
    row.className = `step ${step.kind} ${step.state}`;
    row.dataset.id = step.id;
    for (const part of step.parts) {
      const span = document.createElement('span');
      span.textContent = part.text;
      if (part.style) span.className = part.style;
      row.append(span);
    }
    return row;
  }));
}

function setSettingsOpen(open: boolean) {
  settings.hidden = !open;
  settingsToggle.setAttribute('aria-expanded', String(open));
  if (!open) stopRecording();
}

async function call(command: string, args?: Record<string, unknown>) {
  try { render(await invoke<Status>(command, args)); }
  catch (error) { showMessage('#error', String(error)); }
}

// ---- Opacity ----
const slider = $<HTMLInputElement>('#opacity');
function setOpacity(value: number) {
  const opacity = Number.isFinite(value) ? Math.min(90, Math.max(10, value)) : 25;
  document.documentElement.style.setProperty('--panel-opacity', String(opacity / 100));
  slider.value = String(opacity);
  $('#opacity-value').textContent = `${opacity}%`;
}
try { setOpacity(Number(localStorage.getItem('panel-opacity') ?? 25)); }
catch { setOpacity(25); }
slider.addEventListener('input', () => {
  setOpacity(slider.valueAsNumber);
  try { localStorage.setItem('panel-opacity', slider.value); } catch { /* Optional preference. */ }
});

// ---- Show / hide shortcut ----
function acceleratorFrom(event: KeyboardEvent): string | null {
  if (['Control', 'Alt', 'Shift', 'Meta', 'OS'].includes(event.key)) return null;
  const mods = [event.ctrlKey && 'Ctrl', event.altKey && 'Alt', event.shiftKey && 'Shift', event.metaKey && 'Super'].filter(Boolean);
  // A shortcut needs a modifier, except function keys, so plain typing in game isn't caught.
  if (!mods.length && !/^F\d{1,2}$/.test(event.code)) return null;
  return [...mods, event.code].join('+');
}
function stopRecording() {
  recording = false;
  shortcutButton.classList.remove('recording');
  if (status) shortcutButton.textContent = shortcutLabel(status.toggleShortcut);
}
shortcutButton.addEventListener('click', () => {
  recording = true;
  shortcutButton.classList.add('recording');
  shortcutButton.textContent = 'Press keys…';
  showMessage('#shortcut-error', null);
});
document.addEventListener('keydown', (event) => {
  if (recording) {
    event.preventDefault();
    if (event.key === 'Escape') return stopRecording();
    const accelerator = acceleratorFrom(event);
    if (!accelerator) return;
    void invoke<Status>('set_toggle_shortcut', { accelerator })
      .then((next) => { stopRecording(); render(next); })
      .catch((error) => { stopRecording(); showMessage('#shortcut-error', String(error)); });
    return;
  }
  if (event.key === 'Escape') return setSettingsOpen(false);
  // While the overlay itself has focus the global shortcut may not reach it.
  if (status && acceleratorFrom(event)?.toLowerCase() === status.toggleShortcut.toLowerCase()) {
    event.preventDefault();
    void call('set_visibility', { visible: false });
  }
});

// ---- Window size and the clickable area while locked ----
const panel = $('.panel');
const actions = $('.header-actions');
let lastSize = '';
let lastRect = '';
function fitWindow() {
  if (!isTauri()) return;
  const boxes = [panel, settings].filter((element) => !element.hidden).map((element) => element.getBoundingClientRect());
  const width = Math.ceil(Math.max(...boxes.map((box) => box.right))) + 1;
  const height = Math.ceil(Math.max(...boxes.map((box) => box.bottom))) + 1; // body padding
  if (`${width}x${height}` !== lastSize) {
    lastSize = `${width}x${height}`;
    void getCurrentWindow().setSize(new LogicalSize(width, height)).catch(console.error);
  }
  const box = actions.getBoundingClientRect();
  const rect = { x: box.left, y: box.top, width: box.width, height: box.height };
  if (JSON.stringify(rect) !== lastRect) {
    lastRect = JSON.stringify(rect);
    void invoke('set_controls_rect', { rect }).catch(console.error);
  }
}
const observer = new ResizeObserver(fitWindow);
observer.observe(panel);
observer.observe(settings);

// ---- Pointer actions ----
$('#drag').addEventListener('mousedown', (event) => {
  if ((event.target as Element).closest('button')) return;
  if (status?.interactive && event.button === 0 && isTauri()) {
    void getCurrentWindow().startDragging().catch(console.error);
  }
});
$('#route').addEventListener('click', (event) => {
  const row = (event.target as Element).closest<HTMLElement>('.step.objective');
  if (!row || !status?.interactive || !isTauri()) return;
  void invoke<TrackerView>('toggle_step', { id: row.dataset.id }).then(renderTracker).catch((error) => showMessage('#error', String(error)));
});
lockButton.addEventListener('click', () => void call('set_interaction', { interactive: !status?.interactive }));
settingsToggle.addEventListener('click', async () => {
  const open = settings.hidden;
  // The menu needs mouse input, so opening it unlocks the overlay.
  if (open && !status?.interactive) await call('set_interaction', { interactive: true });
  setSettingsOpen(open);
});
document.addEventListener('mousedown', (event) => {
  const target = event.target as Element;
  if (!settings.hidden && !settings.contains(target) && !settingsToggle.contains(target)) setSettingsOpen(false);
});
$('#exit').addEventListener('click', () => void invoke('quit').catch(console.error));
$('#recenter').addEventListener('click', () => void call('reset_overlay'));
$('#reset-progress').addEventListener('click', () => {
  void invoke<TrackerView>('reset_progress').then(renderTracker).catch((error) => showMessage('#error', String(error)));
});

// Shown only in a plain browser (npm run dev without Tauri), where no log can be read.
const browserPreview: TrackerView = {
  zone: { id: 'G1_1', name: 'The Riverbank', level: 1 },
  currentArea: null,
  log: { connected: true, path: null, message: '' },
  steps: [
    { id: 'talk-wounded-man', kind: 'objective', parts: [{ text: 'Talk to the Wounded Man' }], state: 'done', source: 'log' },
    { id: 'directions-river', kind: 'note', parts: [{ text: 'Directions: Follow the river upstream', style: 'direction' }], state: 'note', source: null },
    { id: 'kill-bloated-miller', kind: 'objective', parts: [{ text: 'Kill ' }, { text: 'The Bloated Miller', style: 'boss' }], state: 'manual', source: null },
    { id: 'enter-clearfell-encampment', kind: 'objective', parts: [{ text: 'Enter Clearfell Encampment' }], state: 'open', source: null },
  ],
};

async function start() {
  if (!isTauri()) {
    render({ visible: true, interactive: true, toggleShortcut: 'Ctrl+Space', shortcutError: null, lastError: null });
    renderTracker(browserPreview);
    return;
  }
  await listen<Status>('overlay-status', (event) => render(event.payload));
  await listen<TrackerView>('tracker-state', (event) => renderTracker(event.payload));
  render(await invoke<Status>('overlay_status'));
  renderTracker(await invoke<TrackerView>('tracker_state'));
  fitWindow();
}
void start().catch((error) => showMessage('#error', String(error)));
