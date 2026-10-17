/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

import {
  buildZcodeSettingsConfigFromImport,
  extractZcodeProviderFromAllApiHub,
  extractZcodeProviderFromCcSwitch,
} from '../../../../../features/coding/zcode/utils/zcodeImportMapping.ts';
import type { CcSwitchProviderCandidate } from '../../../../../services/ccSwitchApi.ts';
import type { OpenCodeAllApiHubProvider } from '../../../../../services/opencodeApi.ts';

const ccSwitchCandidate = (
  settingsConfig: CcSwitchProviderCandidate['settingsConfig'],
  overrides: Partial<CcSwitchProviderCandidate> = {},
): CcSwitchProviderCandidate => ({
  providerId: 'ccs:claude:abc',
  rawId: 'abc',
  name: 'My Relay',
  appType: 'claude',
  normalizedCategory: 'custom',
  settingsConfig,
  hasApiKey: true,
  isLocalEndpoint: false,
  ...overrides,
});

const allApiHubProvider = (
  providerConfig: OpenCodeAllApiHubProvider['providerConfig'],
  overrides: Partial<OpenCodeAllApiHubProvider> = {},
): OpenCodeAllApiHubProvider => ({
  providerId: 'hub-1',
  name: 'Hub Relay',
  npm: '@ai-sdk/openai-compatible',
  requiresBrowserOpen: false,
  isDisabled: false,
  hasApiKey: true,
  accountLabel: 'account',
  sourceProfileName: 'default',
  sourceExtensionId: 'ext',
  providerConfig,
  ...overrides,
});

test('a CC Switch provider reads its endpoint out of the env blob', () => {
  const imported = extractZcodeProviderFromCcSwitch(
    ccSwitchCandidate(
      JSON.stringify({
        env: {
          ANTHROPIC_BASE_URL: 'https://relay.test/anthropic',
          ANTHROPIC_AUTH_TOKEN: 'sk-cc',
        },
      }),
    ),
  );

  // CC Switch stores Claude-shaped providers only, so the format is not a guess
  // to be made from the blob — it is the only one these can be.
  assert.deepEqual(imported, {
    name: 'My Relay',
    apiType: 'anthropic-messages',
    baseUrl: 'https://relay.test/anthropic',
    apiKey: 'sk-cc',
    notes: undefined,
  });
});

test('the api key falls back from the auth token to the api key field', () => {
  const imported = extractZcodeProviderFromCcSwitch(
    ccSwitchCandidate(
      JSON.stringify({ env: { ANTHROPIC_API_KEY: 'sk-fallback' } }),
    ),
  );
  assert.equal(imported?.apiKey, 'sk-fallback');
});

test('a CC Switch provider with neither endpoint nor key is not importable', () => {
  assert.equal(extractZcodeProviderFromCcSwitch(ccSwitchCandidate(JSON.stringify({ env: {} }))), null);
  assert.equal(extractZcodeProviderFromCcSwitch(ccSwitchCandidate('not json')), null);
  assert.equal(
    extractZcodeProviderFromCcSwitch(ccSwitchCandidate(JSON.stringify({ env: { FOO: 'bar' } }))),
    null,
  );
});

test('an All API Hub provider maps to an OpenAI-shaped ZCode provider', () => {
  const imported = extractZcodeProviderFromAllApiHub(
    allApiHubProvider({
      options: { baseURL: ' https://hub.test/v1 ', apiKey: ' sk-hub ' },
      models: {},
    }),
  );

  assert.deepEqual(imported, {
    name: 'Hub Relay',
    apiType: 'openai-chat-completions',
    baseUrl: 'https://hub.test/v1',
    apiKey: 'sk-hub',
  });
});

test('an All API Hub provider carrying nothing usable is not importable', () => {
  assert.equal(extractZcodeProviderFromAllApiHub(allApiHubProvider({ options: {}, models: {} })), null);
});

test('the stored settings config keeps the endpoint, the key and the format', () => {
  const settings = JSON.parse(
    buildZcodeSettingsConfigFromImport({
      name: 'My Relay',
      apiType: 'anthropic-messages',
      baseUrl: 'https://relay.test',
      apiKey: 'sk-1',
    }),
  );

  // The registry identity is derived from the name up front (mirroring the
  // form flow); a blank providerId used to degrade the card and make every
  // later write fail the backend's managed-id projection.
  assert.deepEqual(settings, {
    providerId: 'custom:my-relay',
    providerName: 'My Relay',
    config: {
      access: { type: 'api-key', apiKey: 'sk-1' },
      api: { type: 'anthropic-messages', baseUrl: 'https://relay.test' },
    },
    models: [],
  });
});

test('a missing key or endpoint is omitted rather than written as empty', () => {
  const settings = JSON.parse(
    buildZcodeSettingsConfigFromImport({ name: 'Local', apiType: 'openai-chat-completions' }),
  );

  assert.deepEqual(settings.config, {
    access: { type: 'api-key' },
    api: { type: 'openai-chat-completions' },
  });
});