import { invoke, isTauri } from '@tauri-apps/api/core';
import { emit, listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import './style.css';

interface Status {
  visible: boolean;
  /** false = locked: click-through except the header buttons. */
  interactive: boolean;
  toggleShortcut: string;
  shortcutError: string | null;
  lastError: string | null;
  /** Wayland layer surface: moved with move_overlay_by instead of window dragging. */
  layerShell: boolean;
}

interface StepPart { text: string; style?: string }
interface StepView {
  id: string; kind: string; parts: StepPart[]; optional: boolean;
  /** The compass target this step stands for, if any. */
  target: string | null;
  state: 'done' | 'open' | 'manual' | 'note'; source: string | null;
  /** Which visit to the zone the step belongs to. */
  visit: number;
}
interface TrackerView {
  zone: { id: string; name: string; level: number };
  currentArea: string | null;
  steps: StepView[];
  /** The visit under way: only its steps are listed. */
  visit: number;
  log: { connected: boolean; path: string | null; message: string };
}

interface MinimapView {
  enabled: boolean;
  visible: boolean;
  message: string;
  player: [number, number] | null;
}

/** The layout the app recognised: its paths and exits in the map's screen axes. */
interface LayoutPicture {
  known: boolean; id: string; share: number; sure: boolean; candidates: number; noFit: boolean;
  paths: [number, number][][]; exits: [string, [number, number]][]; player: [number, number] | null;
}
/** The stitched map of the current zone visit, scaled to a cols x rows grid. */
interface LayoutView {
  area: string;
  seed: number | null;
  part: number;
  frames: number;
  matched: number;
  cols: number;
  rows: number;
  walls: [number, number][];
  trail: [number, number][];
  player: [number, number];
  pois: { kind: string; label: string; x: number; y: number }[];
  guide: GuideView | null;
  /** Only the needle is new: keep the map drawn. */
  light: boolean;
  /** The recognised layout (full views only). */
  layout?: LayoutPicture;
}

/** The compass needle (drawn in its own window): the way to the step's target. */
interface GuideView {
  variant: string;
  targetKind: string;
  target: string;
  /** Screen direction: 0 = right, 90 = down. */
  degrees: number | null;
  source: 'estimated' | 'rough' | 'unconfirmed';
  /** Minimap cells. */
  distance: number;
  /** 0-1: how sure the needle is. */
  confidence: number;
}

const icon = (paths: string) => `<svg viewBox="0 0 24 24" aria-hidden="true">${paths}</svg>`;
const LOCKED = icon('<rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 8 0v4"/>');
const UNLOCKED = icon('<rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 7.75-1.4"/>');
const GEAR = icon('<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>');
const RESTART = icon('<path d="M21 12a9 9 0 1 1-2.64-6.36"/><path d="M21 3v6h-6"/>');
/** Step markers: an arrow in front of every open step, a tick when done. */
const mark = (path: string) => `<svg viewBox="0 0 12 12" aria-hidden="true"><path d="${path}"/></svg>`;
const STEP_ARROW = mark('M1.5 6h8M6.2 2.6 9.6 6 6.2 9.4');
const STEP_DONE = mark('M2 6.5 4.8 9.2 10 3.2');
const CLOSE = icon('<path d="M18 6 6 18M6 6l12 12"/>');

document.body.className = 'overlay';
document.querySelector<HTMLElement>('#app')!.innerHTML = `
  <section class="panel" aria-label="Campaign guide">
    <header id="drag">
      <span class="zone"></span>
      <div class="header-actions">
        <button id="lock" class="icon" type="button"></button>
        <button id="restart" class="icon" type="button" title="Restart with the latest version" aria-label="Restart with the latest version">${RESTART}</button>
        <button id="settings-toggle" class="icon" type="button" title="Settings" aria-label="Settings" aria-expanded="false" aria-controls="settings">${GEAR}</button>
        <button id="exit" class="icon" type="button" title="Quit" aria-label="Quit">${CLOSE}</button>
      </div>
    </header>
    <p class="restart-status" id="restart-status" role="status" hidden></p>
    <div class="route" id="route"></div>
    <div class="layout-pick" id="layout-pick" hidden>
      <p class="layout-name" id="layout-name"></p>
      <canvas id="layout-canvas" width="584" height="400"></canvas>
    </div>
    <div class="minimap" id="minimap" hidden>
      <canvas id="minimap-canvas" width="300" height="260"></canvas>
    </div>
    <div class="version" title="App version: goes up with every update">v${__APP_VERSION__}</div>
  </section>
  <div id="settings" class="dropdown" aria-label="Settings" hidden>
    <label class="row"><span>Opacity</span><input id="opacity" type="range" min="10" max="90" value="25" /><output id="opacity-value" for="opacity">25%</output></label>
    <div class="row"><span>Show / hide</span><button id="shortcut" class="keycap" type="button" title="Click, then press a new shortcut"></button></div>
    <label class="row" title="Off (speedrun): only the steps needed to progress are listed. On: side steps marked optional are listed too."><span>Optional content</span><input id="optional-content" type="checkbox" /></label>
    <label class="row" title="Parked until the layout is recognised reliably (roadmap phase 3). On: the experimental arrow around the character."><span>Compass (parked)</span><input id="compass-enabled" type="checkbox" /></label>
    <label class="row" title="The overlay hides while the game is closed or on a workspace that is not shown"><span>Hide when game is off screen</span><input id="auto-hide" type="checkbox" checked /></label>
    <label class="row" title="The overlay hides when a zone boss starts talking and comes back when the kill is logged, you die or leave, or the boss has been quiet for 30 seconds"><span>Hide during boss fights</span><input id="fight-hide" type="checkbox" checked /></label>
    <label class="row" title="The map recorded so far, for checking the tracking"><span>Map preview</span><input id="map-preview" type="checkbox" /></label>
    <p id="shortcut-error" class="warning" hidden></p>
    <hr />
    <button id="reset-progress" class="item" type="button">Reset progress</button>
    <p id="log-warning" class="warning" hidden></p>
    <p id="error" class="warning" role="alert" hidden></p>
  </div>`;

const $ = <T extends HTMLElement = HTMLElement>(selector: string) => document.querySelector<T>(selector)!;
const settings = $('#settings');
const settingsToggle = $('#settings-toggle');
const lockButton = $<HTMLButtonElement>('#lock');
const shortcutButton = $<HTMLButtonElement>('#shortcut');
const optionalContent = $<HTMLInputElement>('#optional-content');
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
  syncStepControls();
  markHeading();
  if (!next.interactive) setSettingsOpen(false);
  if (!recording) shortcutButton.textContent = shortcutLabel(next.toggleShortcut);
  showMessage('#shortcut-error', next.shortcutError);
  showMessage('#error', next.lastError);
  syncInput();
}

/** A checklist longer than this is folded: done steps become one line, later ones "+N more". */
const MAX_ROWS = 9;
/** Open steps shown while folded. */
const MAX_OPEN = 6;
/** Unfolded by a click on the summary line (to untick a hidden step). */
let unfolded = false;
/** Steps of the zone's other visits are listed too (a click on their summary line). */
let allVisits = false;
let lastTracker: TrackerView | null = null;

function stepRow(step: StepView) {
  const row = document.createElement(step.kind === 'objective' ? 'button' : 'div');
  row.className = `step ${step.kind} ${step.state}${step.optional ? ' optional' : ''}`;
  row.dataset.id = step.id;
  if (step.target) row.dataset.target = step.target;
  if (row instanceof HTMLButtonElement) {
    row.type = 'button';
    row.setAttribute('aria-pressed', String(step.state === 'done'));
    row.addEventListener('click', () => {
      row.disabled = true;
      void invoke<TrackerView>('toggle_step', { id: step.id }).then(renderTracker).catch((error) => {
        showMessage('#error', String(error));
        syncStepControls();
      });
    });
  }
  // One row: marker, text (wraps under itself), reward or "optional" on the right.
  const marker = document.createElement('span');
  marker.className = 'mark';
  if (step.kind === 'objective') marker.innerHTML = step.state === 'done' ? STEP_DONE : STEP_ARROW;
  const text = document.createElement('span');
  text.className = 'text';
  const side = document.createElement('span');
  side.className = 'side';
  for (const part of step.parts) {
    const span = document.createElement('span');
    span.textContent = part.text;
    if (part.style) span.className = part.style;
    (part.style === 'reward' ? side : text).append(span);
  }
  if (step.optional && !side.childElementCount) side.textContent = 'optional';
  row.append(marker, text);
  if (side.textContent) row.append(side);
  return row;
}

/** A muted line that stands for folded steps; a click (unlocked) unfolds or folds the list. */
function summaryRow(text: string, done: boolean, onClick = () => { unfolded = !unfolded; }) {
  const row = document.createElement('button');
  row.type = 'button';
  row.className = 'step summary';
  const marker = document.createElement('span');
  marker.className = 'mark';
  if (done) marker.innerHTML = STEP_DONE;
  const label = document.createElement('span');
  label.className = 'text';
  label.textContent = text;
  row.append(marker, label);
  row.addEventListener('click', () => {
    onClick();
    if (lastTracker) renderTracker(lastTracker);
  });
  return row;
}

function renderTracker(view: TrackerView) {
  if (lastTracker && lastTracker.zone.id !== view.zone.id) {
    unfolded = false;
    allVisits = false;
  }
  lastTracker = view;
  showMessage('#log-warning', view.log.connected ? null : view.log.message);
  // A long name (a town's) takes the room of the level.
  const zone = $('.zone');
  zone.title = `${view.zone.name} · Lv ${view.zone.level}`;
  zone.textContent = view.zone.name.length > 16 ? view.zone.name : zone.title;
  const rows: HTMLElement[] = [];
  // A zone's list covers every visit of the route; only this visit's steps are shown.
  // Speedrun (the default): optional steps are left out of the list.
  const wanted = optionalContent.checked ? view.steps : view.steps.filter((step) => !step.optional);
  const steps = allVisits ? wanted : wanted.filter((step) => step.visit === view.visit);
  const otherVisits = wanted.filter((step) => step.visit !== view.visit && step.kind === 'objective').length;
  if (steps.length <= MAX_ROWS || unfolded) {
    rows.push(...steps.map(stepRow));
    if (unfolded) rows.push(summaryRow('Show less', false));
  } else {
    // Folded: the next open steps, each with the note in front of it; the rest as counts.
    const objectives = steps.filter((step) => step.kind === 'objective');
    const open = objectives.filter((step) => step.state !== 'done');
    const shown = new Set(open.slice(0, MAX_OPEN));
    const done = objectives.length - open.length;
    if (done) rows.push(summaryRow(`${done} done`, true));
    steps.forEach((step, index) => {
      const next = step.kind === 'objective' ? step : steps.slice(index).find((later) => later.kind === 'objective');
      if (next && shown.has(next)) rows.push(stepRow(step));
    });
    if (open.length > shown.size) rows.push(summaryRow(`+${open.length - shown.size} more`, false));
  }
  if (otherVisits) {
    rows.push(summaryRow(allVisits ? 'This visit only' : `${otherVisits} on other visits`, false, () => { allVisits = !allVisits; }));
  }
  $('#route').replaceChildren(...rows);
  markHeading();
  syncStepControls();
}

/** Where the compass points, as a step's target label; null when it has no target. */
let guideTarget: string | null = null;
/**
 * The bright arrow marks the step to do now: the one the compass points to, else
 * the first open step that is not optional.
 */
function markHeading() {
  const open = Array.from(document.querySelectorAll<HTMLElement>('#route .step.objective:not(.done)'));
  const heading = open.find((row) => guideTarget !== null && row.dataset.target === guideTarget)
    ?? open.find((row) => !row.classList.contains('optional'))
    ?? open[0];
  for (const row of open) row.classList.toggle('heading', row === heading);
}
function setGuideTarget(target: string | null) {
  if (target === guideTarget) return;
  guideTarget = target;
  markHeading();
}

/** A click on a step marks it done or not done, locked or not; the arrow then heads for
 *  the first step that is still open. */
function syncStepControls() {
  document.querySelectorAll<HTMLButtonElement>('#route button.step').forEach((row) => {
    row.disabled = false;
    if (!row.classList.contains('summary')) row.title = 'Click: done / not done';
  });
  syncInput();
}

// ---- Live minimap ----
const minimapBox = $('#minimap');
const minimapCanvas = $<HTMLCanvasElement>('#minimap-canvas');
let minimapTracking = false;
function renderMinimap(view: MinimapView) {
  minimapTracking = view.enabled && view.visible && view.player !== null;
  minimapBox.hidden = !view.enabled;
  if (!minimapTracking) {
    minimapCanvas.getContext('2d')?.clearRect(0, 0, minimapCanvas.width, minimapCanvas.height);
  }
}

const POI_COLORS: Record<string, string> = { arrived: '#7fd18b', start: '#7fd18b', left: '#8edce8', npc: '#f2d36b' };
// ---- The recognised layout ----
const layoutBox = $('#layout-pick');
const layoutCanvas = $<HTMLCanvasElement>('#layout-canvas');
/** Draws the layout the app recognised under the checklist, or says that it does not
 *  know it yet. Up on screen is up on the in-game map. */
function renderPicture(picture: LayoutPicture | undefined) {
  layoutBox.hidden = !picture;
  if (!picture) return;
  const name = $('#layout-name');
  const context = layoutCanvas.getContext('2d');
  context?.clearRect(0, 0, layoutCanvas.width, layoutCanvas.height);
  layoutCanvas.hidden = !picture.known;
  if (!picture.known) {
    name.textContent = picture.noFit
      ? 'Layout not known: the exits seen fit none of the zone\'s layouts'
      : `Layout not known yet · ${picture.candidates} possible`;
    return;
  }
  name.textContent = `Layout ${picture.id} · ${picture.sure ? 'sure' : `likely, ${Math.round(picture.share * 100)}%`}`;
  if (!context) return;
  const points = [...picture.paths.flat(), ...picture.exits.map(([, at]) => at), ...(picture.player ? [picture.player] : [])];
  const xs = points.map(([x]) => x), ys = points.map(([, y]) => y);
  const [x0, x1, y0, y1] = [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)];
  const pad = 40;
  const scale = Math.min((layoutCanvas.width - 2 * pad) / Math.max(x1 - x0, 1), (layoutCanvas.height - 2 * pad) / Math.max(y1 - y0, 1));
  const ox = (layoutCanvas.width - (x1 - x0) * scale) / 2, oy = (layoutCanvas.height - (y1 - y0) * scale) / 2;
  const at = ([x, y]: [number, number]): [number, number] => [ox + (x - x0) * scale, oy + (y - y0) * scale];
  context.lineCap = 'round';
  context.lineJoin = 'round';
  context.strokeStyle = picture.sure ? 'rgb(185 180 230 / .9)' : 'rgb(185 180 230 / .55)';
  context.lineWidth = 7;
  for (const path of picture.paths) {
    context.beginPath();
    path.forEach((point, i) => (i ? context.lineTo(...at(point)) : context.moveTo(...at(point))));
    context.stroke();
  }
  context.font = '600 20px "Segoe UI", system-ui, sans-serif';
  context.textAlign = 'center';
  for (const [exit, point] of picture.exits) {
    const [x, y] = at(point);
    context.fillStyle = '#ff9a3c';
    context.fillRect(x - 7, y - 7, 14, 14);
    context.fillStyle = '#efe9ff';
    context.fillText(exit, Math.min(Math.max(x, 70), layoutCanvas.width - 70), y > 40 ? y - 14 : y + 32);
  }
  if (picture.player) {
    const [x, y] = at(picture.player);
    context.fillStyle = '#ffd27a';
    context.beginPath();
    context.arc(x, y, 9, 0, Math.PI * 2);
    context.fill();
  }
}

function renderLayout(view: LayoutView | null) {
  // With the compass parked the bright arrow marks the first open step, not its target.
  setGuideTarget(compassEnabled.checked ? view?.guide?.target || null : null);
  if (view?.light) return;
  renderPicture(view?.layout);
  const context = minimapCanvas.getContext('2d');
  if (!context) return;
  context.clearRect(0, 0, minimapCanvas.width, minimapCanvas.height);
  if (!view) return;
  const scale = Math.min(minimapCanvas.width / view.cols, minimapCanvas.height / view.rows);
  context.fillStyle = '#b9b4e6';
  for (const [x, y] of view.walls) context.fillRect(x * scale, y * scale, Math.max(scale, 1.5), Math.max(scale, 1.5));
  if (view.trail.length > 1) {
    context.strokeStyle = 'rgb(255 138 42 / .45)';
    context.lineWidth = 1.5;
    context.beginPath();
    view.trail.forEach(([x, y], i) => (i ? context.lineTo(x * scale, y * scale) : context.moveTo(x * scale, y * scale)));
    context.stroke();
  }
  for (const poi of view.pois) {
    context.fillStyle = POI_COLORS[poi.kind] ?? '#fff';
    context.fillRect(poi.x * scale - 3, poi.y * scale - 3, 6, 6);
  }
  const [x, y] = view.player;
  if (view.guide && minimapTracking && view.guide.degrees !== null) {
    // The arrow's direction from the player, on the map.
    const r = (view.guide.degrees * Math.PI) / 180;
    context.strokeStyle = '#ffd27a';
    context.lineWidth = 2;
    context.beginPath();
    context.moveTo(x * scale, y * scale);
    context.lineTo(x * scale + Math.cos(r) * 40, y * scale + Math.sin(r) * 40);
    context.stroke();
  }
  context.fillStyle = '#ff8a2a';
  context.beginPath();
  context.arc(x * scale, y * scale, 4, 0, Math.PI * 2);
  context.fill();
}
function setSettingsOpen(open: boolean) {
  const wasOpen = !settings.hidden;
  settings.hidden = !open;
  settingsToggle.setAttribute('aria-expanded', String(open));
  syncInput();
  if (open || !wasOpen) return;
  stopRecording();
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

// ---- Switches kept between starts ----
function stored(key: string, fallback: boolean) {
  try { return (localStorage.getItem(key) ?? String(fallback)) === 'true'; } catch { return fallback; }
}
function keep(key: string, value: boolean) {
  try { localStorage.setItem(key, String(value)); } catch { /* Optional preference. */ }
}
// Optional content: off is the speedrun default (optional steps not listed, the arrow
// passes over them); on lists them and lets the arrow lead to them.
optionalContent.checked = stored('optional-content', false);
const sendCompassOptional = () =>
  isTauri() && void invoke('set_compass_optional', { follow: optionalContent.checked }).catch(console.error);
optionalContent.addEventListener('change', () => {
  keep('optional-content', optionalContent.checked);
  sendCompassOptional();
  if (lastTracker) renderTracker(lastTracker);
});
const autoHide = $<HTMLInputElement>('#auto-hide');
autoHide.checked = stored('auto-hide', true);
const sendAutoHide = () => isTauri() && void invoke('set_auto_hide', { enabled: autoHide.checked }).catch(console.error);
autoHide.addEventListener('change', () => { keep('auto-hide', autoHide.checked); sendAutoHide(); });
// The compass is parked (roadmap, 2026-11-12): off unless switched on here. The compass
// window reads the same stored value at start and follows this event.
const compassEnabled = $<HTMLInputElement>('#compass-enabled');
compassEnabled.checked = stored('compass-enabled', false);
const sendCompassEnabled = () => isTauri() && void emit('compass-enabled', compassEnabled.checked).catch(console.error);
compassEnabled.addEventListener('change', () => { keep('compass-enabled', compassEnabled.checked); sendCompassEnabled(); });
const fightHide = $<HTMLInputElement>('#fight-hide');
fightHide.checked = stored('fight-hide', true);
const sendFightHide = () => isTauri() && void invoke('set_fight_hide', { enabled: fightHide.checked }).catch(console.error);
fightHide.addEventListener('change', () => { keep('fight-hide', fightHide.checked); sendFightHide(); });
const mapPreview = $<HTMLInputElement>('#map-preview');
mapPreview.checked = stored('map-preview', false);
const showMapPreview = () => { document.body.dataset.mapPreview = String(mapPreview.checked); };
showMapPreview();
mapPreview.addEventListener('change', () => { keep('map-preview', mapPreview.checked); showMapPreview(); });

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

// ---- Window size and the clickable areas ----
const panel = $('.panel');
const header = $('#drag');
let lastSize = '';
let lastRects = '';
/**
 * Only these areas take the mouse; the rest of the panel is click-through to the game.
 * Locked: the lock button and the checklist's rows (a click ticks a step). Unlocked:
 * the whole top bar and the open menu as well.
 */
function syncInput() {
  if (!isTauri() || !status) return;
  const rows = Array.from(document.querySelectorAll<HTMLElement>('#route button.step'));
  const areas = [...(status.interactive ? [header, ...(settings.hidden ? [] : [settings])] : [lockButton]), ...rows];
  const rects = areas.map((element) => {
    const box = element.getBoundingClientRect();
    return { x: box.left, y: box.top, width: box.width, height: box.height };
  });
  const json = JSON.stringify(rects);
  if (json === lastRects) return;
  lastRects = json;
  void invoke('set_input_rects', { rects }).catch(console.error);
}
function fitWindow() {
  if (!isTauri()) return;
  const boxes = [panel, settings].filter((element) => !element.hidden).map((element) => element.getBoundingClientRect());
  const width = Math.ceil(Math.max(...boxes.map((box) => box.right))) + 1;
  const height = Math.ceil(Math.max(...boxes.map((box) => box.bottom))) + 1; // body padding
  if (`${width}x${height}` !== lastSize) {
    lastSize = `${width}x${height}`;
    // Rust picks the right way: GTK sizing for a layer surface, a window resize elsewhere.
    void invoke('resize_overlay', { width, height }).catch(console.error);
  }
  syncInput();
}
const observer = new ResizeObserver(fitWindow);
observer.observe(panel);
observer.observe(settings);

// ---- Dragging the top bar (unlocked) ----
// A layer surface can't be dragged by the compositor. On Hyprland the app follows the
// real cursor (smooth). Elsewhere the overlay moves itself by pointer deltas: each move
// shifts the window under the pointer by the same amount, so the pointer's offset from
// where the drag started is the distance still to go.
let drag: { native: boolean; x: number; y: number; busy: boolean } | null = null;
header.addEventListener('mousedown', (event) => {
  if ((event.target as Element).closest('button')) return;
  if (!status?.interactive || event.button !== 0 || !isTauri()) return;
  if (!status.layerShell) {
    void getCurrentWindow().startDragging().catch(console.error);
    return;
  }
  const current = { native: false, x: event.clientX, y: event.clientY, busy: true };
  drag = current;
  void invoke<boolean>('start_overlay_drag')
    .then((native) => { current.native = native; })
    .catch(console.error)
    .finally(() => { current.busy = false; });
});
document.addEventListener('mousemove', (event) => {
  if (!drag || drag.native || drag.busy) return;
  const dx = Math.round(event.clientX - drag.x), dy = Math.round(event.clientY - drag.y);
  if (!dx && !dy) return;
  const current = drag;
  current.busy = true;
  void invoke('move_overlay_by', { dx, dy })
    .catch(console.error)
    // Let the compositor apply the move before measuring again.
    .finally(() => requestAnimationFrame(() => { current.busy = false; }));
});
function endDrag() {
  if (!drag) return;
  drag = null;
  // end_overlay_drag also saves the position; harmless when the drag wasn't native.
  void invoke('end_overlay_drag').catch(console.error);
}
document.addEventListener('mouseup', endDrag);
window.addEventListener('blur', endDrag);
lockButton.addEventListener('click', () => void call('set_interaction', { interactive: !status?.interactive }));
// Settings and close only take clicks while unlocked (see syncInput).
settingsToggle.addEventListener('click', () => {
  if (status?.interactive) setSettingsOpen(settings.hidden);
});
document.addEventListener('mousedown', (event) => {
  const target = event.target as Element;
  if (!settings.hidden && !settings.contains(target) && !settingsToggle.contains(target)) setSettingsOpen(false);
});
// ---- Restart with the latest version (pull, build, replace this process) ----
const restartButton = $<HTMLButtonElement>('#restart');
function renderRestart(step: string, error: string | null) {
  restartButton.classList.toggle('busy', !!step);
  const box = $('#restart-status');
  box.classList.toggle('failed', !!error);
  box.textContent = error ? `Restart failed: ${error}` : step;
  box.hidden = !step && !error;
}
restartButton.addEventListener('click', () => {
  if (!status?.interactive || restartButton.classList.contains('busy')) return;
  renderRestart('Starting update…', null);
  void invoke('restart_latest').catch((error) => renderRestart('', String(error)));
});
$('#exit').addEventListener('click', () => { if (status?.interactive) void invoke('quit').catch(console.error); });
$('#reset-progress').addEventListener('click', () => {
  void invoke<TrackerView>('reset_progress').then(renderTracker).catch((error) => showMessage('#error', String(error)));
});

// Shown only in a plain browser (npm run dev without Tauri), where no log can be read.
const browserPreview: TrackerView = {
  zone: { id: 'G1_4', name: 'The Grelwood', level: 4 },
  currentArea: null,
  visit: 1,
  log: { connected: true, path: null, message: '' },
  steps: [
    {"id": "grelwood-talk-una", "kind": "objective", "parts": [{"text": "Find and talk to "}, {"text": "Una", "style": "npc"}], "optional": false, "target": null, "state": "done", "source": "log", "visit": 0},
    {"id": "grelwood-waypoint", "kind": "objective", "parts": [{"text": "Take the waypoint"}], "optional": false, "target": null, "state": "done", "source": "log", "visit": 0},
    {"id": "grelwood-grim-tangle-wp", "kind": "objective", "parts": [{"text": "Grim Tangle WP, return"}], "optional": false, "target": "The Grim Tangle", "state": "done", "source": "log", "visit": 0},
    {"id": "grelwood-red-vale-wp", "kind": "objective", "parts": [{"text": "Red Vale WP, return"}], "optional": false, "target": "The Red Vale", "state": "open", "source": null, "visit": 1},
    {"id": "kill-gerung", "kind": "objective", "parts": [{"text": "Kill "}, {"text": "Gerung", "style": "boss"}, {"text": "Skill Gem", "style": "reward"}], "optional": true, "target": "Gerung", "state": "manual", "source": null, "visit": 2},
    {"id": "kill-areagne", "kind": "objective", "parts": [{"text": "Kill "}, {"text": "Areagne", "style": "boss"}, {"text": " at the hut"}, {"text": "Support Gem", "style": "reward"}], "optional": true, "target": "Areagne's hut", "state": "manual", "source": null, "visit": 2},
    {"id": "directions-grelwood", "kind": "note", "parts": [{"text": "Go upstream along the river to The Red Vale", "style": "direction"}], "optional": false, "target": null, "state": "note", "source": null, "visit": 2},
    {"id": "enter-red-vale", "kind": "objective", "parts": [{"text": "Enter The Red Vale"}], "optional": false, "target": "The Red Vale", "state": "open", "source": null, "visit": 2},
    {"id": "runic-seals", "kind": "objective", "parts": [{"text": "Activate 3 Runic Seals"}], "optional": false, "target": null, "state": "manual", "source": null, "visit": 3},
    {"id": "grelwood-una-seals", "kind": "objective", "parts": [{"text": "Talk to "}, {"text": "Una", "style": "npc"}], "optional": false, "target": null, "state": "manual", "source": null, "visit": 3},
    {"id": "grelwood-portal-town", "kind": "objective", "parts": [{"text": "Portal to town"}], "optional": false, "target": null, "state": "manual", "source": null, "visit": 3},
  ],
};

// The webview has no visible console: page errors go to the app's terminal.
function report(message: string) {
  if (isTauri()) void invoke('client_log', { message }).catch(() => {});
}
window.addEventListener('error', (event) => report(`error: ${event.message} (${event.filename}:${event.lineno})`));
window.addEventListener('unhandledrejection', (event) => report(`unhandled rejection: ${String(event.reason)}`));

async function start() {
  if (!isTauri()) {
    render({ visible: true, interactive: true, toggleShortcut: 'Ctrl+Space', shortcutError: null, lastError: null, layerShell: false });
    renderTracker(browserPreview);
    // A real layout of Clearfell (ClearFell_W_E_N), to see the drawing in a browser.
    renderPicture(JSON.parse('{"known":true,"id":"ClearFell_W_E_N","share":0.93,"sure":true,"candidates":1,"paths":[[[-7481,-16101],[-81,-1090]],[[13466,-21338],[-81,-1090]],[[15710,-23582],[13466,-21338]],[[-14588,-17971],[-17581,-23956]],[[-6359,-32185],[-10848,-21712]],[[-3367,-26949],[-10848,-21712]],[[-14588,-33682],[-19825,-26200],[-17581,-23956]],[[-6359,-32185],[-1496,-34056]],[[-1496,-34056],[2992,-38544]],[[17337,-17597],[13466,-21338]],[[-6359,-32185],[-10100,-35926]],[[-5611,-38170],[-1496,-34056]],[[-10100,-35926],[-14588,-33682]],[[-20947,-21338],[-17581,-23956]],[[-10100,-35926],[-5611,-38170]],[[-10848,-21712],[-7481,-16101]],[[5611,-22460],[-81,-1090]],[[-6359,-32185],[-3367,-26949]],[[11222,-31063],[15710,-23582]],[[-10848,-21712],[-14588,-17971]],[[5611,-22460],[-7481,-16101]],[[2992,-38544],[11222,-31063]],[[11222,-31063],[-4489,-21338]]],"exits":[["Clearfell Encampment",[-81,-1090]],["The Grelwood",[-14588,-33682]],["Mud Burrow",[11222,-31063]]],"player":[-14588,-33682]}') as LayoutPicture);
    return;
  }
  await listen<{ step: string; error: string | null }>('restart-status', (event) => renderRestart(event.payload.step, event.payload.error));
  await listen<Status>('overlay-status', (event) => render(event.payload));
  await listen<TrackerView>('tracker-state', (event) => renderTracker(event.payload));
  await listen<MinimapView>('minimap-state', (event) => renderMinimap(event.payload));
  renderMinimap(await invoke<MinimapView>('minimap_state'));
  await listen<LayoutView>('layout-state', (event) => renderLayout(event.payload));
  renderLayout(await invoke<LayoutView | null>('layout_state'));
  render(await invoke<Status>('overlay_status'));
  renderTracker(await invoke<TrackerView>('tracker_state'));
  sendCompassOptional();
  sendAutoHide();
  sendFightHide();
  sendCompassEnabled();
  fitWindow();
  // The window is first shown now that the page has something to draw.
  await invoke('page_ready');
}
void start().catch((error) => showMessage('#error', String(error)));
