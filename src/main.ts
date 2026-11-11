import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
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
    <header id="drag"><span>The Riverbank - Zone Level: 1</span><span class="preview-label">Preview</span></header>
    <div class="route">
      <div>Talk to the Wounded Man</div>
      <div class="direction">Directions: Follow the river upstream</div>
      <div>Kill <span class="boss">The Bloated Miller</span></div>
      <div>Enter Clearfell Encampment</div>
    </div>
    <div class="edit-tools">
      <div class="opacity-control"><label for="opacity">Opacity</label><input id="opacity" type="range" min="10" max="90" value="25" /><output id="opacity-value" for="opacity">25%</output></div>
      <label class="sample"><input type="checkbox" id="sample" disabled /> Test overlay input</label>
      <span id="sample-result" class="hint">Preview content · game tracking is not connected.</span>
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
  } else {
    document.querySelector('#visibility')!.textContent = next.visible ? 'Overlay visible' : 'Overlay hidden';
    document.querySelector('#interaction')!.textContent = next.interactive ? 'Lock to click-through' : 'Enable interaction';
    document.querySelector('#visibility-toggle')!.textContent = next.visible ? 'Hide overlay' : 'Show overlay';
    document.querySelector('#shortcuts')!.textContent = next.shortcutErrors.length
      ? `Global shortcuts unavailable: ${next.shortcutErrors.join('; ')}. Use these buttons instead.` : 'Global shortcuts registered. Verify they work while the game is focused.';
    showError(next.lastError);
  }
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
  document.querySelector('#drag')!.addEventListener('mousedown', (event) => {
    if (status?.interactive && (event as MouseEvent).button === 0 && isTauri()) {
      void getCurrentWindow().startDragging().catch(console.error);
    }
  });
  document.querySelector('#sample')!.addEventListener('change', () => {
    document.querySelector('#sample-result')!.textContent = 'Click received by the overlay. Toggle interaction off to test passing clicks through.';
  });
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
