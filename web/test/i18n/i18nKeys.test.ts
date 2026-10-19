import assert from 'node:assert/strict';
import test from 'node:test';

interface I18nAnalysis {
  expandedDynamicKeyUsages: Array<{ key: string }>;
  missingStaticKeys: unknown[];
  localeMismatches: unknown[];
}

interface TextSearchResult {
  locale: string;
  key: string;
  value: string;
}

const i18nKeysModuleUrl = new URL('../../../scripts/i18n-keys.mjs', import.meta.url);
const i18nKeys = await import(i18nKeysModuleUrl.href) as {
  analyzeProject: () => Promise<I18nAnalysis>;
  findKeysByText: (analysis: I18nAnalysis, query: string) => TextSearchResult[];
};

test('i18n locale files cover statically used translation keys', async () => {
  const analysis = await i18nKeys.analyzeProject();

  assert.deepEqual(analysis.missingStaticKeys, []);
  assert.deepEqual(analysis.localeMismatches, []);
});

test('i18n check expands known dynamic translation key helpers', async () => {
  const analysis = await i18nKeys.analyzeProject();
  const expandedKeys = new Set(analysis.expandedDynamicKeyUsages.map((usage) => usage.key));

  // Static key builders are resolved at the call site and land in `usedKeys`,
  // not in the dynamic expansion list — the OpenCode provider form is one.
  // The expansion path itself is still exercised by the remaining caller of a
  // per-prefix helper: `OfficialProviderCard`'s `${i18nPrefix}.official.*`.
  assert.ok(expandedKeys.has('opencode.official.modelReadOnlyHint'));
  assert.ok(expandedKeys.has('opencode.official.freeModel'));
  assert.ok(expandedKeys.has('omoNative.official.noModels'));
});

test('i18n text lookup can find translation keys without reading full locale files', async () => {
  const analysis = await i18nKeys.analyzeProject();
  const matches = i18nKeys.findKeysByText(analysis, 'VSCode 集成');

  assert.ok(
    matches.some((match) => (
      match.locale === 'zh-CN'
      && match.key === 'claudecode.settings.vscode'
      && match.value === 'VSCode 集成'
    )),
  );
});
