import assert from 'node:assert/strict';
import test from 'node:test';

import {
  emptyServiceTierOverrideEntry,
  getServiceTierOverridesFromMeta,
  mergeServiceTierOverridesIntoMeta,
} from '../../../../../features/coding/shared/providerServiceTiers/serviceTierOverridesUtils.ts';

test('getServiceTierOverridesFromMeta reports enabled when a non-empty array is present', () => {
  assert.deepEqual(
    getServiceTierOverridesFromMeta({
      modelServiceTiers: [{ model: 'gpt-5.6-sol', serviceTier: 'priority' }],
    }),
    {
      enabled: true,
      overrides: [{ model: 'gpt-5.6-sol', serviceTier: 'priority' }],
    },
  );
});

test('getServiceTierOverridesFromMeta reads snake_case model_service_tiers array', () => {
  const state = getServiceTierOverridesFromMeta({
    model_service_tiers: [{ model: 'gpt-5.6-sol', serviceTier: 'ultrafast' }],
  } as never);
  assert.equal(state.enabled, true);
  assert.deepEqual(state.overrides, [
    { model: 'gpt-5.6-sol', serviceTier: 'ultrafast' },
  ]);
});

test('getServiceTierOverridesFromMeta reports disabled and seeds a blank row when unset', () => {
  const fromUndefined = getServiceTierOverridesFromMeta(undefined);
  assert.equal(fromUndefined.enabled, false);
  assert.equal(fromUndefined.overrides.length, 1);
  assert.deepEqual(fromUndefined.overrides[0], emptyServiceTierOverrideEntry());

  const fromEmpty = getServiceTierOverridesFromMeta({ modelServiceTiers: [] });
  assert.equal(fromEmpty.enabled, false);
  assert.equal(fromEmpty.overrides.length, 1);
});

test('mergeServiceTierOverridesIntoMeta round-trips enabled rules and trims the model', () => {
  const state = {
    enabled: true,
    overrides: [
      { model: ' gpt-5.6-sol ', serviceTier: 'priority' },
      { model: '', serviceTier: 'ultrafast' },
      { model: ' gpt-5.6-astra ', serviceTier: '' },
    ],
  };
  const meta = mergeServiceTierOverridesIntoMeta(undefined, state);
  assert.deepEqual(meta, {
    modelServiceTiers: [{ model: 'gpt-5.6-sol', serviceTier: 'priority' }],
  });

  const readBack = getServiceTierOverridesFromMeta(meta);
  assert.equal(readBack.enabled, true);
  assert.deepEqual(readBack.overrides, [
    { model: 'gpt-5.6-sol', serviceTier: 'priority' },
  ]);
});

test('mergeServiceTierOverridesIntoMeta drops the key when disabled or blank', () => {
  const disabled = mergeServiceTierOverridesIntoMeta(
    { modelServiceTiers: [{ model: 'a', serviceTier: 'priority' }] },
    { enabled: false, overrides: [{ model: 'a', serviceTier: 'priority' }] },
  );
  assert.equal(disabled, undefined);

  const blankRows = mergeServiceTierOverridesIntoMeta(
    undefined,
    { enabled: true, overrides: [{ model: '  ', serviceTier: 'priority' }] },
  );
  assert.equal(blankRows, undefined);
});

test('mergeServiceTierOverridesIntoMeta keeps unrelated meta keys from other subsets', () => {
  const meta = mergeServiceTierOverridesIntoMeta(
    {
      modelRewrites: [{ from: 'a', to: 'b' }],
    } as never,
    { enabled: true, overrides: [{ model: 'gpt-5.6-sol', serviceTier: 'priority' }] },
  );
  assert.deepEqual(
    (meta as never as { modelServiceTiers: unknown }).modelServiceTiers,
    [{ model: 'gpt-5.6-sol', serviceTier: 'priority' }],
  );
  assert.deepEqual(
    (meta as never as { modelRewrites: unknown }).modelRewrites,
    [{ from: 'a', to: 'b' }],
  );
});