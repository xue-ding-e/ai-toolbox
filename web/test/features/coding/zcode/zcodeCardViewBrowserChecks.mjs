import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

/**
 * Drives ZCode's provider list in the card layout with real clicks and real
 * pointer events, and asserts what the user can observe.
 *
 * Why a browser and not a unit test: every claim here is about the rendered
 * page — the toggle switches the layout, the cell states the default model, the
 * catalog followed the card into the sidebar, dragging in the grid reorders and
 * writes back. All of them span the page, the shared shell, the mapping layer
 * and the compact card, and **ZCode is where two of them are likeliest to be
 * false**: its provider facts live inside the `settingsConfig` blob (so an
 * empty card still looks healthy), and its official-account card is a **list
 * member** rather than something inside a provider's card — the grid has to
 * keep it, in its slot, without cramming it into one cell.
 */
export async function verifyZcodeCardView({ send, evaluate, baseUrl, artifactRoot }) {
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
  const fixture = expression => evaluate('zcodeOfficialAccountFixture.' + expression);
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
      `window.zcodeOfficialAccountFixture?.state.runId === ${JSON.stringify(runId)}`
        + ' && zcodeOfficialAccountFixture.renderedMemberIds().length === 4',
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

  // --- the toggle exists, and switching to cards changes the layout --------
  await openFixture('viewMode=list');
  const officialCardId = await fixture('OFFICIAL_ACCOUNT_CARD_ID');
  check('the toolbar carries a layout toggle', await fixture('layoutToggleCenter()') !== null);
  check('the toggle offers the card layout from the list', await fixture('layoutToggleMode()'), 'list');
  check('the list layout renders no card grid', await evaluate(
    'Boolean(document.querySelector("[data-provider-grid]"))',
  ), false);
  // The list card states the protocol and the endpoint on its second line. The
  // cell must not: that line is where the default model goes, because "which
  // model does this provider run" is the question a grid is read to answer.
  check(
    'the list card states the endpoint',
    await fixture('cardText("provider-a")')
      .then(text => text.includes('https://provider-a.example.test/anthropic')),
  );

  await toggleLayout();
  await waitFor('Boolean(document.querySelector("[data-provider-grid]"))', 'the card grid');
  check('switching to cards renders a multi-column grid', await fixture('gridColumnCount()') > 1);
  check('the toggle now offers the list layout', await fixture('layoutToggleMode()'), 'card');
  check('the layout choice is persisted per module', await fixture('lastViewModeSave()'), {
    module: 'zcode',
    mode: 'card',
  });
  await screenshot('zcode-card-view-light');

  // --- what the cell states ------------------------------------------------
  check(
    'a cell states its default model',
    await fixture('subtitleText("provider-a")'),
    '默认模型: Model A',
  );
  check(
    'the cell does not repeat the endpoint',
    await fixture('cardText("provider-a")')
      .then(text => text.includes('provider-a.example.test')),
    false,
  );
  // One state, one wording: the cell states the applied fact through the same
  // `nameTags` fragment the list card draws, so "applied" cannot be true in one
  // layout and invisible in the other — a coloured border alone says "special"
  // without saying why.
  check(
    'the grid cell states the applied tag the list card states',
    await fixture('cardTagLabels("provider-a")'),
    ['默认'],
  );

  // --- the official-account card is a cell like any other ------------------
  //
  // It is not a provider, but it *is* a member of this list, and in a grid of
  // cards the only shape a member has is a cell. It was a full-width bordered
  // card of its own first (2026-10-13, reported twice: it kept the list layout's
  // frame, then lost the frame and sat bare among the cells), so what is pinned
  // here is that its box is a cell's box — same width as its neighbours — and
  // that its manager is not crammed into it: the cell states the live login, and
  // the account rows open in the cell's own sidebar, the way Codex's and Kimi's
  // account blocks open in theirs.
  const boxes = await fixture('gridItemBoxes()');
  assert.ok(boxes, 'the grid must render');
  const accountBox = boxes.find((box) => box.id === officialCardId);
  const cellBoxes = boxes.filter((box) => box.id && box.id !== officialCardId);
  check('the account card is rendered inside the grid', Boolean(accountBox));
  check(
    'it is one cell wide, like every other member',
    cellBoxes.length > 0
      && cellBoxes.every((box) => Math.abs((box.right - box.left) - (accountBox.right - accountBox.left)) <= 1),
  );
  check(
    'it keeps its slot in the order',
    boxes.map((box) => box.id),
    ['provider-a', officialCardId, 'provider-b', 'provider-c'],
  );
  check(
    'the cell states the live login',
    await fixture(`subtitleText(${JSON.stringify(officialCardId)})`),
    '当前登录: user@example.invalid',
  );
  // The manager is a row per account with its own actions, which is the shape
  // the sidebar is for: it is not squeezed into the cell behind the subtitle.
  check(
    'the manager is not crammed into the cell',
    await fixture(`accountRowCount(${JSON.stringify(officialCardId)})`),
    0,
  );
  // The cell frame is the shared one, so the account cell cannot drift into a
  // look of its own: same shell element as its neighbours, the same heading
  // contract (🔗 + 官方账号 + count) — plus the contracts that make it a list
  // member: the drag handle and the locate anchor.
  check(
    'the account cell is drawn in the shared cell frame',
    await evaluate(`(() => {
      const anchor = document.querySelector('[data-provider-id="${officialCardId}"]');
      const surface = anchor.firstElementChild;
      return {
        // The shared shell, by its own contract rather than by a class name:
        // the locate ring's radius is published only by the compact card, so a
        // cell that carries it is a cell drawn by the shared component.
        sharedShell: getComputedStyle(anchor).getPropertyValue('--locate-flash-radius').trim(),
        rounded: getComputedStyle(surface).borderTopLeftRadius,
        hasHandle: Boolean(anchor.querySelector('[data-provider-card-handle]')),
        hasHeading: anchor.textContent.includes('官方账号') && anchor.textContent.includes('(2)'),
        hasListRow: anchor.textContent.includes('账号列表'),
      };
    })()`),
    { sharedShell: '10px', rounded: '10px', hasHandle: true, hasHeading: true, hasListRow: false },
  );

  // --- the cell opens the account manager in its own sidebar ---------------
  //
  // The same call Codex's and Kimi's card views make with the same block: the
  // cell is too small for a manager, so the manager opens beside it. The drawer
  // itself is the shared card's (one surface for every provider sidebar in the
  // app), which is why nothing here knows its width or its panel.
  await clickAt(await fixture(`cardCenter(${JSON.stringify(officialCardId)})`));
  await waitFor('zcodeOfficialAccountFixture.detailTextIncludes("账号列表")', 'the account sidebar');
  // The panel's DOM exists the moment the drawer mounts; the drawer itself is
  // still sliding in. Without this the screenshot below is of an empty page with
  // a sliver of drawer at the right edge — the checks would all pass on a
  // sidebar nobody could see.
  await delay(300);
  check(
    'a body click opens the account manager in the cell\'s own sidebar',
    await fixture('detailAccountRowCount()'),
    2,
  );
  check(
    'the sidebar states the account block\'s explanation',
    await fixture('detailTextIncludes("切换会覆盖")'),
  );
  check(
    'the sidebar carries the login entry the cell folded away',
    await evaluate('(() => {'
      + ' const panel = zcodeOfficialAccountFixture.detailPanel();'
      + ' return [...panel.querySelectorAll("button")].some(b => b.textContent.includes("登录"));'
      + '})()'),
  );
  // Open, like the catalog beside it and like the same block on Codex and Kimi:
  // a caret in front of the rows the sidebar exists to show is a click for
  // nothing.
  check(
    'the sidebar shows the accounts open, with no caret to open them',
    await fixture('detailAccountListHasCaret()'),
    false,
  );
  await screenshot('zcode-card-view-account-detail');
  await fixture('closeDetail()');
  // The drawer animates out and unmounts late (lesson: waiting for "a panel
  // exists" is satisfied by the one that is still closing).
  await waitFor('!zcodeOfficialAccountFixture.detailPanel()', 'the account sidebar to close');

  // --- the body click opens the sidebar, and the catalog is inside it ------
  await clickAt(await fixture('cardCenter("provider-a")'));
  // Waited on the sidebar's *content*, not on "a panel exists": the account
  // drawer just closed and is still on screen while it animates out, so the
  // weaker condition would be satisfied by the panel that is going away.
  await waitFor('zcodeOfficialAccountFixture.detailTextIncludes("连接信息")', 'the provider sidebar');
  await delay(200);
  check(
    'a body click opens the sidebar',
    await evaluate('Boolean(zcodeOfficialAccountFixture.detailPanel())'),
  );
  check(
    'the sidebar states the connection fields',
    await fixture('detailSectionTitles()').then(titles => titles.includes('连接信息')),
  );
  check(
    'the sidebar states the protocol the form asked for, not its raw value',
    await fixture('detailTextIncludes("Anthropic Messages")'),
  );
  check(
    'the sidebar states the endpoint',
    await fixture('detailTextIncludes("https://provider-a.example.test/anthropic")'),
  );
  check(
    'the model catalog followed the card into the sidebar',
    await fixture('openDetailCatalog()') && await fixture('detailTextIncludes("模型列表 (2)")'),
  );
  // The catalog is a manager, not a read-only list: its toolbar came with it.
  check(
    'the sidebar catalog keeps its manager actions',
    await evaluate('(() => {'
      + ' const panel = zcodeOfficialAccountFixture.detailPanel();'
      + ' const labels = [...panel.querySelectorAll("button")].map(b => b.textContent.trim());'
      + ' return ["批量删除", "模型测试", "获取模型", "添加模型"].filter(l => labels.includes(l)).length;'
      + '})()'),
    4,
  );
  check(
    'the sidebar catalog keeps the rows\' own actions',
    await evaluate('(() => {'
      + ' const panel = zcodeOfficialAccountFixture.detailPanel();'
      + ' const rows = [...panel.querySelectorAll("[data-model-row]")];'
      + ' return rows.length === 2 && rows.every(row => row.querySelectorAll("button").length >= 3);'
      + '})()'),
  );
  check(
    'the sidebar marks the default model on its own row',
    await fixture('detailTextIncludes("当前主模型")'),
  );
  check(
    'the sidebar carries the note in its own section',
    await fixture('detailTextIncludes("Provider A note")'),
  );
  await screenshot('zcode-card-view-detail');

  // The sidebar's catalog is the *open* form of a section the list card folds:
  // the two hosts render one component, and only this one has no caret.
  check(
    'the sidebar shows the catalog open, with no caret to open it',
    await evaluate('(() => {'
      + ' const panel = zcodeOfficialAccountFixture.detailPanel();'
      + ' const section = panel.querySelector("[data-model-list-section]");'
      + ' return Boolean(section) && !section.querySelector(".ant-collapse-header");'
      + '})()'),
  );
  await fixture('closeDetail()');
  // The drawer animates out and unmounts late, so a drag that started while it
  // was still on screen would be dragging over an overlay.
  await waitFor('!zcodeOfficialAccountFixture.detailPanel()', 'the sidebar to close');

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
  await dragCard('provider-a', 'provider-c');
  const orderAfter = await fixture('renderedMemberIds()');
  check('dragging in the grid moves the card', orderAfter.join('|') !== orderBefore.join('|'));
  check(
    'the reorder payload carries provider ids only — the account card is not a provider',
    await fixture('lastReorder()').then(ids => ids && !ids.includes(officialCardId)),
  );
  check('the drag writes the order back', await evaluate(
    'zcodeOfficialAccountFixture.state.providers.map(p => p.id).join("|")',
  ) !== 'provider-a|provider-b|provider-c');
  await screenshot('zcode-card-view-dragged');

  // --- and so does the member that is not a provider -----------------------
  //
  // Dragging the account row is the capability the grid had to take over when
  // the row stopped being a card: a cell registers itself through its own shell
  // (`ProviderCompactCard`), and this member has none. Asserted as a *pair* —
  // the row moved, and the stored slot moved with it — because a row that
  // reorders on screen while the backend keeps the old index would look right
  // until the next reload.
  const dragged = async (fromId, point) => {
    const from = await fixture(`handleCenter(${JSON.stringify(fromId)})`);
    await send('Input.dispatchMouseEvent', { type: 'mousePressed', x: from.x, y: from.y, button: 'left', clickCount: 1 });
    const steps = 8;
    for (let step = 1; step <= steps; step++) {
      await send('Input.dispatchMouseEvent', {
        type: 'mouseMoved',
        x: from.x + ((point.x - from.x) * step) / steps,
        y: from.y + ((point.y - from.y) * step) / steps,
        button: 'left',
      });
      await delay(60);
    }
    await send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: point.x, y: point.y, button: 'left', clickCount: 1 });
    await delay(500);
  };

  const slotBefore = await fixture('renderedMemberIds()');
  const targetCell = await fixture('cardCenter("provider-c")');
  await dragged(officialCardId, targetCell);
  check(
    'the account row participates in grid dragging like any other member',
    await fixture('renderedMemberIds()').then(ids => ids.join('|') !== slotBefore.join('|')),
  );
  check(
    'the row keeps a handle of its own to be dragged by',
    await evaluate(`Boolean(document.querySelector('[data-provider-id="${officialCardId}"] [data-provider-card-handle]'))`),
  );
  // Pressing the body opens the manager; pressing the handle and moving the card
  // must not. The handle is a `<span>`, so "is this a control?" cannot be
  // answered by the tag name alone.
  check(
    'dragging the account row does not open its sidebar',
    await evaluate('!zcodeOfficialAccountFixture.detailPanel()'),
  );

  // --- switching back restores the list layout ----------------------------
  await toggleLayout();
  check('switching back removes the grid', await evaluate(
    'Boolean(document.querySelector("[data-provider-grid]"))',
  ), false);
  check('the list layout renders every member again', await fixture('renderedMemberIds()').then(
    ids => ids.length,
  ), 4);

  // --- dark theme keeps the grid readable ---------------------------------
  await openFixture('viewMode=card&theme=dark');
  check(
    'the grid renders in dark theme',
    await evaluate('document.documentElement.dataset.theme'),
    'dark',
  );
  check('the dark grid is still a grid', await fixture('gridColumnCount()') > 1);
  await screenshot('zcode-card-view-dark');

  // --- the login line answers in every state ------------------------------
  //
  // The cell's second line is reserved whether or not it states something, so a
  // channel with accounts but none in use left the band blank — which reads as a
  // broken card rather than as "nothing to say" (user-reported 2026-10-14, on
  // two idle channels). It now answers the question it exists for in every
  // state: the login in use, the live login the CLI's own credential file holds,
  // or 未登录 when there is neither.
  await openFixture('viewMode=card&accounts=idle');
  check(
    'a channel with no login in use says so instead of leaving the line empty',
    await fixture(`subtitleText(${JSON.stringify(officialCardId)})`),
    '当前登录: 未登录',
  );
  check(
    'the idle channel still states how many accounts it holds',
    await fixture(`cardText(${JSON.stringify(officialCardId)})`).then(text => text.includes('(2)')),
  );
  await screenshot('zcode-card-view-idle-channel');

  // --- the cell states the control this CLI actually has -------------------
  //
  // ZCode applies no provider — `设为默认` lives on a model row — so the cell's
  // action row, the one Codex's and Kimi's cells give to 应用 / 网关代理, used to
  // state nothing at all, while the per-provider enable switch sat folded into
  // the "more" menu. The user asked for it out and in that slot: 「针对 zcode
  // 这种不需要应用的渠道，把它在列表模式下的开关按钮放出来，放到现在应用的地方」
  // (2026-10-15). One control, one place: the cell states it, the menu drops it.
  await openFixture('viewMode=card');
  check(
    'a cell states the provider\'s enable switch on its action row',
    await fixture('cellSwitchState("provider-b")'),
    { checked: true, endsTheRow: true, rowButtonCount: 1 },
  );
  // A pointer that is already somewhere else, then onto the control: the
  // tooltip opens on `mouseenter`, and a single move to one spot need not
  // produce one (the same reset the Codex suite makes before its own hover).
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 2, y: 2 });
  await delay(200);
  const switchCenter = await fixture('cellSwitchCenter("provider-b")');
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: switchCenter.x, y: switchCenter.y });
  await delay(500);
  check(
    'the switch is the one the list card draws — same control, same wording',
    await fixture('openTooltipText()'),
    '已启用',
  );

  // A real click, because everything here is about what the user can do: the
  // provider is turned off, the card says so, and the way back stays on screen.
  await clickAt(switchCenter);
  await delay(600);
  check(
    'clicking the switch disables this provider',
    await fixture('lastProviderToggle()'),
    { providerId: 'provider-b', isDisabled: true },
  );
  check(
    'the card states the provider it just disabled',
    await fixture('cardTagLabels("provider-b")'),
    ['已禁用'],
  );
  check(
    'the way back is on the row that turned it off',
    await fixture('cellSwitchState("provider-b")').then(state => state && state.checked),
    false,
  );
  // The switch is a control on the card body, and the body click opens the
  // sidebar — the same trap the drag handle sets (2026-10-14).
  check(
    'clicking the switch does not open the cell\'s sidebar',
    await evaluate('!zcodeOfficialAccountFixture.detailPanel()'),
  );

  // And the menu gives up its copy: the switch is on the card now, so the entry
  // would be the same control twice on one card.
  await send('Input.dispatchMouseEvent', {
    type: 'mouseMoved',
    x: (await fixture('cardCenter("provider-b")')).x,
    y: (await fixture('cardCenter("provider-b")')).y,
  });
  await delay(300);
  await clickAt(await fixture('moreButtonCenter("provider-b")'));
  await delay(300);
  check(
    'the menu no longer carries the switch the cell states',
    await fixture('openMenuItemLabels()'),
    ['编辑', '复制', '删除'],
  );
  await evaluate('document.body.click()');
  await delay(200);
  await screenshot('zcode-card-view-provider-switch');

  return checks;
}