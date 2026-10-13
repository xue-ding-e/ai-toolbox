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

  // --- the official-account card is still a member of the list -------------
  //
  // It is not a provider, so there is no cell shape for it — but it is a list
  // member, and the grid has to keep it in the slot the user dragged it to.
  // Measured rather than asserted in the abstract: a card that merely *looks*
  // full-width because it is alone in its row would pass a CSS-only claim.
  const boxes = await fixture('gridItemBoxes()');
  const gridBox = await evaluate(`(() => {
    const grid = document.querySelector('[data-provider-grid]');
    const rect = grid.getBoundingClientRect();
    return { left: Math.round(rect.left), right: Math.round(rect.right) };
  })()`);
  assert.ok(boxes, 'the grid must render');
  const accountBox = boxes.find((box) => box.id === officialCardId);
  const cellBoxes = boxes.filter((box) => box.id && box.id !== officialCardId);
  check('the account card is rendered inside the grid', Boolean(accountBox));
  check(
    'it spans the whole row rather than sitting in one column',
    accountBox.left === gridBox.left && accountBox.right === gridBox.right,
  );
  check(
    'the provider cells are narrower than that row',
    cellBoxes.length > 0 && cellBoxes.every((box) => box.right - box.left < accountBox.right - accountBox.left),
  );
  check(
    'it keeps its slot in the order',
    boxes.map((box) => box.id),
    ['provider-a', officialCardId, 'provider-b', 'provider-c'],
  );
  check(
    'the account block came with it',
    await fixture(`accountRowCount(${JSON.stringify(officialCardId)})`),
    2,
  );

  // --- the body click opens the sidebar, and the catalog is inside it ------
  await clickAt(await fixture('cardCenter("provider-a")'));
  await waitFor('Boolean(zcodeOfficialAccountFixture.detailPanel())', 'the detail sidebar');
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

  return checks;
}