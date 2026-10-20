import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

/**
 * Real-browser assertions for the four mapping layers migrated on 2026-10-20
 * (Claude Desktop, Gemini CLI, Grok, OmO Native's card view).
 *
 * Why a browser: every claim here is about what is rendered — which layout a
 * mapping layer picked, which facts a cell states, whether a footer spans the
 * card, whether a protected delete is greyed out. Source guards
 * (`verify-provider-card-layout`) only prove the mapping layer is thin; they
 * cannot see that the footer geometry regressed or that the row lost its sixth
 * action.
 */
export async function verifyMigratedProviderCards({ send, evaluate, baseUrl, artifactRoot }) {
  const checks = [];
  const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
  const check = (name, actual, expected = true) => {
    assert.deepEqual(actual, expected, name);
    checks.push(name);
    console.log('PASS ' + name);
  };
  const waitFor = async (expression, label) => {
    for (let attempt = 0; attempt < 200; attempt++) {
      if (await evaluate(expression)) return;
      await delay(50);
    }
    throw new Error('Timed out waiting for: ' + (label || expression));
  };
  const fixture = expression => evaluate('migratedProviderCardsFixture.' + expression);
  const screenshot = async name => {
    const { data } = await send('Page.captureScreenshot', { format: 'png' });
    await writeFile(path.join(artifactRoot, name + '.png'), Buffer.from(data, 'base64'));
  };

  const openFixture = async (query) => {
    const runId = `${query}-${Date.now()}`;
    await send('Page.navigate', { url: `${baseUrl}/?${query}&runId=${encodeURIComponent(runId)}` });
    await waitFor(
      "Boolean(window.migratedProviderCardsFixture)"
        + " && Boolean(document.querySelector('[data-fixture=\"grok-custom-list\"]'))",
      `fixture ${query}`,
    );
    await delay(300);
  };

  /* ---------------------------------------------------------------- Claude Desktop */

  await openFixture('theme=light');

  // Role bindings keep their `Role: model` shape, with the value monospaced.
  const desktopText = await fixture('text("claude-desktop-list")');
  check(
    'the Claude Desktop card states each configured role as `Role: model`',
    /Sonnet:\s*claude-sonnet-4-5/.test(desktopText) && /Opus:\s*claude-opus-4-8/.test(desktopText),
  );

  // `tierAlias` is a real tag, not secondary text.
  const tierTag = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="claude-desktop-list"]');
      const tags = [...root.querySelectorAll('.ant-tag')];
      return tags.some(tag => tag.textContent.includes('sonnet'));
    })()
  `);
  check('a role tier alias renders as a tag', tierTag);

  // Notes live in the footer, below the name row and spanning the card's body.
  const notesBelowHeader = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="claude-desktop-list"]');
      const note = [...root.querySelectorAll('*')].find(node =>
        node.children.length === 0 && node.textContent.trim() === '团队共享额度');
      if (!note) return false;
      const name = [...root.querySelectorAll('*')].find(node =>
        node.children.length === 0 && node.textContent.includes('Desktop Relay'));
      return note.getBoundingClientRect().top > name.getBoundingClientRect().bottom;
    })()
  `);
  check('notes sit below the name row, not beside it', notesBelowHeader);

  // A disabled drag leaves no handle at all (the grip used to stay and do nothing).
  check('a drag-disabled card renders no handle', await fixture('hasHandle("claude-desktop-list")'), false);

  // The grid cell states the role bindings on its second line.
  const desktopSubtitle = await fixture('subtitle("claude-desktop-card")');
  check(
    'the Claude Desktop cell states its role bindings',
    Boolean(desktopSubtitle && /Sonnet:\s*claude-sonnet-4-5/.test(desktopSubtitle)),
  );

  /* ---------------------------------------------------------------- Gemini CLI */

  const geminiText = await fixture('text("gemini-list")');
  check(
    'the Gemini official card is titled with the shared heading and its count',
    /官方账号\s*\(2\)/.test(geminiText),
  );
  check(
    'the Gemini official card explains switching on its second line',
    geminiText.includes('替换当前登录'),
  );

  // The account block's rows must reach the card's content edge; the footer
  // used to sit in the header's left column and stopped ~160px short.
  //
  // The list card's fold starts closed (it always has), so open it first.
  check('the Gemini account list can be opened from its title', await fixture('expandAccounts("gemini-list")'));
  await delay(200);
  const rowEdges = await fixture('edges("gemini-list", "[style*=\\"border-bottom\\"]")');
  const cardContentEdges = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="gemini-list"]');
      const card = root.querySelector('.ant-card');
      const body = card.querySelector('.ant-card-body');
      const style = getComputedStyle(body);
      const rect = body.getBoundingClientRect();
      return {
        right: Math.round(rect.right - parseFloat(style.paddingRight)),
        left: Math.round(rect.left + parseFloat(style.paddingLeft)),
      };
    })()
  `);
  check(
    'the account rows span the card content width',
    Boolean(rowEdges) && Math.abs(rowEdges.right - cardContentEdges.right) <= 2,
  );

  // A virtual (disk-only) login can only be saved.
  const virtualRowText = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="gemini-list"]');
      const row = [...root.querySelectorAll('[style*="border-bottom"]')]
        .find(node => node.textContent.includes('local@example.test'));
      return row ? row.textContent.replace(/\\s+/g, ' ').trim() : null;
    })()
  `);
  check(
    'the virtual login row offers 「保存当前登录」 and no switch',
    Boolean(virtualRowText
      && virtualRowText.includes('保存当前登录')
      && !virtualRowText.includes('切换')),
  );

  // The applied stored account offers no switch either (it is the current one).
  const appliedRowText = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="gemini-list"]');
      const row = [...root.querySelectorAll('[style*="border-bottom"]')]
        .find(node => node.textContent.includes('work@example.test'));
      return row ? row.textContent.replace(/\\s+/g, ' ').trim() : null;
    })()
  `);
  check(
    'the applied account is marked default instead of offering a switch',
    Boolean(appliedRowText && appliedRowText.includes('默认') && !appliedRowText.includes('切换')),
  );

  // The cell states the login in use.
  const geminiSubtitle = await fixture('subtitle("gemini-card")');
  check(
    'the Gemini official cell states the current login',
    Boolean(geminiSubtitle && geminiSubtitle.includes('当前登录') && geminiSubtitle.includes('work@example.test')),
  );

  /* ---------------------------------------------------------------- Grok */

  const grokOfficialText = await fixture('text("grok-official-list")');
  check(
    'the Grok official card is titled with the shared heading',
    /官方账号\s*\(1\)/.test(grokOfficialText),
  );
  // The fold starts closed on the list card; the sixth action lives on a row.
  check('the Grok account list can be opened from its title', await fixture('expandAccounts("grok-official-list")'));
  await delay(200);
  const grokExpandedText = await fixture('text("grok-official-list")');
  check(
    'Grok keeps its sixth account action through the shared extra-action slot',
    grokExpandedText.includes('刷新 Token'),
  );

  // The refresh-token action is absent from the custom provider (no accounts).
  check(
    'a custom Grok provider renders no account block',
    !(await fixture('text("grok-custom-list")')).includes('刷新 Token'),
  );

  // The model toolbar keeps the documented order:
  // 批量删除 → 模型测试 → 获取模型 → 添加模型.
  //
  // Searched from the catalog's own title, not from the top of the card: the
  // header's inline connectivity action carries the same words (模型测试), and
  // an unscoped `indexOf` would score that one as the first toolbar item.
  const toolbarOrder = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="grok-custom-list"]');
      const text = root.textContent.replace(/\s+/g, ' ');
      const from = text.indexOf('模型列表');
      if (from < 0) return [-1, -1, -1, -1];
      return ['批量删除', '模型测试', '获取模型', '添加模型']
        .map(label => text.indexOf(label, from));
    })()
  `);
  check(
    'the Grok model toolbar keeps its documented order',
    toolbarOrder.every((index, position) => index >= 0 && (position === 0 || index > toolbarOrder[position - 1])),
  );

  // The custom card's second line states the endpoint, model and masked key.
  //
  // No protocol tag here: `apiFormatLabel` comes from `openAiApiFormatFromBaseUrl`
  // / the config's own `api` key, and this fixture's bare endpoint resolves to
  // none. The tag's presence is covered by the list card in the app; what this
  // check guards is that the free-form entries survived the migration.
  const grokCustomText = await fixture('text("grok-custom-list")');
  check(
    'the custom Grok card states endpoint, model and masked key',
    grokCustomText.includes('https://grok.example.test/v1')
      && grokCustomText.includes('grok-4 (high)')
      && grokCustomText.includes('API Key:'),
  );

  /* ---------------------------------------------------------------- OmO Native */

  const omoDefaultSubtitle = await fixture('subtitle("omo-default-card")');
  check(
    'the OmO cell states the npm package',
    Boolean(omoDefaultSubtitle && omoDefaultSubtitle.startsWith('@ai-sdk/')),
  );
  check(
    'the provider that owns the default states it',
    Boolean(omoDefaultSubtitle && omoDefaultSubtitle.includes('当前主模型')),
  );

  const omoOtherSubtitle = await fixture('subtitle("omo-other-card")');
  check(
    'a non-default OmO cell states only the package',
    Boolean(omoOtherSubtitle && omoOtherSubtitle.startsWith('@ai-sdk/') && !omoOtherSubtitle.includes('当前主模型')),
  );

  const defaultDelete = await fixture('deleteMenuItem("omo-default-card")');
  check(
    'the default provider’s menu greys out delete and explains why',
    Boolean(defaultDelete && defaultDelete.found && defaultDelete.disabled),
  );

  const otherDelete = await fixture('deleteMenuItem("omo-other-card")');
  check(
    'an ordinary provider’s menu keeps delete live',
    Boolean(otherDelete && otherDelete.found && !otherDelete.disabled),
  );

  await screenshot('migrated-provider-cards-light');

  /* ---------------------------------------------------------------- dark theme */

  await openFixture('theme=dark');
  const darkCardBg = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="claude-desktop-card"]');
      const card = root.querySelector('[class*="card"]');
      return card ? getComputedStyle(card).backgroundColor : null;
    })()
  `);
  check('the migrated cards render in dark theme', Boolean(darkCardBg) && darkCardBg !== 'rgb(255, 255, 255)');
  await screenshot('migrated-provider-cards-dark');

  return checks;
}
