import assert from 'node:assert/strict';
import test from 'node:test';

import {
  CLAUDE_CARD_ROLE_LABELS,
  getClaudeConfiguredModelIds,
  getClaudeProviderModelConfig,
  hasClaudeOneMMarker,
  resolveClaudeCardSubtitles,
  setClaudeOneMMarker,
  stripClaudeOneMMarker,
} from '../../../../../features/coding/claudecode/utils/claudeModelConfig.ts';

test('Claude 1M marker helpers handle case-insensitive suffixes', () => {
  assert.equal(hasClaudeOneMMarker('claude-sonnet[1m]'), true);
  assert.equal(stripClaudeOneMMarker('claude-sonnet[1m]'), 'claude-sonnet');
  assert.equal(setClaudeOneMMarker('claude-sonnet', true), 'claude-sonnet[1M]');
  assert.equal(setClaudeOneMMarker('claude-sonnet[1M]', false), 'claude-sonnet');
});

test('Claude 1M marker helpers keep empty models empty and preserve existing markers', () => {
  assert.equal(setClaudeOneMMarker('', true), '');
  assert.equal(setClaudeOneMMarker('   [1M] ', true), '');
  assert.equal(setClaudeOneMMarker('claude-sonnet[1M]', true), 'claude-sonnet[1M]');
  assert.equal(hasClaudeOneMMarker('claude-sonnet[1M] '), true);
});

test('getClaudeProviderModelConfig keeps the 1M marker on the fallback model', () => {
  const config = getClaudeProviderModelConfig({
    env: {
      ANTHROPIC_MODEL: 'env-fallback[1M]',
    },
  });

  assert.equal(config.fallbackModel, 'env-fallback[1M]');
});

test('getClaudeProviderModelConfig prefers env fields and keeps legacy fallbacks', () => {
  const config = getClaudeProviderModelConfig({
    env: {
      ANTHROPIC_MODEL: 'env-fallback',
      ANTHROPIC_DEFAULT_SONNET_MODEL: 'env-sonnet[1M]',
      ANTHROPIC_DEFAULT_SONNET_MODEL_NAME: 'Env Sonnet',
      ANTHROPIC_DEFAULT_OPUS_MODEL: 'env-opus',
      ANTHROPIC_DEFAULT_FABLE_MODEL: 'env-fable[1M]',
      ANTHROPIC_DEFAULT_FABLE_MODEL_NAME: 'Env Fable',
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'env-haiku[1M]',
      ANTHROPIC_REASONING_MODEL: 'env-reasoning',
    },
    model: 'legacy-fallback',
    haikuModel: 'legacy-haiku',
    sonnetModel: 'legacy-sonnet',
    opusModel: 'legacy-opus',
    fableModel: 'legacy-fable',
    reasoningModel: 'legacy-reasoning',
  });

  assert.equal(config.fallbackModel, 'env-fallback');
  assert.equal(config.roles.sonnet.model, 'env-sonnet[1M]');
  assert.equal(config.roles.sonnet.displayName, 'Env Sonnet');
  assert.equal(config.roles.opus.model, 'env-opus');
  assert.equal(config.roles.opus.displayName, 'env-opus');
  assert.equal(config.roles.fable.model, 'env-fable[1M]');
  assert.equal(config.roles.fable.displayName, 'Env Fable');
  assert.equal(config.roles.fable.supportsOneM, true);
  assert.equal(config.roles.haiku.model, 'env-haiku[1M]');
  assert.equal(config.roles.haiku.displayName, 'env-haiku');
  assert.equal(config.roles.haiku.supportsOneM, true);
  assert.equal(config.legacyReasoningModel, 'env-reasoning');
});

test('getClaudeProviderModelConfig leaves Fable empty when Fable is unset', () => {
  const config = getClaudeProviderModelConfig({
    env: {
      ANTHROPIC_DEFAULT_OPUS_MODEL: 'env-opus[1M]',
      ANTHROPIC_DEFAULT_OPUS_MODEL_NAME: 'Env Opus',
    },
  });

  assert.equal(config.roles.fable.model, '');
  assert.equal(config.roles.fable.displayName, '');
});

test('getClaudeConfiguredModelIds can strip 1M markers and dedupe ids', () => {
  const modelIds = getClaudeConfiguredModelIds({
    env: {
      ANTHROPIC_MODEL: 'env-sonnet',
      ANTHROPIC_DEFAULT_SONNET_MODEL: 'env-sonnet[1M]',
      ANTHROPIC_DEFAULT_OPUS_MODEL: 'env-opus[1M]',
      ANTHROPIC_DEFAULT_FABLE_MODEL: 'env-fable[1M]',
      ANTHROPIC_REASONING_MODEL: 'env-opus',
    },
  }, {
    stripOneMMarker: true,
  });

  assert.deepEqual(modelIds, ['env-sonnet', 'env-opus', 'env-fable']);
});

test('resolveClaudeCardSubtitles lists the configured roles in tier order', () => {
  const config = getClaudeProviderModelConfig({
    env: {
      // Deliberately out of order in the config, and with the default and
      // reasoning slots set as well: neither belongs on the card's line.
      ANTHROPIC_MODEL: 'fallback-model',
      ANTHROPIC_DEFAULT_FABLE_MODEL: 'fable-model',
      ANTHROPIC_DEFAULT_OPUS_MODEL: 'opus-model',
      ANTHROPIC_DEFAULT_SONNET_MODEL: 'sonnet-model',
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'haiku-model',
      ANTHROPIC_REASONING_MODEL: 'reasoning-model',
    },
  });

  assert.deepEqual(resolveClaudeCardSubtitles(config), [
    { role: 'opus', model: 'opus-model' },
    { role: 'sonnet', model: 'sonnet-model' },
    { role: 'haiku', model: 'haiku-model' },
    { role: 'fable', model: 'fable-model' },
  ]);
});

test('resolveClaudeCardSubtitles skips roles with no model and keeps the rest', () => {
  const onlyFable = getClaudeProviderModelConfig({
    env: { ANTHROPIC_DEFAULT_FABLE_MODEL: 'fable-model' },
  });

  // The unconfigured roles leave no gap and no placeholder entry — the line
  // simply states what exists.
  assert.deepEqual(resolveClaudeCardSubtitles(onlyFable), [
    { role: 'fable', model: 'fable-model' },
  ]);
});

test('resolveClaudeCardSubtitles prefers the display name and strips 1M markers', () => {
  const config = getClaudeProviderModelConfig({
    env: {
      ANTHROPIC_DEFAULT_OPUS_MODEL: 'opus-model[1M]',
      ANTHROPIC_DEFAULT_OPUS_MODEL_NAME: 'DeepSeek V4 Pro',
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'haiku-model[1M]',
    },
  });

  assert.deepEqual(resolveClaudeCardSubtitles(config), [
    { role: 'opus', model: 'DeepSeek V4 Pro' },
    // No display name configured: the raw id is shown, without the 1M marker.
    { role: 'haiku', model: 'haiku-model' },
  ]);
});

test('resolveClaudeCardSubtitles ignores the default and reasoning slots', () => {
  // A provider whose only models are the catch-alls the list card shows: the
  // card grid states no role bindings, so its second line is empty.
  const catchAllsOnly = getClaudeProviderModelConfig({
    env: {
      ANTHROPIC_MODEL: 'fallback-model',
      ANTHROPIC_REASONING_MODEL: 'reasoning-model',
    },
  });

  // An empty array, not a `null` or a blank entry: the card renders no line.
  assert.deepEqual(resolveClaudeCardSubtitles(catchAllsOnly), []);
});

test('every card role has a label key', () => {
  const config = getClaudeProviderModelConfig({
    env: {
      ANTHROPIC_DEFAULT_OPUS_MODEL: 'opus-model',
      ANTHROPIC_DEFAULT_SONNET_MODEL: 'sonnet-model',
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'haiku-model',
      ANTHROPIC_DEFAULT_FABLE_MODEL: 'fable-model',
    },
  });

  // The labels are a total map over the roles, so a role added to the line
  // without a label fails here rather than rendering a bare key on the card.
  for (const entry of resolveClaudeCardSubtitles(config)) {
    assert.equal(typeof CLAUDE_CARD_ROLE_LABELS[entry.role], 'string');
    assert.ok(CLAUDE_CARD_ROLE_LABELS[entry.role].startsWith('claudecode.model.role'));
  }
});
