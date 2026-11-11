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
    <div class="route">
      <div>Talk to the Wounded Man</div>
      <div class="direction">Directions: Follow the river upstream</div>
      <div>Kill <span class="boss">The Bloated Miller</span></div>
      <div>Enter Clearfell Encampment</div>
    </div>
    <div class="edit-tools">
      <div id="settings" hidden>
        <div class="opacity-control"><label for="opacity">Opacity</label><input id="opacity" type="range" min="10" max="90" value="25" /><output id="opacity-value" for="opacity">25%</output></div>
        <label class="sample"><input type="checkbox" id="sample" disabled /> Test overlay input</label>
        <span id="sample-result" class="hint">Preview content · game tracking is not connected.</span>
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
    <div class="buttons"><button id="interaction">Enable interaction</button><button id="visibility-toggle">Hide overlay</button><button id="reset">Recenter & lock</button><button id="quit" class="secondary">Quit</button></div>
    <dl><dt>Ctrl + Alt + I</dt><dd>Toggle interaction</dd><dt>Ctrl + Alt + O</dt><dd>Show / hide (returns to click-through)</dd></dl>
    <p class="hint">In interaction mode, drag the panel header. In click-through mode, mouse input reaches the window underneath. Close this controls window to quit both windows.</p>
    <div id="error" role="alert" hidden></div>
    <div id="shortcuts" role="status"></div>
    <p class="hint">No logs, inventory scan or screen capture are connected in this experiment.</p>
  </div>`;

let status: Status | undefined;
let busy = false;
function render(next: Status) {
  status = next;
  document.body.dataset.interactive = String(next.interactive);
  document.querySelector('#mode')!.textContent = next.interactive ? 'INTERACTION ON' : 'CLICK-THROUGH';
  if (overlay) {
    const checkbox = document.querySelector<HTMLInputElement>('#sample')!;
    checkbox.disabled = !next.interactive;
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
  document.querySelector('#sample')!.addEventListener('change', () => {
    document.querySelector('#sample-result')!.textContent = 'Click received by the overlay. Toggle interaction off to test passing clicks through.';
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
}

async function start() {
  if (!isTauri()) {
    render({ visible: true, interactive: true, shortcutErrors: ['Browser preview only; native window behavior is unavailable.'], lastError: null });
    document.querySelectorAll<HTMLButtonElement>('button').forEach((button) => button.disabled = true);
    return;
  }
  await listen<Status>('overlay-status', (event) => render(event.payload));
  render(await invoke<Status>('overlay_status'));
}
void start().catch((error) => showError(String(error)));
