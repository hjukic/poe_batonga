import { readFile } from 'node:fs/promises';
import { PNG } from 'pngjs';

// Experiment settings for synthetic fixtures, NOT a calibrated PoE 2 profile.
export const PROFILE = Object.freeze({ mapMin: 170, mapSpread: 35, playerMinGreen: 180, playerMaxRed: 120, playerMaxBlue: 140 });
export const LIMITS = Object.freeze({ minMapPixels: 50, maxShift: 16, tolerance: 1, minAgreement: 0.94, minGap: 0.08, exclusionRadius: 4 });

export async function readPng(path) {
  const bytes = await readFile(path);
  if (bytes.length < 24 || bytes.subarray(0, 8).toString('hex') !== '89504e470d0a1a0a') throw Error('Expected a PNG image');
  if (bytes.readUInt32BE(16) * bytes.readUInt32BE(20) > 16_000_000) throw Error('Image exceeds experiment pixel limit');
  return PNG.sync.read(bytes);
}

export function extract(image, { roi = [0, 0, image.width, image.height], profile = PROFILE } = {}) {
  const [left, top, width, height] = roi;
  if (!roi.every(Number.isInteger) || left < 0 || top < 0 || width < 1 || height < 1 || left + width > image.width || top + height > image.height) throw Error('Invalid image ROI');
  if (width > 256 || height > 256) throw Error('Baseline accepts map crops up to 256 x 256; resize/calibrate offline first');
  const map = new Uint8Array(width * height);
  const marker = new Uint8Array(map.length);
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
    const i = ((top + y) * image.width + left + x) * 4;
    const [r, g, b, a] = image.data.subarray(i, i + 4);
    if (a < 128) continue;
    const index = y * width + x;
    if (Math.min(r, g, b) >= profile.mapMin && Math.max(r, g, b) - Math.min(r, g, b) <= profile.mapSpread) map[index] = 1;
    if (g >= profile.playerMinGreen && r <= profile.playerMaxRed && b <= profile.playerMaxBlue) marker[index] = 1;
  }
  const components = [];
  const visited = new Uint8Array(marker.length);
  for (let start = 0; start < marker.length; start++) {
    if (!marker[start] || visited[start]) continue;
    const queue = [start]; visited[start] = 1;
    let sumX = 0, sumY = 0;
    for (let q = 0; q < queue.length; q++) {
      const current = queue[q], x = current % width, y = Math.floor(current / width);
      sumX += x; sumY += y;
      for (const [nx, ny] of [[x - 1, y], [x + 1, y], [x, y - 1], [x, y + 1]]) {
        const next = ny * width + nx;
        if (nx >= 0 && ny >= 0 && nx < width && ny < height && marker[next] && !visited[next]) { visited[next] = 1; queue.push(next); }
      }
    }
    if (queue.length >= 4 && queue.length <= 80) components.push({ x: sumX / queue.length, y: sumY / queue.length, pixels: queue.length });
  }
  const points = [];
  for (let i = 0; i < map.length; i++) if (map[i]) points.push([i % width, Math.floor(i / width)]);
  return { width, height, roi, map, points, players: components };
}

function distanceField(mask, tolerance) {
  const { width, height, map } = mask;
  const distance = new Uint16Array(map.length); distance.fill(65535);
  const queue = [];
  for (let i = 0; i < map.length; i++) if (map[i]) { distance[i] = 0; queue.push(i); }
  for (let q = 0; q < queue.length; q++) {
    const i = queue[q], d = distance[i];
    if (d >= tolerance) continue;
    const x = i % width, y = Math.floor(i / width);
    for (const [nx, ny] of [[x - 1, y], [x + 1, y], [x, y - 1], [x, y + 1]]) {
      const next = ny * width + nx;
      if (nx >= 0 && ny >= 0 && nx < width && ny < height && distance[next] > d + 1) { distance[next] = d + 1; queue.push(next); }
    }
  }
  return distance;
}

export function analyze(observation, templates, limits = LIMITS) {
  const base = { experimentOnly: true, liveNavigationEnabled: false, mapPixels: observation.points.length, playerCandidates: observation.players, rankings: [] };
  const unknown = (reason, extra = {}) => ({ ...base, decision: 'unknown', reason, ...extra });
  if (observation.points.length < limits.minMapPixels) return unknown('insufficient-map-evidence');
  if (observation.players.length !== 1) return unknown(observation.players.length ? 'ambiguous-player-marker' : 'missing-player-marker');
  if (!templates.length) return unknown('no-templates');
  // Compare observed geometry to templates. Unexplored parts of a template are NOT evidence.
  // Fixed scale/orientation only: translation is the sole supported transform in this baseline.
  const stride = Math.max(1, Math.ceil(observation.points.length / 350));
  const points = observation.points.filter((_, i) => i % stride === 0);
  const ranks = [];
  for (const template of templates) {
    const field = distanceField(template.mask, limits.tolerance);
    const { width, height } = template.mask;
    for (let dy = -limits.maxShift; dy <= limits.maxShift; dy++) for (let dx = -limits.maxShift; dx <= limits.maxShift; dx++) {
      let hits = 0, totalDistance = 0;
      for (const [x, y] of points) {
        const tx = x - dx, ty = y - dy;
        const distance = tx >= 0 && ty >= 0 && tx < width && ty < height ? field[ty * width + tx] : 65535;
        if (distance <= limits.tolerance) hits++;
        totalDistance += Math.min(distance, limits.tolerance + 1);
      }
      ranks.push({ templateId: template.id, dx, dy, geometryAgreement: hits / points.length, meanDistance: totalDistance / points.length });
    }
  }
  ranks.sort((a, b) => b.geometryAgreement - a.geometryAgreement || a.meanDistance - b.meanDistance || Math.hypot(a.dx, a.dy) - Math.hypot(b.dx, b.dy));
  const best = ranks[0];
  // Also reject competing positions in the SAME template, not just competing template IDs.
  const alternative = ranks.find((r) => r.templateId !== best.templateId || Math.hypot(r.dx - best.dx, r.dy - best.dy) > limits.exclusionRadius);
  const gap = alternative ? best.geometryAgreement - alternative.geometryAgreement : 1;
  const ranking = { rankings: [best, alternative].filter(Boolean), candidateGap: gap };
  if (best.geometryAgreement < limits.minAgreement) return unknown('poor-geometry-match', ranking);
  if (gap < limits.minGap) return unknown('ambiguous-layout-or-position', ranking);
  const player = observation.players[0];
  return { ...base, ...ranking, decision: 'candidate', reason: 'fixed-scale-geometry-match', match: best,
    playerInTemplate: { x: player.x - best.dx, y: player.y - best.dy } };
}

export function maskPng(mask) {
  const png = new PNG({ width: mask.width, height: mask.height });
  for (let i = 0; i < mask.map.length; i++) png.data.set(mask.map[i] ? [235, 235, 235, 255] : [12, 14, 20, 255], i * 4);
  for (const player of mask.players) {
    const i = (Math.round(player.y) * mask.width + Math.round(player.x)) * 4;
    png.data.set([40, 240, 90, 255], i);
  }
  return PNG.sync.write(png);
}
