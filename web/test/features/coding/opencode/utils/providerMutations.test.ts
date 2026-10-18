import assert from 'node:assert/strict';
import test from 'node:test';
import type { OpenCodeConfig, OpenCodeProvider } from '../../../../../types/opencode.ts';
import {
  canDeleteOpenCodeProvider,
  overwriteOpenCodeProvider,
  replaceOpenCodeProvider,
  sanitizeOpenCodeModelReferences,
} from '../../../../../features/coding/opencode/utils/providerMutations.ts';

const makeConfig = (): OpenCodeConfig => ({
  provider: {
    relay: {
      npm: '@ai-sdk/openai-compatible',
      options: { baseURL: 'https://old.example/v1', apiKey: 'old-key' },
      models: { 'org/keep': { name: 'Keep' }, 'org/remove': { name: 'Remove' } },
    },
    other: { models: { fallback: { name: 'Fallback' } } },
  },
  model: 'relay/org/keep',
  small_model: 'relay/org/remove',
  agent: {
    build: { model: 'relay/org/keep', variant: 'high', prompt: 'Keep this prompt' },
    helper: { model: 'relay/org/remove', variant: 'low', description: 'Helper', options: { custom: true } },
    reviewer: { model: 'other/fallback', variant: 'medium', description: 'Reviewer' },
  },
});

test('default provider protection uses the provider prefix even when model IDs contain slashes', () => {
  const config = makeConfig();
  assert.equal(canDeleteOpenCodeProvider(config, 'relay'), false);
  assert.equal(canDeleteOpenCodeProvider(config, 'other'), true);
  assert.equal(canDeleteOpenCodeProvider(config, 'missing'), false);
});

test('removing a model clears a default reference carrying a variant', () => {
  const config = makeConfig();
  const saved = sanitizeOpenCodeModelReferences({
    ...config,
    model: 'relay/org/remove#high',
    small_model: 'relay/org/keep#low',
  }, ['relay/org/remove']);
  assert.equal(saved.model, undefined);
  assert.equal(saved.small_model, 'relay/org/keep#low');
});

test('overwriting only connection fields preserves default and Agent model references', () => {
  const config = makeConfig();
  const replacement = {
    ...config.provider.relay,
    options: { baseURL: 'https://new.example/v1', apiKey: 'new-key' },
  };
  const saved: OpenCodeConfig = JSON.parse(JSON.stringify(replaceOpenCodeProvider(config, 'relay', replacement)));
  assert.equal(saved.model, config.model);
  assert.equal(saved.small_model, config.small_model);
  assert.deepEqual(saved.agent, config.agent);
  assert.equal(saved.provider.relay.options?.apiKey, 'new-key');
  assert.equal(config.provider.relay.options?.apiKey, 'old-key');
});

test('replacement clears only removed model references and preserves other Agent fields', () => {
  const config = makeConfig();
  const saved = replaceOpenCodeProvider(config, 'relay', {
    ...config.provider.relay,
    models: { 'org/keep': { name: 'Keep' } },
  });
  assert.equal(saved.model, 'relay/org/keep');
  assert.equal(saved.small_model, undefined);
  assert.deepEqual(saved.agent?.build, config.agent?.build);
  assert.deepEqual(saved.agent?.reviewer, config.agent?.reviewer);
  assert.deepEqual(saved.agent?.helper, { description: 'Helper', options: { custom: true } });
});

test('failed overwrite leaves both runtime config and the favorite import source unchanged', async () => {
  const config = makeConfig();
  const incoming = { ...config.provider.relay, options: { apiKey: 'incoming-key' } };
  let favorite: OpenCodeProvider = incoming;
  let backupCalls = 0;
  await assert.rejects(overwriteOpenCodeProvider({
    config, providerId: 'relay', provider: incoming,
    saveConfig: async () => { throw new Error('write failed'); },
    backupPreviousProvider: async (previous) => { backupCalls++; favorite = previous; },
    onBackupError: () => assert.fail('the backup must not start after a failed runtime save'),
  }), /write failed/);
  assert.equal(config.provider.relay.options?.apiKey, 'old-key');
  assert.equal(favorite.options?.apiKey, 'incoming-key');
  assert.equal(backupCalls, 0);

  const savedOnRetry: OpenCodeConfig[] = [];
  await overwriteOpenCodeProvider({
    config, providerId: 'relay', provider: favorite,
    saveConfig: async (next) => { savedOnRetry.push(JSON.parse(JSON.stringify(next))); },
    backupPreviousProvider: async (previous) => { favorite = previous; },
    onBackupError: () => assert.fail('retry backup should succeed'),
  });
  assert.equal(savedOnRetry[0].provider.relay.options?.apiKey, 'incoming-key');
  assert.equal(savedOnRetry[0].model, config.model);
});

test('successful overwrite persists the incoming config before preserving the previous version', async () => {
  const config = makeConfig();
  const incoming = { ...config.provider.relay, options: { apiKey: 'incoming-key' } };
  let runtime = config;
  let favorite: OpenCodeProvider = incoming;
  await overwriteOpenCodeProvider({
    config, providerId: 'relay', provider: incoming,
    saveConfig: async (next) => { runtime = JSON.parse(JSON.stringify(next)); },
    backupPreviousProvider: async (previous) => {
      assert.equal(runtime.provider.relay.options?.apiKey, 'incoming-key');
      favorite = previous;
    },
    onBackupError: () => assert.fail('backup should succeed'),
  });
  assert.equal(runtime.provider.relay.options?.apiKey, 'incoming-key');
  assert.equal(favorite.options?.apiKey, 'old-key');
});

test('a post-save backup failure is reported without turning the saved overwrite into a failure', async () => {
  const config = makeConfig();
  const incoming = { ...config.provider.relay, options: { apiKey: 'incoming-key' } };
  let runtime = config;
  const errors: unknown[] = [];
  await overwriteOpenCodeProvider({
    config, providerId: 'relay', provider: incoming,
    saveConfig: async (next) => { runtime = next; },
    backupPreviousProvider: async () => { throw new Error('backup failed'); },
    onBackupError: (error) => errors.push(error),
  });
  assert.equal(runtime.provider.relay.options?.apiKey, 'incoming-key');
  assert.equal(errors.length, 1);
});
