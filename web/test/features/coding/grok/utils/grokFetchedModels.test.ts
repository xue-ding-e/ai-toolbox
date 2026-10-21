/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import {
  buildFetchedGrokCatalogModel,
  findGrokPresetForModel,
  mergeFetchedGrokCatalogModels,
} from '../../../../../features/coding/grok/utils/grokFetchedModels.ts';
import { updatePresetModels } from '../../../../../constants/presetModels.ts';

const bundledPresets = JSON.parse(
  readFileSync(
    new URL('../../../../../../tauri/resources/preset_models.json', import.meta.url),
    'utf8',
  ),
);

const channelDefaults = { baseUrl: 'https://relay.example.test/v1', apiBackend: 'chat_completions' };

test('a fetched model known to the xAI preset group keeps the upstream id and gains its metadata', () => {
  updatePresetModels(bundledPresets);

  const row = buildFetchedGrokCatalogModel(
    { id: 'grok-4.7', name: 'API display name' },
    findGrokPresetForModel('grok-4.7', 'openai_chat'),
    channelDefaults,
  );

  assert.equal(row.key, 'grok-4.7');
  assert.equal(row.model, 'grok-4.7');
  // The preset's own name wins over the provider's `/models` label: that label
  // is the same id at best, and the preset is the curated display name.
  assert.equal(row.displayName, 'Grok 4.7');
  assert.equal(row.contextWindow, 500_000);
  assert.equal(row.supportsReasoningEffort, true);
  assert.deepEqual(row.reasoningEfforts, ['low', 'medium', 'high', 'xhigh']);
  assert.equal(row.reasoningEffort, 'high');
  assert.deepEqual(row.modalities, { input: ['text', 'image'], output: ['text'] });
  assert.equal(row.attachment, true);
  assert.equal(row.supportsImage, true);
  assert.equal(row.baseUrl, channelDefaults.baseUrl);
  assert.equal(row.apiBackend, channelDefaults.apiBackend);
  // `maxCompletionTokens` stays out of the catalog: the bundled xAI presets
  // declare an output limit equal to the context window, and the model editor
  // has no field for it.
  assert.equal(row.maxCompletionTokens, undefined);
});

test('preset matching is case-insensitive but never rewrites the upstream id', () => {
  updatePresetModels(bundledPresets);

  const preset = findGrokPresetForModel('GROK-4.7', 'openai_chat');
  assert.equal(preset?.id, 'grok-4.7');

  const row = buildFetchedGrokCatalogModel({ id: 'GROK-4.7' }, preset, channelDefaults);
  assert.equal(row.key, 'GROK-4.7');
  assert.equal(row.model, 'GROK-4.7');
});

test('a model with no preset writes only the upstream facts', () => {
  updatePresetModels(bundledPresets);

  const row = buildFetchedGrokCatalogModel(
    { id: 'vendor/private-model', name: 'Private Model' },
    findGrokPresetForModel('vendor/private-model', 'openai_chat'),
    channelDefaults,
  );

  assert.deepEqual(row, {
    key: 'vendor/private-model',
    model: 'vendor/private-model',
    displayName: 'Private Model',
    baseUrl: channelDefaults.baseUrl,
    apiBackend: channelDefaults.apiBackend,
  });
});

test('the channel protocol decides which preset group is searched first', () => {
  updatePresetModels(bundledPresets);

  // Known in the Anthropic group only; reachable through the protocol-matched
  // group even though the xAI group does not list it.
  assert.equal(findGrokPresetForModel('claude-opus-4-8', 'anthropic_messages')?.id, 'claude-opus-4-8');
  // And the fallback still finds it when the channel speaks chat, because the
  // preset lookup is provider-aware but not provider-limited.
  assert.equal(findGrokPresetForModel('claude-opus-4-8', 'openai_chat')?.id, 'claude-opus-4-8');
});

test('merging keeps an existing row’s own settings and enriches only the new ones', () => {
  updatePresetModels(bundledPresets);

  // Existing catalog keys equal the upstream model id, so this row *is* the
  // fetched `grok-4.7` — the modal disables it, but a stale caller may still
  // pass it, and it must not lose the user's context window / effort menu.
  const current = [{
    key: 'grok-4.7',
    model: 'grok-4.7',
    displayName: 'My Grok',
    contextWindow: 123_456,
    reasoningEfforts: ['low', 'high'],
    reasoningEffort: 'low',
    supportsReasoningEffort: true,
  }];

  const merged = mergeFetchedGrokCatalogModels(
    current,
    [
      { id: 'grok-4.7' },
      { id: 'grok-4.5', name: 'Grok 4.5' },
    ],
    'openai_chat',
    channelDefaults,
  );

  const existing = merged.find((model) => model.key === 'grok-4.7');
  assert.equal(existing?.contextWindow, 123_456);
  assert.deepEqual(existing?.reasoningEfforts, ['low', 'high']);
  assert.equal(existing?.reasoningEffort, 'low');
  assert.equal(existing?.baseUrl, channelDefaults.baseUrl);
  assert.equal(existing?.apiBackend, channelDefaults.apiBackend);
  // The row keeps its own display name when the fetched entry has none.
  assert.equal(existing?.displayName, 'My Grok');

  const added = merged.find((model) => model.key === 'grok-4.5');
  assert.equal(added?.displayName, 'Grok 4.5');
  assert.equal(added?.contextWindow, 500_000);
  assert.deepEqual(added?.reasoningEfforts, ['low', 'medium', 'high']);
});
