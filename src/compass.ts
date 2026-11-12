// The compass: a click-through window centred on the player character. An arrow on a
// ring around the player points the way to the next objective, like a compass needle.
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import './compass.css';

interface GuideView {
  targetKind: string;
  target: string;
  /** Screen direction: 0 = right, 90 = down; null when there is not even a guess. */
  degrees: number | null;
  /** "estimated" (layout confirmed), "rough" (likely) or "unconfirmed" (best guess). */
  source: string;
}
interface LayoutView { guide: GuideView | null }
interface MinimapView { enabled: boolean; visible: boolean; player: [number, number] | null }

/** Ring radius around the player, in CSS pixels (the window is 460 x 460, room for the label). */
const RADIUS = 125;

document.body.innerHTML = `
  <svg id="compass" viewBox="-230 -230 460 460" aria-hidden="true">
    <circle class="ring" r="${RADIUS}"/>
    <g id="needle">
      <!-- A dart: long and narrow with a deep notch, so the tip is unmistakable at any angle. -->
      <path class="head" d="M${RADIUS + 32} 0 L${RADIUS - 12} -13 L${RADIUS - 2} 0 L${RADIUS - 12} 13 Z"/>
    </g>
  </svg>
  <span id="label"></span>`;

const svg = document.querySelector<SVGSVGElement>('#compass')!;
const needle = document.querySelector<SVGGElement>('#needle')!;
const label = document.querySelector<HTMLSpanElement>('#label')!;

let guide: GuideView | null = null;
let tracking = false;
let shown: number | null = null;

/** The needle is always shown while there is a target: gold when the layout is
 *  confirmed, fainter gold when it is likely, gray while it is not recognised, there is
 *  no direction yet, or the minimap is not tracking the player (it then keeps its last
 *  direction). */
function update() {
  svg.classList.toggle('off', !guide);
  label.classList.toggle('off', !guide);
  if (!guide) return;
  const known = guide.degrees !== null && Number.isFinite(guide.degrees);
  svg.classList.toggle('rough', tracking && known && guide.source === 'rough');
  svg.classList.toggle('unsure', !tracking || !known || guide.source === 'unconfirmed');
  label.classList.toggle('unsure', !tracking || !known || guide.source === 'unconfirmed');
  label.textContent = guide.target || (guide.targetKind === 'entrance' ? 'Exit' : guide.targetKind);
}

/** Turns the needle smoothly, always the short way round. */
function frame() {
  const known = guide?.degrees;
  // Without a direction the needle stays where it was (up at first).
  const target = known !== null && known !== undefined && Number.isFinite(known) ? known : (shown ?? -90);
  if (guide) {
    if (shown === null) shown = target;
    const delta = ((target - shown + 540) % 360) - 180;
    shown += Math.abs(delta) < 0.5 ? delta : delta * 0.18;
    needle.setAttribute('transform', `rotate(${shown})`);
    const r = (shown * Math.PI) / 180;
    // The label sits just outside the arrow head.
    label.style.transform = `translate(${Math.cos(r) * (RADIUS + 56)}px, ${Math.sin(r) * (RADIUS + 56)}px) translate(-50%, -50%)`;
  }
  requestAnimationFrame(frame);
}

async function start() {
  if (!isTauri()) {
    guide = { targetKind: 'boss', target: 'Miller', degrees: -40, source: 'rough' };
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
