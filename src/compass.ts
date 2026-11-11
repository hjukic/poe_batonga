// The compass: a click-through window centred on the player character. An arrow on a
// ring around the player points the way to the next objective, like a compass needle.
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import './compass.css';

interface GuideView {
  targetKind: string;
  target: string;
  /** Screen direction: 0 = right, 90 = down; null while not confirmed. */
  degrees: number | null;
}
interface LayoutView { guide: GuideView | null }
interface MinimapView { enabled: boolean; visible: boolean; player: [number, number] | null }

/** Ring radius around the player, in CSS pixels (the window is 360 x 360). */
const RADIUS = 125;

document.body.innerHTML = `
  <svg id="compass" viewBox="-180 -180 360 360" aria-hidden="true">
    <circle class="ring" r="${RADIUS}"/>
    <g id="needle">
      <path class="head" d="M${RADIUS + 22} 0 L${RADIUS - 6} -17 L${RADIUS + 2} 0 L${RADIUS - 6} 17 Z"/>
    </g>
  </svg>
  <span id="label"></span>`;

const svg = document.querySelector<SVGSVGElement>('#compass')!;
const needle = document.querySelector<SVGGElement>('#needle')!;
const label = document.querySelector<HTMLSpanElement>('#label')!;

let guide: GuideView | null = null;
let tracking = false;
let shown: number | null = null;

function update() {
  const target = tracking && guide && guide.degrees !== null && Number.isFinite(guide.degrees) ? guide.degrees : null;
  svg.classList.toggle('off', target === null);
  label.classList.toggle('off', target === null);
  if (target === null || !guide) return;
  label.textContent = guide.target || (guide.targetKind === 'entrance' ? 'Exit' : guide.targetKind);
}

/** Turns the needle smoothly, always the short way round. */
function frame() {
  const target = guide?.degrees;
  if (tracking && target !== null && target !== undefined && Number.isFinite(target)) {
    if (shown === null) shown = target;
    const delta = ((target - shown + 540) % 360) - 180;
    shown += Math.abs(delta) < 0.5 ? delta : delta * 0.18;
    needle.setAttribute('transform', `rotate(${shown})`);
    const r = (shown * Math.PI) / 180;
    // The label sits just outside the arrow head.
    label.style.transform = `translate(${Math.cos(r) * (RADIUS + 48)}px, ${Math.sin(r) * (RADIUS + 48)}px) translate(-50%, -50%)`;
  }
  requestAnimationFrame(frame);
}

async function start() {
  if (!isTauri()) {
    guide = { targetKind: 'boss', target: 'Miller', degrees: -40 };
    tracking = true;
  } else {
    await listen<LayoutView | null>('layout-state', (event) => { guide = event.payload?.guide ?? null; update(); });
    await listen<MinimapView>('minimap-state', (event) => {
      tracking = event.payload.enabled && event.payload.visible && event.payload.player !== null;
      update();
    });
    guide = (await invoke<LayoutView | null>('layout_state'))?.guide ?? null;
    const minimap = await invoke<MinimapView>('minimap_state');
    tracking = minimap.enabled && minimap.visible && minimap.player !== null;
  }
  update();
  requestAnimationFrame(frame);
}
void start().catch(console.error);
