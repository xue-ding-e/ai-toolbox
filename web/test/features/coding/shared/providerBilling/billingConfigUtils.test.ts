import assert from 'node:assert/strict';
import test from 'node:test';

import {
  getBillingConfigFromMeta,
  mergeBillingConfigIntoMeta,
  normalizeBillingPricingModelSource,
} from '../../../../../features/coding/shared/providerBilling/billingConfigUtils.ts';

test('normalizeBillingPricingModelSource accepts UI and legacy aliases', () => {
  assert.equal(normalizeBillingPricingModelSource('requested'), 'requested');
  assert.equal(normalizeBillingPricingModelSource('request'), 'requested');
  assert.equal(normalizeBillingPricingModelSource('upstream'), 'upstream');
  assert.equal(normalizeBillingPricingModelSource('response'), 'upstream');
  assert.equal(normalizeBillingPricingModelSource(undefined), 'inherit');
});

test('getBillingConfigFromMeta enables provider billing when meta has custom values', () => {
  assert.deepEqual(getBillingConfigFromMeta({
    costMultiplier: ' 1.50 ',
    pricingModelSource: 'request',
  }), {
    enabled: true,
    costMultiplier: '1.50',
    pricingModelSource: 'requested',
    fastCostMultiplier: undefined,
    ultrafastCostMultiplier: undefined,
  });

  assert.deepEqual(getBillingConfigFromMeta(undefined), {
    enabled: false,
    costMultiplier: undefined,
    pricingModelSource: 'inherit',
    fastCostMultiplier: undefined,
    ultrafastCostMultiplier: undefined,
  });
});

test('getBillingConfigFromMeta keeps service-tier multipliers and enables on them alone', () => {
  assert.deepEqual(getBillingConfigFromMeta({
    fastCostMultiplier: ' 2.5 ',
    ultrafastCostMultiplier: '8',
  }), {
    enabled: true,
    costMultiplier: undefined,
    pricingModelSource: 'inherit',
    fastCostMultiplier: '2.5',
    ultrafastCostMultiplier: '8',
  });
});

test('mergeBillingConfigIntoMeta round-trips service-tier multipliers', () => {
  assert.deepEqual(mergeBillingConfigIntoMeta({
    providerType: 'openai',
    fastCostMultiplier: '9',
    ultrafastCostMultiplier: '9',
  }, {
    enabled: true,
    pricingModelSource: 'inherit',
    fastCostMultiplier: ' 2.5 ',
  }), {
    providerType: 'openai',
    fastCostMultiplier: '2.5',
  });
});

test('mergeBillingConfigIntoMeta clears billing fields when disabled', () => {
  assert.deepEqual(mergeBillingConfigIntoMeta({
    providerType: 'anthropic',
    costMultiplier: '1.5',
    pricingModelSource: 'upstream',
  }, {
    enabled: false,
    pricingModelSource: 'inherit',
  }), {
    providerType: 'anthropic',
  });
});

test('mergeBillingConfigIntoMeta omits inherit pricing source when enabled', () => {
  assert.deepEqual(mergeBillingConfigIntoMeta(undefined, {
    enabled: true,
    costMultiplier: ' 2 ',
    pricingModelSource: 'inherit',
  }), {
    costMultiplier: '2',
  });
});
