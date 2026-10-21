import assert from 'node:assert/strict';
import test from 'node:test';

import {
  codexWireApiFormatFromConfig,
  grokProviderNeedsGatewayProxy,
  hasClaudeDesktopRoutingModels,
  isGatewayConfigFlagEnabled,
  providerNeedsGatewayProxy,
} from '../../../../../features/coding/shared/gateway/providerProtocol.ts';

test('codex wire api follows the selected model provider table', () => {
  assert.equal(codexWireApiFormatFromConfig(`
model_provider = "chat"
wire_api = "responses"

[model_providers.responses]
wire_api = "responses"

[model_providers.chat]
wire_api = "chat"
`), 'chat');
});

test('codex wire api keeps root-level legacy compatibility', () => {
  assert.equal(codexWireApiFormatFromConfig('wire_api = "chat"'), 'chat');
});

test('gateway config flag parser matches backend truthy compatibility values', () => {
  for (const value of [true, 1, 'true', '1', 'yes', 'on', ' YES ']) {
    assert.equal(isGatewayConfigFlagEnabled(value), true);
  }

  for (const value of [false, 0, 'false', '0', 'no', 'off', '', null, undefined]) {
    assert.equal(isGatewayConfigFlagEnabled(value), false);
  }
});

test('grok only needs gateway for gemini native, not chat/responses/anthropic', () => {
  for (const format of ['openai_chat', 'openai_responses', 'anthropic_messages', 'chat', 'responses', 'messages']) {
    assert.equal(grokProviderNeedsGatewayProxy(format), false);
  }
  assert.equal(grokProviderNeedsGatewayProxy('gemini_native'), true);
  assert.equal(grokProviderNeedsGatewayProxy(null), false);
  // Contrast: single-native compare would wrongly force proxy for chat.
  assert.equal(providerNeedsGatewayProxy('openai_chat', 'openai_responses'), true);
});

test('claude desktop routing models mirror the backend has_routing_models', () => {
  // Identity routes: Direct mode can write them, no gateway needed.
  assert.equal(hasClaudeDesktopRoutingModels({
    'claude-sonnet-5': { model: 'claude-sonnet-5' },
    'claude-opus-5': { model: 'claude-opus-5' },
  }), false);

  // A claude-safe upstream that differs from its route id is still a mapping:
  // Direct mode rejects it and the backend writes a gateway profile, so the
  // card must offer 「应用并代理」, not a plain 应用 (AnyRouter, 2026-10-21).
  assert.equal(hasClaudeDesktopRoutingModels({
    'claude-sonnet-5': { model: 'claude-opus-5' },
  }), true);

  // Non-claude upstreams are a mapping too (the old hasNonClaudeModelIds case).
  assert.equal(hasClaudeDesktopRoutingModels({
    'claude-sonnet-5': { model: 'deepseek-v4-pro' },
  }), true);

  // Empty / missing routes say nothing about needing the gateway.
  assert.equal(hasClaudeDesktopRoutingModels({}), false);
  assert.equal(hasClaudeDesktopRoutingModels(undefined), false);
  assert.equal(hasClaudeDesktopRoutingModels(null), false);

  // A blank model is skipped rather than treated as a mapping of "".
  assert.equal(hasClaudeDesktopRoutingModels({ 'claude-sonnet-5': { model: '  ' } }), false);
  // Surrounding whitespace does not make an identity route look like a mapping.
  assert.equal(hasClaudeDesktopRoutingModels({ 'claude-sonnet-5': { model: ' claude-sonnet-5 ' } }), false);
});
