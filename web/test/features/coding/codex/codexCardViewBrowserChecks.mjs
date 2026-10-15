import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

/**
 * Drives Codex's provider list in the card layout with real clicks and real
 * pointer events, and asserts what the user can observe.
 *
 * Why a browser and not a unit test: every claim here is about the rendered
 * page. "The toggle switches the layout", "the cell states the primary model",
 * "the catalog followed the card into the sidebar", "dragging in the grid
 * reorders and writes back" — all of them span the page, the shared shell, the
 * mapping layer and the card. **Codex is where those claims are most likely to
 * be false**: unlike Claude it owns a model catalog and an official-account
 * block, and a card grid that simply dropped them would look completely
 * healthy.
 */
export async function verifyCodexCardView({ send, evaluate, baseUrl, artifactRoot }) {
  /** The fixture's key for the applied provider, long enough to overflow. */
  const rawApiKey = 'sk-codex-fixture-0123456789abcdefghijklmnopqrstuvwxyz-TAIL';
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
  const fixture = expression => evaluate('codexCardViewFixture.' + expression);
  const screenshot = async name => {
    const { data } = await send('Page.captureScreenshot', { format: 'png' });
    await writeFile(path.join(artifactRoot, name + '.png'), Buffer.from(data, 'base64'));
  };
  const clickAt = async ({ x, y }) => {
    await send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', clickCount: 1 });
    await send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', clickCount: 1 });
  };

  const openFixture = async (query) => {
    const runId = `${query}-${Date.now()}`;
    await send('Page.navigate', { url: `${baseUrl}/?${query}&runId=${encodeURIComponent(runId)}` });
    await waitFor(
      `window.codexCardViewFixture?.state.runId === ${JSON.stringify(runId)}`
        + ' && codexCardViewFixture.renderedOrder().length === 4',
      `fixture ${query}`,
    );
    await delay(300);
  };

  const toggleLayout = async () => {
    const center = await fixture('layoutToggleCenter()');
    assert.ok(center, 'the layout toggle must be on screen');
    await clickAt(center);
    await delay(300);
  };

  const openDetail = async (name) => {
    await clickAt(await fixture(`cardCenter(${JSON.stringify(name)})`));
    await waitFor('Boolean(codexCardViewFixture.detailPanel())', 'the detail sidebar');
    await delay(200);
  };

  /** A real pointer drag: dnd-kit's PointerSensor only activates past its
   *  `distance` constraint, so the move must overshoot it, in steps. */
  const dragCard = async (fromName, toName) => {
    const from = await fixture(`handleCenter(${JSON.stringify(fromName)})`);
    const to = await fixture(`cardCenter(${JSON.stringify(toName)})`);
    await send('Input.dispatchMouseEvent', { type: 'mousePressed', x: from.x, y: from.y, button: 'left', clickCount: 1 });
    const steps = 6;
    for (let step = 1; step <= steps; step++) {
      await send('Input.dispatchMouseEvent', {
        type: 'mouseMoved',
        x: from.x + ((to.x - from.x) * step) / steps,
        y: from.y + ((to.y - from.y) * step) / steps,
        button: 'left',
        buttons: 1,
      });
      await delay(30);
    }
    await send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: to.x, y: to.y, button: 'left', clickCount: 1 });
    await delay(300);
  };

  // --- the toggle exists, and switching to cards changes the layout --------
  await openFixture('viewMode=list');
  check('the toolbar carries a layout toggle', await fixture('layoutToggleCenter()') !== null);
  check('the toggle offers the card layout from the list', await fixture('layoutToggleMode()'), 'list');
  check('the toggle names the layout it switches to', await fixture('layoutToggleLabel()'), '卡片');
  check('the toggle does not reuse the common-config icon', await fixture('layoutToggleIconName()') !== 'appstore');
  check('the list layout renders no card grid', await evaluate(
    'Boolean(document.querySelector("[data-provider-grid]"))',
  ), false);
  // The list card states the endpoint and the catalog; the grid cell states
  // only the model, and everything else moves into the sidebar.
  check('the list card states the endpoint', await fixture('cardText("provider-a")')
    .then(text => text.includes('provider-a.example.test')));

  // --- only the sidebar's catalog is open ---------------------------------
  //
  // The two hosts draw the same section, and the detail sidebar is the one that
  // shows its rows outright (a sidebar is opened *to* read them). The list card
  // keeps the fold it has always had: cards are read side by side, and a card
  // that unfolded its whole catalog would be a different height from its
  // neighbours. Asserted as a pair — folded at rest, rows after the header is
  // clicked — because "no rows" alone would also pass on a broken section.
  const catalogOnListCard = `(() => {
    const card = document.querySelector('[data-provider-id="provider-a"] .ant-card');
    if (!card) return null;
    const header = [...card.querySelectorAll('.ant-collapse-header')]
      .find(node => node.textContent.includes('模型列表'));
    return {
      hasHeader: Boolean(header),
      rowsRendered: card.querySelectorAll('[data-model-row]').length,
    };
  })()`;
  const folded = await evaluate(catalogOnListCard);
  assert.ok(folded, 'the list card must render');
  check('the list card carries the catalog behind its own header', folded.hasHeader);
  check('the list card does not unfold the catalog', folded.rowsRendered, 0);

  await evaluate(`(() => {
    const card = document.querySelector('[data-provider-id="provider-a"] .ant-card');
    const header = [...card.querySelectorAll('.ant-collapse-header')]
      .find(node => node.textContent.includes('模型列表'));
    header.click();
    return true;
  })()`);
  await waitFor(
    `document.querySelectorAll('[data-provider-id="provider-a"] [data-model-row]').length > 0`,
    'the list card\'s catalog rows',
  );
  check(
    'opening the list card\'s catalog shows its rows',
    await evaluate(`document.querySelectorAll('[data-provider-id="provider-a"] [data-model-row]').length`),
    2,
  );

  // The rows inside a card are their own appearance, and it is the one the list
  // layout has always had: the card is wide enough for a grip on every row and
  // for the labelled actions to hold their width and fade in. That is the exact
  // opposite of the sidebar's row on every count (`appearance`), so each of the
  // three is worth a check — a single shared row that lost one of them would
  // look fine in a screenshot and be unreadable in a half-width card.
  const listRow = `(() => {
    const row = document.querySelector('[data-provider-id="provider-a"] [data-model-row]');
    const grip = row.querySelector('.anticon-holder');
    const name = row.querySelector('span[title] > span');
    return {
      nameFontSize: getComputedStyle(name).fontSize,
      gripWidth: grip ? Math.round(grip.parentElement.getBoundingClientRect().width) : 0,
      gripColor: grip ? getComputedStyle(grip.parentElement).color : null,
      labelledActionWidths: [...row.querySelectorAll('button')]
        .filter(button => button.textContent.trim().length > 0)
        .map(button => Math.round(button.getBoundingClientRect().width)),
      labelledActionFontSizes: [...row.querySelectorAll('button')]
        .filter(button => button.textContent.trim().length > 0)
        .map(button => getComputedStyle(button).fontSize),
    };
  })()`;
  const rowStyle = await evaluate(listRow);
  assert.ok(rowStyle, 'the list card\'s catalog must render a row');
  check('the list row keeps the card\'s type scale', rowStyle.nameFontSize, '13px');
  check('the list row keeps its grip on screen', rowStyle.gripWidth > 0);
  check('the list row\'s grip is the card grey, not the sidebar\'s', rowStyle.gripColor, 'rgb(187, 187, 187)');
  check(
    'the list row holds the width of its labelled actions',
    rowStyle.labelledActionWidths.length > 0
      && rowStyle.labelledActionWidths.every(width => width > 0),
  );
  check(
    'the list row keeps the button default for those actions',
    rowStyle.labelledActionFontSizes.length > 0
      && rowStyle.labelledActionFontSizes.every(size => size === '14px'),
  );
  await evaluate(`(() => {
    const card = document.querySelector('[data-provider-id="provider-a"] .ant-card');
    const header = [...card.querySelectorAll('.ant-collapse-header')]
      .find(node => node.textContent.includes('模型列表'));
    header.click();
    return true;
  })()`);
  await delay(300);

  await toggleLayout();
  await waitFor('Boolean(document.querySelector("[data-provider-grid]"))', 'the card grid');
  check('switching to cards renders a multi-column grid', await fixture('gridColumnCount()') > 1);
  check('the toggle now offers the list layout', await fixture('layoutToggleMode()'), 'card');
  check('the layout choice is persisted per module', await fixture('lastViewModeSave()'), {
    module: 'codex',
    mode: 'card',
  });
  await screenshot('codex-card-view-light');

  // --- the second line is the primary model, and nothing else -------------
  check(
    'a card states its primary model, with the reasoning effort',
    await fixture('subtitleText("provider-a")'),
    '主模型: gpt-5 (high)',
  );
  check(
    'a card whose config declares no model renders no second line',
    await fixture('subtitleText("provider-d")'),
    null,
  );
  check(
    'the grid cell does not state the endpoint',
    await fixture('cardText("provider-a")').then(text => text.includes('provider-a.example.test')),
    false,
  );
  // The catalog is a manager, not a label: it must not spill onto the cell.
  // Matched on its heading *and* on a model name — the sidebar's copy of the
  // section is open, so a cell that carried the rows without the title would
  // slip past a heading-only check.
  check(
    'the grid cell does not state the model catalog',
    await fixture('cardText("provider-a")').then(
      text => text.includes('模型列表') || text.includes('GPT-5 mini'),
    ),
    false,
  );
  check('the grid card keeps the Skills card type scale', await fixture('cardSummary("provider-a", "Provider Main")').then(
    summary => ({
      titleFontSize: summary.titleFontSize,
      subtitleFontSize: summary.subtitleFontSize,
      footerButtonFontSize: summary.footerButtonFontSize,
    }),
  ), { titleFontSize: '14px', subtitleFontSize: '11px', footerButtonFontSize: '11px' });
  // The grid stretches every card in a row to the tallest, and the card body
  // absorbs the difference — so a provider without a model line used to grow
  // 17px of slack directly above its footer, reading as "the divider drifted
  // away from the text" (reported 2026-10-12). The line is reserved whether or
  // not a card states one, which is what keeps the two heights equal.
  check(
    'a card without a model line is the same height as one with it',
    await evaluate(`(() => {
      const heightOf = (id) => Math.round(document.querySelector('[data-provider-id="' + id + '"]').getBoundingClientRect().height);
      return heightOf('provider-d') === heightOf('provider-a');
    })()`),
  );
  check(
    'the model line sits the same distance above the divider on both',
    await evaluate(`(() => {
      const gapAboveDivider = (id) => {
        const card = document.querySelector('[data-provider-id="' + id + '"]');
        const subtitle = card.querySelector('[data-provider-subtitle]');
        const footer = card.querySelector('[data-provider-card-footer]');
        return Math.round(footer.getBoundingClientRect().top - subtitle.getBoundingClientRect().bottom);
      };
      return gapAboveDivider('provider-a') === gapAboveDivider('provider-b');
    })()`),
  );

  // --- state tags: applied, and the official heading ----------------------
  check(
    'the applied card states 已应用 beside its title',
    await fixture('cardTagLabels("provider-a")'),
    ['已应用'],
  );
  // The heading of an official card is the account block's heading: the shared
// glyph before the shared title, with the count after — so the grid and the
// sidebar that opens from it read as one block.
  check(
    'the official provider is headed like the account block it opens',
    await fixture('cardText("provider-c")').then(text => text.includes('官方账号')),
  );
  check(
    'the official heading puts the link glyph before its title',
    await fixture('headingGlyphPrecedesTitle("provider-c", "官方账号")'),
  );
  check(
    'the official card renders the shared heading glyph at all',
    await fixture('cardIconNames("provider-c")').then(icons => icons.includes('link')),
  );
  check(
    'the official heading still carries its account count',
    await fixture('cardText("provider-c")').then(text => text.includes('(2)')),
  );
  // Its second line answers the question this row is about. The model line
  // ("which model does this run") belongs to a provider; the official row is
  // about the login, and the value is the label that login's own row in the
  // block carries — one account, one way of naming it.
  check(
    'the official channel states the login it is running on, not a model',
    await fixture('subtitleText("provider-c")'),
    '当前登录: first@example.invalid',
  );

  // --- the footer matches the list card's rules ---------------------------
  // Codex has no "launch CLI" action, so the cell's footer is the apply /
  // gateway set only.
  check(
    'the applied provider offers the gateway button, not 应用',
    await fixture('footerActions("provider-a")'),
    ['网关代理'],
  );
  check(
    'a non-applied provider offers 应用',
    await fixture('footerActions("provider-b")'),
    ['应用'],
  );

  // --- hover reveals the more button, and the menu keeps every entry -------
  check('the more button is hidden until hover', await fixture('moreButtonOpacity("provider-a")'), 0);
  const cardCenter = await fixture('cardCenter("provider-a")');
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: cardCenter.x, y: cardCenter.y });
  await delay(300);
  check('hovering the card reveals the more button', await fixture('moreButtonOpacity("provider-a")') > 0.9);
  await clickAt(await fixture('moreButtonCenter("provider-a")'));
  await delay(300);
  check(
    'the menu carries the same entries as the list card, plus the connectivity test',
    await fixture('openMenuItemLabels()'),
    ['启用配置已启用', '编辑', '复制', '分享', '模型测试', '删除'],
  );
  await evaluate('document.body.click()');
  await delay(200);
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 2, y: 2 });
  await delay(200);

  // --- the title is the endpoint link, as it is on the list card ----------
  const titleCenter = await fixture('titleCenter("Provider Main")');
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: titleCenter.x, y: titleCenter.y });
  await delay(400);
  const titleTooltip = await fixture('openTooltipText()');
  check('hovering the title names the origin it would open', titleTooltip?.includes('provider-a.example.test') === true);
  await clickAt(titleCenter);
  await delay(400);
  check(
    'clicking the title opens the provider origin',
    await fixture('openedUrls()'),
    ['https://provider-a.example.test'],
  );
  check('clicking the title does not open the sidebar', await evaluate(
    'Boolean(codexCardViewFixture.detailPanel())',
  ), false);

  // --- the body click opens the sidebar, and the catalog is inside it -----
  await openDetail('provider-a');
  check(
    'the sidebar lists the connection, model and notes sections',
    await fixture('detailSectionTitles()'),
    ['连接信息', '模型配置', '备注'],
  );
  check(
    'the sidebar states the endpoint the cell no longer shows',
    await fixture('detailTextIncludes("https://provider-a.example.test/v1")'),
  );
  check(
    'the sidebar names the wire protocol',
    await fixture('detailFieldText("apiFormat")'),
    'OpenAI Chat Completions',
  );
  check('the sidebar states the primary model', await fixture('detailFieldText("primaryModel")'), 'gpt-5 (high)');
  check(
    'the model catalog followed the card into the sidebar',
    await fixture('detailTextIncludes("模型列表 (2)")'),
  );
  await fixture('openDetailCatalog()');
  await delay(300);
  check(
    'opening it lists the provider\'s models, with their display names',
    await fixture('detailTextIncludes("GPT-5 mini")'),
  );
  // Codex's two auto-review affordances are part of the catalog, not of the
  // card: the line above the rows and the per-row "set as auto-review" button.
  // A sidebar rendering the catalog without them would still look right.
  check(
    'the sidebar catalog keeps the auto-review line',
    await fixture('detailTextIncludes("自动审批模型: gpt-5-mini")'),
  );
  check(
    'the sidebar catalog keeps the per-row auto-review action',
    await fixture('detailTextIncludes("已是自动审批模型")'),
  );
  // The row at rest, which is what the list looks like when you are reading it
  // rather than acting on it. Three things were wrong at once and each needs
  // its own assertion, because each has a different cause:
  //
  //  - The text was squeezed (29px, wrapped mid-word: "DeepSee / k V4.1 / Flash
  //    (d / eepseek-…"), because the hover-revealed actions held their full
  //    width while invisible.
  //  - The rows were indented past a grip that was only *drawn* under the
  //    pointer, so a model name did not line up with the 模型列表 title above it.
  //  - Hovering re-laid the row out, so the text jumped sideways as the pointer
  //    crossed it.
  //
  // The fix is that the grip and the labelled actions are out of the flow until
  // the pointer arrives, and that is asserted as a *pair*: at rest the row
  // starts at the title's edge and the text has room; on hover the same row
  // gives some back. A check that only looked at rest would pass on a row whose
  // hover state was broken, and one that only looked at hover would pass on a
  // row that always reserved the space.
  const rowGeometry = `(() => {
    const row = document.querySelector('[data-model-row]');
    if (!row) return null;
    const text = row.querySelector('span[title]');
    const grip = row.querySelector('.anticon-holder');
    const title = [...document.querySelectorAll('[data-model-list-section] *')]
      .find((el) => el.children.length === 0 && el.textContent.trim().startsWith('模型列表'));
    return {
      rowLeft: Math.round(row.getBoundingClientRect().left),
      titleLeft: title ? Math.round(title.getBoundingClientRect().left) : null,
      textLeft: Math.round(text.getBoundingClientRect().left),
      textWidth: Math.round(text.getBoundingClientRect().width),
      truncated: text.scrollWidth > text.getBoundingClientRect().width + 1,
      textHeight: Math.round(text.getBoundingClientRect().height),
      gripWidth: grip ? Math.round(grip.parentElement.getBoundingClientRect().width) : 0,
      labelledActionWidth: [...row.querySelectorAll('button')]
        .filter((b) => b.textContent.trim().length > 0)
        .reduce((total, b) => total + b.getBoundingClientRect().width, 0),
      labelledActionFontSizes: [...row.querySelectorAll('button')]
        .filter((b) => b.textContent.trim().length > 0)
        .map((b) => getComputedStyle(b).fontSize),
    };
  })()`;

  const hoverRow = async () => {
    const point = await evaluate(`(() => {
      const rect = document.querySelector('[data-model-row]').getBoundingClientRect();
      return { x: Math.round(rect.left + rect.width / 2), y: Math.round(rect.top + rect.height / 2) };
    })()`);
    await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: point.x, y: point.y });
    await delay(300);
  };
  const parkPointer = async () => {
    await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 2, y: 2 });
    await delay(300);
  };

  await parkPointer();
  const rest = await evaluate(rowGeometry);
  check(
    'a model row lines up with the list title above it at rest',
    rest.rowLeft,
    rest.titleLeft,
  );
  check(
    'the grip takes no space until the row is hovered',
    rest.gripWidth,
    0,
  );
  check(
    'the labelled actions take no space until the row is hovered',
    rest.labelledActionWidth,
    0,
  );
  check(
    'a model row keeps its name, id and limits on one line',
    rest.truncated,
    false,
  );
  check(
    'the model text is given the row, not the leftovers',
    rest.textWidth >= 400,
  );
  check(
    'a model row is one text line tall, not a wrapped block',
    rest.textHeight < 30,
  );

  await hoverRow();
  const hovered = await evaluate(rowGeometry);
  check('hovering the row reveals the grip', hovered.gripWidth > 0);
  check('hovering the row reveals the labelled actions', hovered.labelledActionWidth > 0);
  // The pair of labels is the widest thing on this row, so its type is the
  // row's rather than the button default: at 14px they read louder than the
  // model name they act on (user-directed, 2026-10-13).
  check(
    'the labelled actions run at the row\'s type scale, not the button default',
    hovered.labelledActionFontSizes.length > 0
      && hovered.labelledActionFontSizes.every((size) => size === '12px'),
  );
  check(
    'the text yields room to the actions rather than wrapping',
    hovered.textWidth < rest.textWidth && hovered.truncated === false,
  );
  await parkPointer();
  // The catalog is a working manager, not a read-only list: its toolbar came
  // with it.
  check(
    'the sidebar catalog keeps its manager actions',
    await evaluate('(() => {'
      + ' const panel = codexCardViewFixture.detailPanel();'
      + ' const labels = [...panel.querySelectorAll("button")].map(b => b.textContent.trim());'
      + ' return ["批量删除", "模型测试", "获取模型", "添加模型"].filter(l => labels.includes(l)).length;'
      + '})()'),
    4,
  );
  // The counterpart of the official block below: a custom provider has no
  // accounts, so it gets no block — and the sidebar is where that has to be
  // true, since the grid cell renders no footer at all.
  check(
    'a custom provider gets no account block',
    await fixture('detailTextIncludes("账号列表")'),
    false,
  );
  check('the sidebar footer keeps the quieter actions', await fixture('detailFooterActions()'), [
    '模型测试', '编辑', '复制', '分享', '删除',
  ]);
  check(
    "the sidebar state line carries the card's action row",
    await fixture('detailStatusActions()'),
    ['网关代理'],
  );
  // The same tag row the grid cell states, because it *is* that row.
  check(
    "the sidebar states the cell's state tags verbatim",
    await fixture('detailStatusTagLabels()'),
    await fixture('cardTagLabels("provider-a")'),
  );
  await screenshot('codex-card-view-detail');

  // --- the credential: elided on screen, whole on the clipboard ----------
  const keyLine = await fixture('detailFieldText("apiKey")');
  check('the sidebar elides a key that does not fit one line', keyLine?.includes('…') === true);
  check(
    'the elided key keeps both identifying ends',
    keyLine.startsWith('sk-codex-fixture-01') && keyLine.endsWith('-TAIL'),
  );
  check('the sidebar offers copy buttons on both connection fields', await fixture(
    'detailCopyFields()',
  ), ['baseUrl', 'apiKey']);
  await fixture('copyDetailField("apiKey")');
  await delay(200);
  check('copying the key yields the whole value', await fixture('clipboardWrites()'), [rawApiKey]);
  // Each copy raises an antd "已复制" toast at the top of the window, over the
  // toolbar the checks below click. Wait them out.
  await waitFor('!document.querySelector(".ant-message-notice")', 'the copy toast to clear');

  await fixture('closeDetail()');
  await waitFor('!codexCardViewFixture.detailPanel()', 'the sidebar to close');

  // --- the official provider's accounts live in its sidebar --------------
  await openDetail('provider-c');
  // The block is always open, so its title and its rows are both on screen at
  // once — and the rows are the evidence that it arrived *with* its accounts,
  // which a title-only check would not see. Scoped to the sidebar, whose host
  // cell cannot carry them (the grid cell renders no footer).
  check(
    'the official account block followed the card into the sidebar',
    await fixture('detailTextIncludes("账号列表")'),
  );
  check(
    'it brought its saved accounts with it',
    await fixture('detailTextIncludes("first@example.invalid")'),
  );
  check(
    'the official sidebar offers no catalog — that provider has none',
    await fixture('detailTextIncludes("模型列表")'),
    false,
  );
  // The one sidebar a check cannot fully describe: it is the case where the
  // catalog is absent and the account block is present, and the assertions
  // above read it only through text.
  await screenshot('codex-card-view-official-detail');
  await fixture('closeDetail()');
  await waitFor('!codexCardViewFixture.detailPanel()', 'the sidebar to close');

  // --- locate still finds a card in the grid ------------------------------
  check(
    'grid cards keep the locate contract',
    await evaluate('Boolean(document.querySelector(\'[data-provider-id="provider-a"]\'))'),
  );

  // --- dragging reorders in the grid, and writes back ---------------------
  const beforeDrag = await fixture('renderedOrder()');
  await dragCard('provider-d', 'provider-a');
  const afterDrag = await fixture('waitForOrder(["provider-d", "provider-a", "provider-b", "provider-c"])');
  check('dragging in the grid moves the card', afterDrag[0], 'provider-d');
  check('the new order is not the old one', afterDrag.join('|') !== beforeDrag.join('|'));
  check('the drag writes the order back', await fixture('lastReorder()'), [
    'provider-d',
    'provider-a',
    'provider-b',
    'provider-c',
  ]);
  await screenshot('codex-card-view-after-drag');

  // --- switching back restores the list layout ----------------------------
  await toggleLayout();
  check('switching back removes the grid', await evaluate(
    'Boolean(document.querySelector("[data-provider-grid]"))',
  ), false);
  check('the list layout renders every provider again', await fixture('renderedOrder()').then(
    order => order.length,
  ), 4);
  check(
    'the list layout still carries the catalog',
    await fixture('cardText("provider-a")').then(text => text.includes('模型列表 (2)')),
  );

  // --- dark theme keeps the card readable ---------------------------------
  await openFixture('viewMode=card&theme=dark');
  check(
    'the card renders in dark theme',
    await evaluate('document.documentElement.dataset.theme'),
    'dark',
  );
  await screenshot('codex-card-view-dark');

  // --- the login line answers in every state ------------------------------
  //
  // The cell's second line is reserved whether or not it states something, so a
  // channel that has accounts but none in use left the band blank — which reads
  // as a broken card, not as "nothing to say" (user-reported 2026-10-14, on two
  // idle channels). The line now answers the question it exists for in all three
  // states: the login the channel runs on, the login only the CLI's own file
  // holds, or 「未登录」 when there is neither.
  await openFixture('viewMode=card&accounts=virtual');
  check(
    'a login only the CLI file holds is stated as the current login',
    await fixture('subtitleText("provider-c")'),
    '当前登录: live@example.invalid',
  );

  await openFixture('viewMode=card&accounts=idle');
  check(
    'a channel with no login in use says so instead of leaving the line empty',
    await fixture('subtitleText("provider-c")'),
    '当前登录: 未登录',
  );
  check(
    'the idle channel still states how many accounts it holds',
    await fixture('cardText("provider-c")').then(text => text.includes('(2)')),
  );
  await screenshot('codex-card-view-idle-channel');

  return checks;
}