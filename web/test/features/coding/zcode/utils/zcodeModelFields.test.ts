/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import {
  buildInputFormat,
  buildZcodeModelRowFromPreset,
  pickZcodeSystemProperties,
  preferredPresetNpmTypes,
  presetInputModalities,
  readInputModalities,
  zcodeModalityValuesFor,
} from '../../../../../features/coding/zcode/utils/zcodeModelFields.ts';
import {
  findPresetModelById,
  updatePresetModels,
} from '../../../../../constants/presetModels.ts';

const bundledPresets = JSON.parse(
  readFileSync(
    new URL('../../../../../../tauri/resources/preset_models.json', import.meta.url),
    'utf8',
  ),
);

test('a known preset fills every field ZCode stores', () => {
  updatePresetModels(bundledPresets);

  const row = buildZcodeModelRowFromPreset(
    'claude-opus-4-8',
    'smart',
    findPresetModelById('claude-opus-4-8'),
  );

  assert.deepEqual(row, {
    modelId: 'claude-opus-4-8',
    displayName: 'Claude Opus 4.8',
    ruleKind: 'smart',
    enabled: true,
    properties: {
      contextWindow: 1_000_000,
      inputFormat: { supportsText: true, supportsImage: true, supportsPdf: true },
    },
    optionSpecs: { maxOutputTokens: { max: 128_000 } },
    isDefault: false,
  });
});

test('a manual rule carries only the modalities its schema declares', () => {
  updatePresetModels(bundledPresets);

  const row = buildZcodeModelRowFromPreset(
    'claude-opus-4-8',
    'manual',
    findPresetModelById('claude-opus-4-8'),
  );

  // `text` is smart-only: ZCode's manual rule schema does not declare it.
  assert.deepEqual(row.properties?.inputFormat, { supportsImage: true, supportsPdf: true });
});

test('a preset never writes the system properties, whatever the catalog says', () => {
  updatePresetModels(bundledPresets);

  // The bundled catalog reports `tool_call: true` for this model, and ZCode's
  // own dialog still leaves `supportsToolCall` to the CLI rather than to the
  // preset. Writing it here would produce a key the official editor never sets.
  const row = buildZcodeModelRowFromPreset(
    'claude-opus-4-8',
    'smart',
    findPresetModelById('claude-opus-4-8'),
  );

  assert.equal(row.properties?.supportsToolCall, undefined);
  assert.equal(row.properties?.requiresMfjsToolSchema, undefined);
});

test('system properties survive a save that does not expose them', () => {
  // A row ZCode itself wrote carries these; re-saving from a form that has no
  // control for them must not delete them.
  const existing = {
    supportsToolCall: true,
    requiresMfjsToolSchema: false,
    contextWindow: 200_000,
  };

  assert.deepEqual(pickZcodeSystemProperties(existing), {
    supportsToolCall: true,
    requiresMfjsToolSchema: false,
  });
});

test('the outputFormat system property survives a save too', () => {
  // `outputFormat` is the third smart-only system field; it was missing from
  // the carry list, so saving any row ZCode wrote with an output format
  // silently dropped the key and the model stopped declaring text output.
  const existing = {
    outputFormat: { supportsText: true, supportsReasoning: true },
    contextWindow: 200_000,
  };

  assert.deepEqual(pickZcodeSystemProperties(existing), {
    outputFormat: { supportsText: true, supportsReasoning: true },
  });
});

test('an absent system property stays absent rather than becoming false', () => {
  // "Not set" and "set to false" are different to ZCode: an absent key lets its
  // catalog decide, an explicit false overrides it.
  assert.deepEqual(pickZcodeSystemProperties({ contextWindow: 1 }), {});
  assert.deepEqual(pickZcodeSystemProperties(undefined), {});
});

test('a model the catalog does not know still yields a usable row', () => {
  updatePresetModels(bundledPresets);

  assert.deepEqual(buildZcodeModelRowFromPreset('my-private-model', 'smart'), {
    modelId: 'my-private-model',
    displayName: undefined,
    ruleKind: 'smart',
    enabled: true,
    properties: undefined,
    optionSpecs: undefined,
    isDefault: false,
  });
});

test('preset modalities ZCode cannot carry are dropped, not written', () => {
  const gemini = {
    id: 'gemini-3.8-flash',
    name: 'Gemini 3.8 Flash',
    modalities: { input: ['text', 'image', 'pdf', 'video', 'audio'], output: ['text'] },
  };

  assert.deepEqual(presetInputModalities(gemini, 'smart'), [
    'text',
    'image',
    'pdf',
    'video',
    'audio',
  ]);
  assert.deepEqual(presetInputModalities(gemini, 'manual'), ['image', 'pdf', 'video']);
});

test('an empty modality selection writes no inputFormat at all', () => {
  assert.equal(buildInputFormat([]), undefined);
  assert.deepEqual(buildInputFormat(['image', 'pdf']), {
    supportsImage: true,
    supportsPdf: true,
  });
});

test('the stored per-modality booleans read back as the form selection', () => {
  const stored = buildInputFormat(['text', 'video']);
  assert.deepEqual(readInputModalities(stored ?? {}), ['text', 'video']);
  assert.deepEqual(readInputModalities({}), []);
});

test('a provider API format picks which preset group is consulted first', () => {
  assert.deepEqual(preferredPresetNpmTypes('anthropic-messages'), ['@ai-sdk/anthropic']);
  assert.deepEqual(preferredPresetNpmTypes('openai-chat-completions'), [
    '@ai-sdk/openai',
    '@ai-sdk/openai-compatible',
  ]);
  assert.deepEqual(preferredPresetNpmTypes(undefined), []);
});

test('each rule kind offers exactly the modalities its schema declares', () => {
  assert.deepEqual(zcodeModalityValuesFor('smart'), ['text', 'image', 'video', 'audio', 'pdf']);
  assert.deepEqual(zcodeModalityValuesFor('manual'), ['image', 'video', 'pdf']);
});