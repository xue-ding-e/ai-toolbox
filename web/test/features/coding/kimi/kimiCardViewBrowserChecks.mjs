import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

/**
 * Drives Kimi's provider list in the card layout with real clicks and real
 * pointer events, and asserts what the user can observe.
 *
 * Why a browser and not a unit test: every claim here is about the rendered
 * page — the toggle switches the layout, the cell states the default model, the
 * catalog and the account block followed the card into the sidebar, dragging in
 * the grid reorders and writes back. All of them span the page, the shared
 * shell, the mapping layer and the compact card, and **Kimi is where one of
 * them is likeliest to be false**: its official channel is an ordinary provider
 * row whose card carries the account block, so a grid that dropped the footer
 * would leave the accounts with nowhere to be.
 */
export async function verifyKimiCardView({ send, evaluate, baseUrl, artifactRoot }) {
  const checks = [];
  const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
  const check = (name, actual, expected = true) => {
    assert.deepEqual(actual, expected, name);
    checks.push(name);
    console.log('PASS ' + name);
  };
  // 50s, not 10s — a full `pnpm test` run keeps the machine busy enough that a
  // short budget fails on boot time alone.
  const waitFor = async (expression, label) => {
    for (let attempt = 0; attempt < 400; attempt++) {
      if (await evaluate(expression)) return;
      await delay(125);
    }
    throw new Error('Timed out waiting for: ' + (label || expression));
  };
  const fixture = expression => evaluate('kimiOfficialAccountFixture.' + expression);
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
      `window.kimiOfficialAccountFixture?.state.runId === ${JSON.stringify(runId)}`
        + ' && kimiOfficialAccountFixture.renderedMemberIds().length === 4',
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

  const OFFICIAL_ID = 'provider-official';

  // --- the toggle exists, and switching to cards changes the layout --------
  await openFixture('viewMode=list');
  check('the toolbar carries a layout toggle', await fixture('layoutToggleCenter()') !== null);
  check('the toggle offers the card layout from the list', await fixture('layoutToggleMode()'), 'list');
  check('the list layout renders no card grid', await evaluate(
    'Boolean(document.querySelector("[data-provider-grid]"))',
  ), false);
  // The list card states the endpoint on its second line; the cell must not —
  // that line is where the default model goes. The protocol row moved into the
  // sidebar with the rest of the provider's facts.
  check(
    'the list card states the endpoint',
    await fixture('cardText("provider-a")')
      .then(text => text.includes('https://provider-a.example.test/v1')),
  );
  // And the list card's catalog is still the folded one it has always been: the
  // sidebar's copy is the open one, and only that one.
  check(
    'the list card keeps its catalog folded',
    await evaluate('(() => {'
      + ' const card = document.querySelector(\'[data-provider-id="provider-a"]\');'
      + ' return Boolean(card?.querySelector(".ant-collapse-header"))'
      + '   && card.querySelectorAll("[data-model-row]").length === 0;'
      + '})()'),
  );

  await toggleLayout();
  await waitFor('Boolean(document.querySelector("[data-provider-grid]"))', 'the card grid');
  check('switching to cards renders a multi-column grid', await fixture('gridColumnCount()') > 1);
  check('the toggle now offers the list layout', await fixture('layoutToggleMode()'), 'card');
  check('the layout choice is persisted per module', await fixture('lastViewModeSave()'), {
    module: 'kimi',
    mode: 'card',
  });
  await screenshot('kimi-card-view-light');

  // --- what the cell states ------------------------------------------------
  check(
    'a cell states its default model',
    await fixture('subtitleText("provider-a")'),
    '默认模型: provider-a-model-a',
  );
  check(
    'the cell does not repeat the endpoint',
    await fixture('cardText("provider-a")')
      .then(text => text.includes('provider-a.example.test')),
    false,
  );
  // One state, one wording: the cell states the applied fact through the same
  // `nameTags` fragment the list card draws.
  check(
    'the grid cell states the applied tag the list card states',
    await fixture('cardTagLabels("provider-a")'),
    ['已应用'],
  );

  // --- the official channel is still an ordinary cell ----------------------
  check(
    'the official cell is headed by the shared account heading',
    await fixture(`cardText(${JSON.stringify(OFFICIAL_ID)})`)
      .then(text => text.includes('官方账号') && text.includes('(2)')),
  );
  check(
    'the heading glyph is in front of the words, not after them',
    await fixture(`hasHeadingGlyph(${JSON.stringify(OFFICIAL_ID)})`),
  );
  check(
    'the official channel states no model of its own',
    await fixture(`subtitleText(${JSON.stringify(OFFICIAL_ID)})`).then(text => Boolean(text?.includes('默认模型'))),
    false,
  );
  // It states the login instead: the accounts are what this row is about, and
  // the value is the label the account's own row carries in the block below.
  check(
    'the official channel states the login it is running on',
    await fixture(`subtitleText(${JSON.stringify(OFFICIAL_ID)})`),
    '当前登录: moonwalker',
  );

  // --- the body click opens the sidebar, and the catalog is inside it ------
  await clickAt(await fixture('cardCenter("provider-a")'));
  await waitFor('Boolean(kimiOfficialAccountFixture.detailPanel())', 'the detail sidebar');
  await delay(200);
  check(
    'a body click opens the sidebar',
    await evaluate('Boolean(kimiOfficialAccountFixture.detailPanel())'),
  );
  check(
    'the sidebar states the connection fields',
    await fixture('detailSectionTitles()').then(titles => titles.includes('连接信息')),
  );
  check(
    'the sidebar states the endpoint',
    await fixture('detailTextIncludes("https://provider-a.example.test/v1")'),
  );
  // The stored key travels to the sidebar in full: a truncated credential is
  // one you cannot check against the console it came from.
  check(
    'the sidebar states the whole stored API key',
    await fixture('detailTextIncludes("fixture-key-provider-a")'),
  );
  check(
    'the model catalog followed the card into the sidebar',
    await fixture('openDetailCatalog()') && await fixture('detailTextIncludes("模型列表 (1)")'),
  );
  check(
    'the sidebar catalog keeps its manager actions',
    await evaluate('(() => {'
      + ' const panel = kimiOfficialAccountFixture.detailPanel();'
      + ' const labels = [...panel.querySelectorAll("button")].map(b => b.textContent.trim());'
      + ' return ["批量删除", "模型测试", "获取模型", "添加模型"].filter(l => labels.includes(l)).length;'
      + '})()'),
    4,
  );
  check(
    'the sidebar marks the default model on its own row',
    await fixture('detailTextIncludes("当前主模型")'),
  );
  await screenshot('kimi-card-view-detail');
  await fixture('closeDetail()');
  // The drawer animates out and unmounts late, so the next sidebar's `waitFor`
  // would otherwise be satisfied by the *closing* one — and every assertion
  // after it would be reading the previous provider's panel.
  await waitFor('!kimiOfficialAccountFixture.detailPanel()', 'the sidebar to close');

  // --- the official channel's accounts are in *its* sidebar ----------------
  await clickAt(await fixture(`cardCenter(${JSON.stringify(OFFICIAL_ID)})`));
  await waitFor(
    'kimiOfficialAccountFixture.detailTextIncludes("账号列表")',
    'the official sidebar',
  );
  await delay(200);
  check(
    'the official sidebar carries the account block',
    await fixture('detailTextIncludes("账号列表")'),
  );
  // Its rows came with it, and they are *open*: the sidebar's copy draws no
  // caret, and the account rows are the reason this sidebar was opened.
  // Counted inside the panel rather than in the card — in this layout the block
  // is not in the cell at all, which is the whole reason it had to follow.
  check(
    'showing both accounts open, with no caret to open them',
    await evaluate('(() => {'
      + ' const panel = kimiOfficialAccountFixture.detailPanel();'
      + ' const rows = [...panel.querySelectorAll("div")].filter(node =>'
      + '   String(node.style?.borderBottom || "").includes("1px solid"));'
      + ' return rows.length === 2'
      + '   && !panel.querySelector(".ant-collapse-header")'
      + '   && panel.textContent.includes("moonwalker")'
      + '   && panel.textContent.includes("nightowl");'
      + '})()'),
  );
  check(
    'and the cell no longer carries the block it handed over',
    await fixture(`accountRowCount(${JSON.stringify(OFFICIAL_ID)})`),
    0,
  );
  check(
    'and no catalog — that provider has none',
    await fixture('detailTextIncludes("模型列表")'),
    false,
  );
  await screenshot('kimi-card-view-official-detail');
  await fixture('closeDetail()');
  await delay(300);

  // --- dragging in the grid reorders and writes back -----------------------
  const dragCard = async (fromId, toId) => {
    const from = await fixture(`handleCenter(${JSON.stringify(fromId)})`);
    const to = await fixture(`cardCenter(${JSON.stringify(toId)})`);
    const onScreen = await evaluate(`(() => {
      const point = ${JSON.stringify(to)};
      return point.y > 0 && point.y < window.innerHeight && point.x > 0 && point.x < window.innerWidth;
    })()`);
    assert.ok(onScreen, `the drop point must be on screen: ${JSON.stringify(to)}`);
    // dnd-kit's PointerSensor only activates past its `distance` constraint, so
    // the move has to overshoot it, in steps.
    await send('Input.dispatchMouseEvent', { type: 'mousePressed', x: from.x, y: from.y, button: 'left', clickCount: 1 });
    const steps = 6;
    for (let step = 1; step <= steps; step++) {
      await send('Input.dispatchMouseEvent', {
        type: 'mouseMoved',
        x: from.x + ((to.x - from.x) * step) / steps,
        y: from.y + ((to.y - from.y) * step) / steps,
        button: 'left',
      });
      await delay(60);
    }
    await send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: to.x, y: to.y, button: 'left', clickCount: 1 });
    await delay(400);
  };

  const orderBefore = await fixture('renderedMemberIds()');
  await dragCard('provider-b', 'provider-c');
  const orderAfter = await fixture('renderedMemberIds()');
  check('dragging in the grid moves the card', orderAfter.join('|') !== orderBefore.join('|'));
  check('the drag writes the order back', await fixture('lastReorder()') !== null);
  check(
    'the payload is the order the user sees',
    await fixture('lastReorder()').then(ids => ids && ids.join('|') === orderAfter.join('|')),
  );
  await screenshot('kimi-card-view-dragged');

  // --- switching back restores the list layout ----------------------------
  await toggleLayout();
  check('switching back removes the grid', await evaluate(
    'Boolean(document.querySelector("[data-provider-grid]"))',
  ), false);
  check('the list layout renders every member again', await fixture('renderedMemberIds()').then(
    ids => ids.length,
  ), 4);
  check(
    'and its catalog is folded again, as it was',
    await evaluate('(() => {'
      + ' const card = document.querySelector(\'[data-provider-id="provider-a"]\');'
      + ' return card.querySelectorAll("[data-model-row]").length === 0;'
      + '})()'),
  );

  // --- dark theme keeps the grid readable ---------------------------------
  await openFixture('viewMode=card&theme=dark');
  check(
    'the grid renders in dark theme',
    await evaluate('document.documentElement.dataset.theme'),
    'dark',
  );
  check('the dark grid is still a grid', await fixture('gridColumnCount()') > 1);
  await screenshot('kimi-card-view-dark');

  // --- the login line answers in every state ------------------------------
  //
  // The cell's second line is reserved whether or not it states something, so a
  // channel that has accounts but none in use left the band blank — which reads
  // as a broken card rather than as "nothing to say" (user-reported 2026-10-14,
  // on an idle channel). It now answers the question it exists for in every
  // state: the login in use, the login only the CLI's credential file holds, or
  // 未登录 when there is neither.
  await openFixture('viewMode=card&accounts=virtual');
  check(
    'a login only the CLI file holds is stated as the current login',
    await fixture(`subtitleText(${JSON.stringify(OFFICIAL_ID)})`),
    '当前登录: moonwalker',
  );

  await openFixture('viewMode=card&accounts=idle');
  check(
    'a channel with no login in use says so instead of leaving the line empty',
    await fixture(`subtitleText(${JSON.stringify(OFFICIAL_ID)})`),
    '当前登录: 未登录',
  );
  check(
    'the idle channel still states how many accounts it holds',
    await fixture(`cardText(${JSON.stringify(OFFICIAL_ID)})`).then(text => text.includes('(2)')),
  );

  return checks;
}