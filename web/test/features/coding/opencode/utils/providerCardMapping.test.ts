import assert from 'node:assert/strict';
import test from 'node:test';
import type { OpenCodeProvider } from '../../../../../types/opencode.ts';
import {
  buildOpenCodeModelRows,
  buildOpenCodeProviderCardModel,
  resolveOpenCodeDefaultProviderId,
} from '../../../../../features/coding/opencode/utils/providerCardMapping.ts';

const provider = (overrides: Partial<OpenCodeProvider> = {}): OpenCodeProvider => ({
  models: {},
  ...overrides,
});

test('the card header falls back to the id, the openai-compatible npm and an empty endpoint', () => {
  assert.deepEqual(buildOpenCodeProviderCardModel('relay', provider()), {
    id: 'relay',
    name: 'relay',
    sdkName: '@ai-sdk/openai-compatible',
    baseUrl: '',
  });
});

test('the card header keeps the configured name, npm and endpoint', () => {
  assert.deepEqual(
    buildOpenCodeProviderCardModel('relay', provider({
      name: 'Relay',
      npm: '@ai-sdk/anthropic',
      options: { baseURL: 'https://relay.example/v1' },
    })),
    {
      id: 'relay',
      name: 'Relay',
      sdkName: '@ai-sdk/anthropic',
      baseUrl: 'https://relay.example/v1',
    },
  );
});

test('model rows carry limits and mark the default row through the parsed reference', () => {
  const rows = buildOpenCodeModelRows(
    'relay',
    provider({
      models: {
        'openai/gpt-5.5': { name: 'GPT 5.5', limit: { context: 400000, output: 128000 } },
        plain: {},
      },
    }),
    { providerId: 'relay', modelId: 'openai/gpt-5.5' },
  );

  assert.deepEqual(rows, [
    {
      id: 'openai/gpt-5.5',
      name: 'GPT 5.5',
      contextLimit: 400000,
      outputLimit: 128000,
      isPrimary: true,
    },
    { id: 'plain', name: 'plain', contextLimit: undefined, outputLimit: undefined, isPrimary: false },
  ]);
});

test('a variant suffix does not change which row is the default', () => {
  const rows = buildOpenCodeModelRows(
    'relay',
    provider({ models: { fast: { name: 'Fast' } } }),
    { providerId: 'relay', modelId: 'fast', variant: 'high' },
  );
  assert.equal(rows[0]?.isPrimary, true);
});

test('another provider owning the same model id does not mark this card', () => {
  const rows = buildOpenCodeModelRows(
    'relay',
    provider({ models: { fast: {} } }),
    { providerId: 'other', modelId: 'fast' },
  );
  assert.equal(rows[0]?.isPrimary, false);
});

test('the default provider is resolved from the stored reference', () => {
  assert.equal(resolveOpenCodeDefaultProviderId('relay/openai/gpt-5.5#high'), 'relay');
  assert.equal(resolveOpenCodeDefaultProviderId(undefined), '');
  assert.equal(resolveOpenCodeDefaultProviderId(''), '');
  assert.equal(resolveOpenCodeDefaultProviderId('/missing-provider'), '');
});
