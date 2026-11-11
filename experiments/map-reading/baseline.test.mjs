import test from 'node:test';
import assert from 'node:assert/strict';
import { extract, analyze } from './baseline.mjs';
import { alpha, beta, scene, cases } from './fixtures.mjs';

const templates = [{ id: 'alpha', mask: extract(scene({ segments: alpha, players: [] })) }, { id: 'beta', mask: extract(scene({ segments: beta, players: [] })) }];
for (const sample of cases) test(sample.id, () => {
  const result = analyze(extract(sample.image), templates);
  assert.equal(result.decision, sample.expected);
  assert.equal(result.liveNavigationEnabled, false);
  if (sample.reason) assert.equal(result.reason, sample.reason);
  if (sample.templateId) {
    assert.equal(result.match.templateId, sample.templateId);
    assert.ok(Math.hypot(result.playerInTemplate.x - sample.player[0], result.playerInTemplate.y - sample.player[1]) <= 2);
  }
});
test('duplicate templates must abstain', () => {
  const result = analyze(extract(cases[0].image), [templates[0], { ...templates[0], id: 'duplicate' }]);
  assert.equal(result.reason, 'ambiguous-layout-or-position');
});
test('repeated positions within one template must abstain', () => {
  const repeated = [[10, 20, 40, 20], [10, 20, 10, 70], [70, 20, 100, 20], [70, 20, 70, 70]];
  const observation = extract(scene({ segments: repeated.slice(0, 2), players: [[10, 40]] }));
  const template = { id: 'repeated', mask: extract(scene({ segments: repeated, players: [] })) };
  assert.equal(analyze(observation, [template], { minMapPixels: 50, maxShift: 64, tolerance: 1, minAgreement: .94, minGap: .08, exclusionRadius: 4 }).reason, 'ambiguous-layout-or-position');
});
test('out-of-bounds crop is rejected', () => {
  assert.throws(() => extract(cases[0].image, { roi: [100, 100, 50, 50] }), /Invalid image ROI/);
});
