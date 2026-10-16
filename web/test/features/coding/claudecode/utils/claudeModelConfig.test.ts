import assert from 'node:assert/strict';
import test from 'node:test';

import {
  CLAUDE_CARD_ROLE_LABELS,
  CLAUDE_DETAIL_BINDING_LABELS,
  getClaudeConfiguredModelIds,
  getClaudeProviderModelConfig,
  hasClaudeOneMMarker,
  resolveClaudeCardSubtitles,
  resolveClaudeDetailBindings,
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

test('getClaudeProviderModelConfig reads the subagent binding without a display name', () => {
  const config = getClaudeProviderModelConfig({
    env: {
      CLAUDE_CODE_SUBAGENT_MODEL: 'subagent-model[1M]',
    },
  });

  assert.equal(config.roles.subagent.model, 'subagent-model[1M]');
  assert.equal(config.roles.subagent.supportsOneM, true);
  // Claude Code has no `CLAUDE_CODE_SUBAGENT_MODEL_NAME`, so this role carries
  // no display name — the form renders a read-only placeholder for that column
  // rather than an input that would write a variable nothing reads.
  assert.equal(config.roles.subagent.hasDisplayName, false);
  assert.equal(config.roles.subagent.displayName, '');
});

test('getClaudeProviderModelConfig reads the custom model option and its label', () => {
  const config = getClaudeProviderModelConfig({
    env: {
      ANTHROPIC_CUSTOM_MODEL_OPTION: 'my-gateway/deepseek-v4-pro[1M]',
      ANTHROPIC_CUSTOM_MODEL_OPTION_NAME: 'DeepSeek via gateway',
    },
  });

  assert.equal(config.roles.custom.model, 'my-gateway/deepseek-v4-pro[1M]');
  assert.equal(config.roles.custom.displayName, 'DeepSeek via gateway');
  assert.equal(config.roles.custom.supportsOneM, true);
  assert.equal(config.roles.custom.hasDisplayName, true);
});

test('the four tiers and the custom option carry a display name while subagent does not', () => {
  const config = getClaudeProviderModelConfig({ env: {} });

  for (const role of ['sonnet', 'opus', 'fable', 'haiku', 'custom'] as const) {
    assert.equal(config.roles[role].hasDisplayName, true, role);
  }
  assert.equal(config.roles.subagent.hasDisplayName, false);
});

test('getClaudeConfiguredModelIds collects the subagent and custom bindings', () => {
  const modelIds = getClaudeConfiguredModelIds({
    env: {
      ANTHROPIC_MODEL: 'fallback',
      CLAUDE_CODE_SUBAGENT_MODEL: 'subagent-model[1M]',
      ANTHROPIC_CUSTOM_MODEL_OPTION: 'custom-model',
    },
  }, {
    stripOneMMarker: true,
  });

  // Both are shareable provider state, so the deep-link payload must carry them
  // alongside the tiers.
  assert.deepEqual(modelIds, ['fallback', 'subagent-model', 'custom-model']);
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

test('resolveClaudeDetailBindings lists every configured binding including subagent and custom', () => {
  const config = getClaudeProviderModelConfig({
    env: {
      // Out of order on purpose: the result follows the declared order, not the
      // config's key order.
      ANTHROPIC_DEFAULT_FABLE_MODEL: 'fable-model[1M]',
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'haiku-model',
      CLAUDE_CODE_SUBAGENT_MODEL: 'subagent-model',
      ANTHROPIC_CUSTOM_MODEL_OPTION: 'my-gateway/deepseek-v4-pro',
      // Neither of these is a binding this list states.
      ANTHROPIC_MODEL: 'fallback-model',
      ANTHROPIC_REASONING_MODEL: 'reasoning-model',
    },
  });

  assert.deepEqual(resolveClaudeDetailBindings(config), [
    { role: 'haiku', model: 'haiku-model' },
    { role: 'fable', model: 'fable-model[1M]' },
    { role: 'subagent', model: 'subagent-model' },
    { role: 'custom', model: 'my-gateway/deepseek-v4-pro' },
  ]);
});

test('resolveClaudeDetailBindings keeps the 1M marker and skips unconfigured bindings', () => {
  const config = getClaudeProviderModelConfig({
    env: { CLAUDE_CODE_SUBAGENT_MODEL: 'subagent-model[1M]' },
  });

  // The marker is part of the stored value and these surfaces exist so a user
  // can check it is set, so it stays on.
  assert.deepEqual(resolveClaudeDetailBindings(config), [
    { role: 'subagent', model: 'subagent-model[1M]' },
  ]);
});

test('every detail binding has a label key', () => {
  const config = getClaudeProviderModelConfig({
    env: {
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'haiku-model',
      ANTHROPIC_DEFAULT_SONNET_MODEL: 'sonnet-model',
      ANTHROPIC_DEFAULT_OPUS_MODEL: 'opus-model',
      ANTHROPIC_DEFAULT_FABLE_MODEL: 'fable-model',
      CLAUDE_CODE_SUBAGENT_MODEL: 'subagent-model',
      ANTHROPIC_CUSTOM_MODEL_OPTION: 'custom-model',
    },
  });

  // A total map over every `ClaudeModelRole`, so adding a role without a label
  // fails here rather than rendering a bare key in the sidebar.
  for (const entry of resolveClaudeDetailBindings(config)) {
    assert.equal(typeof CLAUDE_DETAIL_BINDING_LABELS[entry.role], 'string');
    assert.ok(CLAUDE_DETAIL_BINDING_LABELS[entry.role].startsWith('claudecode.model.role'));
  }
});
