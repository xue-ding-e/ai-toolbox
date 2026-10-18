import assert from 'node:assert/strict';
import test from 'node:test';
import {
  parseOpenCodeModelReference,
  stripOpenCodeModelVariant,
} from '../../../../../features/coding/opencode/utils/modelReference.ts';

test('parses provider, slash-containing model id, and variant', () => {
  assert.deepEqual(parseOpenCodeModelReference('relay/org/keep#high'), {
    providerId: 'relay',
    modelId: 'org/keep',
    variant: 'high',
  });
  assert.deepEqual(parseOpenCodeModelReference('relay/org/keep'), {
    providerId: 'relay',
    modelId: 'org/keep',
    variant: undefined,
  });
});

test('rejects references without a provider or model', () => {
  assert.equal(parseOpenCodeModelReference(undefined), undefined);
  assert.equal(parseOpenCodeModelReference('   '), undefined);
  assert.equal(parseOpenCodeModelReference('relay'), undefined);
  assert.equal(parseOpenCodeModelReference('/model'), undefined);
  assert.equal(parseOpenCodeModelReference('relay/'), undefined);
  assert.equal(parseOpenCodeModelReference('relay/#high'), undefined);
});

test('stripOpenCodeModelVariant removes only the hash suffix', () => {
  assert.equal(stripOpenCodeModelVariant('relay/model#high'), 'relay/model');
  assert.equal(stripOpenCodeModelVariant('relay/model'), 'relay/model');
  assert.equal(stripOpenCodeModelVariant('relay/org/model#max'), 'relay/org/model');
});
