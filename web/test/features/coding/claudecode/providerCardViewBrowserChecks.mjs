import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

/**
 * Drives the provider list's card layout with real clicks and real pointer
 * events in a browser, and asserts what the user can observe.
 *
 * Why a browser and not a unit test: every claim here is about the rendered
 * page. "The toggle switches the layout", "the card states the Opus model",
 * "the applied card offers the gateway button instead of 应用", "the more
 * button appears on hover", "dragging in the grid reorders and writes back" —
 * all of them span the page, the shared shell, the mapping layer and the card.
 * The repo has hit this class twice: a control that renders and does nothing,
 * and a card whose buttons silently differed from its sibling's.
 */
export async function verifyProviderCardView({ send, evaluate, baseUrl, artifactRoot }) {
  /** The fixture's key for the applied provider, long enough to overflow. */
  const rawApiKey = 'sk-ant-fixture-0123456789abcdefghijklmnopqrstuvwxyz-TAIL';
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
  const fixture = expression => evaluate('providerCardViewFixture.' + expression);
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
    // Encoded: the run id travels as a query parameter the fixture reads back.
    await send('Page.navigate', { url: `${baseUrl}/?${query}&runId=${encodeURIComponent(runId)}` });
    await waitFor(
      `window.providerCardViewFixture?.state.runId === ${JSON.stringify(runId)}`
        + ' && providerCardViewFixture.renderedOrder().length === 3',
      `fixture ${query}`,
    );
    await delay(300);
  };

  /** Clicks the toolbar's layout toggle (a single icon button). */
  const toggleLayout = async () => {
    const center = await fixture('layoutToggleCenter()');
    assert.ok(center, 'the layout toggle must be on screen');
    await clickAt(center);
    await delay(300);
  };

  /** Clicks the card body, which is the detail sidebar's entry point. */
  const openDetail = async (name) => {
    await clickAt(await fixture(`cardCenter(${JSON.stringify(name)})`));
    await waitFor('Boolean(providerCardViewFixture.detailPanel())', 'the detail sidebar');
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
  // The toggle used to draw lucide's `LayoutGrid` — a 2x2 of squares, which is
  // exactly `AppstoreOutlined`. Sitting next to 通用配置, it read as a second
  // copy of that button.
  check(
    'the toggle does not reuse the common-config icon',
    await fixture('layoutToggleIconName()') !== await fixture('commonConfigIconName()'),
  );
  check('the common-config button is still the appstore glyph for comparison', await fixture(
    'commonConfigIconName()',
  ), 'appstore');
  check('the list layout renders no card grid', await evaluate(
    'Boolean(document.querySelector("[data-provider-grid]"))',
  ), false);

  await toggleLayout();
  await waitFor('Boolean(document.querySelector("[data-provider-grid]"))', 'the card grid');
  check('switching to cards renders a multi-column grid', await fixture('gridColumnCount()') > 1);
  check('the toggle now offers the list layout', await fixture('layoutToggleMode()'), 'card');
  check('the toggle word follows the layout it offers', await fixture('layoutToggleLabel()'), '列表');
  check('the layout choice is persisted per module', await fixture('lastViewModeSave()'), {
    module: 'claudecode',
    mode: 'card',
  });
  await screenshot('card-view-light');

  // --- the second line lists every configured role model -------------------
  // `Role: value` pairs, in tier order, skipping the roles with no model.
  check(
    'a card states every configured role model, in tier order',
    await fixture('waitForSubtitle("Provider Opus", "Opus: claude-opus-4-8  Sonnet: claude-sonnet-4-6")'),
    'Opus: claude-opus-4-8  Sonnet: claude-sonnet-4-6',
  );
  check(
    'a card with only Sonnet states just that role',
    await fixture('waitForSubtitle("Provider Sonnet Only", "Sonnet: claude-sonnet-4-6")'),
    'Sonnet: claude-sonnet-4-6',
  );
  check(
    'a card with no role models renders no second line',
    await fixture('subtitleText("Provider No Models")'),
    null,
  );
  // The pairs are separated by a hairline bar, drawn as a pseudo-element so it
  // is punctuation between two facts rather than a third one.
  check(
    'adjacent model pairs are separated by a bar',
    await fixture('subtitleSeparator("Provider Opus")'),
    '"|"',
  );
  check(
    'a single-pair line draws no bar',
    await fixture('subtitleSeparator("Provider Sonnet Only")'),
    null,
  );

  // --- the card's three zones sit at the Skills card's type scale ----------
  // `SkillCard.module.less` sets the title at 14px/600 and its whole meta bar
  // at 11px; the grid card has to match, since it is the same card object on a
  // different page.
  check('the card type scale matches the Skills card', await fixture('cardTypeScale("Provider Opus")'), {
    title: '14px',
    titleWeight: '600',
    subtitle: '11px',
    footerButton: '11px',
    tag: '10px',
  });

  // --- the card no longer states the endpoint next to the title ------------
  check(
    'the card title carries no address',
    await fixture('cardText("Provider Opus")').then(text => text.includes('provider-a.example.test')),
    false,
  );

  // --- the applied state is stated, not only coloured ---------------------
  // The card's blue border says something about this provider; only a tag says
  // what. Without it the grid was the one layout where "已应用" was invisible.
  check(
    'the applied card states 已应用 beside its title',
    await fixture('cardTagLabels("Provider Opus")'),
    ['已应用'],
  );
  check(
    'a card that is not applied states no tag',
    await fixture('cardTagLabels("Provider Sonnet Only")'),
    [],
  );

  // --- the title is the endpoint link, as it is on the list card ----------
  const titleCenter = await fixture('titleCenter("Provider Opus")');
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: titleCenter.x, y: titleCenter.y });
  await delay(400);
  const titleTooltip = await fixture('openTooltipText()');
  check('hovering the title names the origin it would open', titleTooltip?.includes('provider-a.example.test') === true);
  check('nothing is opened merely by hovering', await fixture('openedUrls()'), []);
  await clickAt(titleCenter);
  await delay(400);
  check(
    'clicking the title opens the provider origin',
    await fixture('openedUrls()'),
    ['https://provider-a.example.test'],
  );
  // The link is a control: it must not also cover the list with the sidebar.
  check('clicking the title does not open the sidebar', await evaluate(
    'Boolean(providerCardViewFixture.detailPanel())',
  ), false);
  // Park the pointer off the card: the checks below read the hover-revealed
  // "more" button, which is only hidden while nothing is hovered.
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 2, y: 2 });
  await delay(300);

  // --- the footer buttons match the list card's rules ----------------------
  check(
    'the applied provider offers the gateway button, not 应用',
    await fixture('footerActions("Provider Opus")'),
    ['CLI 启动', '网关代理'],
  );
  check(
    'a non-applied provider offers 应用',
    await fixture('footerActions("Provider Sonnet Only")'),
    ['CLI 启动', '应用'],
  );

  // --- hover reveals the more button --------------------------------------
  check('the more button is hidden until hover', await fixture('moreButtonOpacity("Provider Opus")'), 0);
  const cardCenter = await fixture('cardCenter("Provider Opus")');
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: cardCenter.x, y: cardCenter.y });
  await delay(300);
  check('hovering the card reveals the more button', await fixture('moreButtonOpacity("Provider Opus")') > 0.9);

  const moreCenter = await fixture('moreButtonCenter("Provider Opus")');
  await clickAt(moreCenter);
  await delay(300);
  check(
    'the menu carries the same entries as the list card, plus the connectivity test',
    await fixture('openMenuItemLabels()'),
    // The enable row renders its state sub-label inside the same item, so its
    // text is 启用 + the state sentence rather than 启用 alone.
    ['启用配置已启用', '编辑', '复制', '分享', '模型测试', '删除'],
  );
  await evaluate('document.body.click()');
  await delay(200);

  // --- clicking the card body opens the detail sidebar ---------------------
  await openDetail('Provider Opus');
  check(
    'the sidebar states the endpoint the card no longer shows',
    await fixture('detailTextIncludes("https://provider-a.example.test")'),
  );
  // The key is no longer masked with a fixed 6…4 head/tail — the whole value is
// there, shortened only when it does not fit one line, and shortened in the
// middle so both identifying ends survive.
  const keyLine = await fixture('detailKeyLine()');
  check('the sidebar elides a key that does not fit one line', keyLine?.includes('…') === true);
  check(
    'the elided key keeps both identifying ends',
    keyLine.startsWith('sk-ant-fixture-01') && keyLine.endsWith('-TAIL'),
  );
  check('the elided line is shorter than the key itself', keyLine.length < rawApiKey.length);
  check(
    'the sidebar names the wire protocol the card never shows',
    await fixture('detailTextIncludes("OpenAI Chat Completions")'),
  );
  check(
    'the sidebar lists the model mapping and the connection sections',
    await fixture('detailSectionTitles()'),
    ['连接信息', '模型映射'],
  );
  // The endpoint and the key are both long strings that exist to be pasted
  // somewhere else. What matters is not that a button is rendered but what it
  // puts on the clipboard, so the fixture records the writes.
  check(
    'the connection fields each carry a copy button',
    await fixture('detailCopyFields()'),
    ['baseUrl', 'apiKey'],
  );
  await fixture('copyDetailField("baseUrl")');
  await delay(200);
  await fixture('copyDetailField("apiKey")');
  await delay(200);
  check(
    'copying the fields yields the full values, not the elided ones',
    await fixture('clipboardWrites()'),
    ['https://provider-a.example.test', rawApiKey],
  );
  // Each copy raises an antd "已复制" toast, and those sit at the top of the
  // window — directly over the toolbar the checks below click. Waiting them out
  // is the difference between driving the page and driving whatever happens to
  // be painted on top of it.
  await waitFor('!document.querySelector(".ant-message-notice")', 'the copy toasts to clear');
  // The sidebar's premise: it offers what the card offers, because both render
  // the same computed actions through the same two renderers. The card's own
  // row sits on the title's state line; the footer keeps the quieter set.
  check(
    "the sidebar state line carries the card's action row",
    await fixture('detailStatusActions()'),
    ['CLI 启动', '网关代理'],
  );
  // The same tag row the grid cell states, because it *is* that row — the
  // sidebar used to list three of the tags by hand and would have gone on
  // saying nothing about 代理 / 绕过 / P0.
  check(
    "the sidebar states the cell's state tags verbatim",
    await fixture('detailStatusTagLabels()'),
    await fixture('cardTagLabels("Provider Opus")'),
  );
  check(
    'the sidebar footer keeps the quieter actions, each with an icon',
    await fixture('detailFooterActions()'),
    ['模型测试', '编辑', '复制', '分享', '删除'],
  );
  check(
    'the sidebar footer runs at the card footer scale',
    await fixture('detailFooterButtonFontSize()'),
    '11px',
  );
  await screenshot('card-view-detail');

  // A control inside the card must not open the sidebar — otherwise clicking
  // 应用 would also cover the list with a drawer.
  await fixture('closeDetail()');
  await waitFor('!providerCardViewFixture.detailPanel()', 'the sidebar to close');
  const moreCenterForGuard = await fixture('moreButtonCenter("Provider Opus")');
  await clickAt(moreCenterForGuard);
  await delay(300);
  check('clicking a card control does not open the sidebar', await evaluate(
    'Boolean(providerCardViewFixture.detailPanel())',
  ), false);
  await evaluate('document.body.click()');
  await delay(200);

  // --- batch-selection mode reserves the body click ------------------------
  // In selection mode a body click means "select this card"; opening the
  // sidebar instead would make selecting a run of cards impossible.
  check('the card body is a click target in browse mode', await evaluate(
    'Boolean(document.querySelector("[data-provider-card-clickable]"))',
  ));
  await evaluate('document.querySelector("[data-provider-card-clickable]").click()');
  await delay(300);
  check('a plain body click opens the sidebar', await evaluate(
    'Boolean(providerCardViewFixture.detailPanel())',
  ));
  await fixture('closeDetail()');
  await waitFor('!providerCardViewFixture.detailPanel()', 'the sidebar to close');

  // Matched by either label (多选 / 退出 — the control renames itself once
  // selection is on) and re-measured on every click: entering selection mode
  // also adds the batch toolbar's own controls to the same row, which shifts
  // this button.
  //
  // A DOMRect does not survive `returnByValue`; return plain numbers.
  const batchButtonCenter = () => evaluate(
    '(() => {'
      + ' const b = [...document.querySelectorAll("button")]'
      + '   .find(x => ["多选", "退出"].includes(x.textContent.trim()));'
      + ' if (!b) return null;'
      + ' const r = b.getBoundingClientRect();'
      + ' return { x: r.left + r.width / 2, y: r.top + r.height / 2 };'
      + ' })()',
  );
  await clickAt(await batchButtonCenter());
  await delay(300);
  check('entering batch selection removes the body click target', await evaluate(
    'Boolean(document.querySelector("[data-provider-card-clickable]"))',
  ), false);
  await clickAt(await fixture('cardCenter("Provider Opus")'));
  await delay(300);
  check('in selection mode the body click does not open the sidebar', await evaluate(
    'Boolean(providerCardViewFixture.detailPanel())',
  ), false);
  // Leave selection mode so the later checks see the browse-mode layout.
  await clickAt(await batchButtonCenter());
  await delay(300);
  check('leaving selection mode restores the body click target', await evaluate(
    'Boolean(document.querySelector("[data-provider-card-clickable]"))',
  ));

  // --- locate still finds a card in the grid ------------------------------
  check(
    'grid cards keep the locate contract',
    await evaluate('Boolean(document.querySelector(\'[data-provider-id="provider-a"]\'))'),
  );

  // --- dragging reorders in the grid, and writes back ---------------------
  const beforeDrag = await fixture('renderedOrder()');
  await dragCard('Provider No Models', 'Provider Opus');
  const afterDrag = await fixture('waitForOrder(["Provider No Models", "Provider Opus", "Provider Sonnet Only"])');
  check('dragging in the grid moves the card', afterDrag[0], 'Provider No Models');
  check('the new order is not the old one', afterDrag.join('|') !== beforeDrag.join('|'));
  check('the drag writes the order back', await fixture('lastReorder()'), [
    'provider-c',
    'provider-a',
    'provider-b',
  ]);
  await screenshot('card-view-after-drag');

  // --- switching back restores the list layout ----------------------------
  await toggleLayout();
  check('switching back removes the grid', await evaluate(
    'Boolean(document.querySelector("[data-provider-grid]"))',
  ), false);
  check('the list layout renders every provider again', await fixture('renderedOrder()').then(
    order => order.length,
  ), 3);

  // --- dark theme keeps the card readable ---------------------------------
  await openFixture('viewMode=card&theme=dark');
  check(
    'the card renders in dark theme',
    await evaluate('document.documentElement.dataset.theme'),
    'dark',
  );
  await screenshot('card-view-dark');

  return checks;
}
