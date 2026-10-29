import assert from 'node:assert/strict';

/** Drives the real CodexPage through the browser OAuth IPC lifecycle. */
export async function verifyCodexBrowserOauth({ send, evaluate, baseUrl, screenshot }) {
  const checks = [];
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
  const waitFor = async (expression, label) => {
    for (let attempt = 0; attempt < 200; attempt++) {
      if (await evaluate(`Boolean(${expression})`)) return;
      await delay(100);
    }
    throw new Error('Timed out waiting for: ' + label);
  };
  const check = (name, actual, expected = true) => {
    assert.deepEqual(actual, expected, name);
    checks.push(name);
  };
  const clickNamed = async (label) => {
    const found = await evaluate(`(() => {
      const button = [...document.querySelectorAll('button')]
        .find(item => item.textContent.trim() === ${JSON.stringify(label)} && item.offsetWidth > 0);
      if (!button) return false;
      button.click();
      return true;
    })()`);
    assert.equal(found, true, 'button exists: ' + label);
  };
  const openLogin = async () => {
    await clickNamed('OAuth 登录');
    await waitFor("document.body.textContent.includes('选择登录方式')", 'login method chooser');
    await clickNamed('浏览器 OAuth');
    await waitFor("document.body.textContent.includes('Codex 浏览器登录')", 'browser OAuth modal');
  };
  const browserModal = "document.querySelector('.ant-modal-wrap:not([style*=\"display: none\"]) .ant-modal-title')?.textContent === 'Codex 浏览器登录'";
  const cancelModal = async () => {
    const clicked = await evaluate(`(() => {
      const button = document.querySelector('.ant-modal-wrap:not([style*="display: none"]) .ant-modal-footer button:last-child');
      if (!button) return false;
      button.click();
      return true;
    })()`);
    assert.equal(clicked, true, 'browser OAuth cancel button');
  };
  const starts = "codexCardViewFixture.state.requests.filter(r => r.command === 'start_codex_official_account_oauth').length";
  const cancels = "codexCardViewFixture.state.requests.filter(r => r.command === 'cancel_codex_official_account_oauth').length";

  await send('Page.navigate', { url: `${baseUrl}/?browserOauth=1&runId=oauth-check` });
  await waitFor("window.codexCardViewFixture?.state.runId === 'oauth-check' && document.querySelector('[data-provider-id=provider-c]')", 'Codex page');
  await openLogin();
  await waitFor('codexCardViewFixture.pendingOauthStartCount() === 1', 'first start');
  check('listener precedes start result', await evaluate('codexCardViewFixture.oauthListenerInstalled()'));
  check('waiting modal opens before start result', await evaluate("document.body.textContent.includes('正在准备登录')"));
  const listsBefore = await evaluate("codexCardViewFixture.state.requests.filter(r => r.command === 'list_codex_official_accounts').length");
  await evaluate("codexCardViewFixture.emitOauthStatus('oauth-session-1', 'authorized'), codexCardViewFixture.resolveOauthStart(), true");
  await waitFor(`(${starts}) === 1 && !(${browserModal})`, 'fast completion');
  await waitFor(`codexCardViewFixture.state.requests.filter(r => r.command === 'list_codex_official_accounts').length > ${listsBefore}`, 'account reload');
  check('early authorized event reloads accounts once', await evaluate("codexCardViewFixture.state.requests.filter(r => r.command === 'list_codex_official_accounts').length > " + listsBefore));
  const listsAfterLogin = await evaluate("codexCardViewFixture.state.requests.filter(r => r.command === 'list_codex_official_accounts').length");
  await evaluate('codexCardViewFixture.emitAccountsChanged(), true');
  await waitFor(`codexCardViewFixture.state.requests.filter(r => r.command === 'list_codex_official_accounts').length > ${listsAfterLogin}`, 'background quota reconciliation');
  check('background usage refresh reloads accounts after the login dialog closes', await evaluate(`!(${browserModal})`));

  await openLogin();
  await waitFor('codexCardViewFixture.pendingOauthStartCount() === 1', 'second start');
  await evaluate("codexCardViewFixture.resolveOauthStart('oauth-session-2'), true");
  await waitFor("codexCardViewFixture.openedUrls().length === 1", 'browser open');
  check('start opens returned authorization URL', await evaluate("codexCardViewFixture.openedUrls()[0]"), 'https://example.invalid/oauth?session=oauth-session-2');
  await clickNamed('重新打开浏览器');
  await waitFor('codexCardViewFixture.openedUrls().length === 2', 'reopen browser');
  await clickNamed('复制登录链接');
  await waitFor('codexCardViewFixture.clipboardWrites().length === 1', 'copy URL');
  check('copy uses the exact authorization URL', await evaluate('codexCardViewFixture.clipboardWrites()[0]'), 'https://example.invalid/oauth?session=oauth-session-2');
  await evaluate("codexCardViewFixture.emitOauthStatus('oauth-session-2', 'exchanging'), true");
  await waitFor("document.body.textContent.includes('正在保存账号')", 'exchange status');
  await cancelModal();
  await waitFor(`(${cancels}) === 1`, 'cancel command');
  check('cancel targets active session', await evaluate("codexCardViewFixture.state.requests.filter(r => r.command === 'cancel_codex_official_account_oauth')[0].args.sessionId"), 'oauth-session-2');

  await openLogin();
  await waitFor('codexCardViewFixture.pendingOauthStartCount() === 1', 'third start');
  await cancelModal();
  await evaluate("codexCardViewFixture.resolveOauthStart('oauth-session-3'), true");
  await waitFor(`(${cancels}) === 2`, 'late start cancellation');
  check('closing before start resolves still cancels its session', await evaluate("codexCardViewFixture.state.requests.filter(r => r.command === 'cancel_codex_official_account_oauth')[1].args.sessionId"), 'oauth-session-3');

  await openLogin();
  await waitFor('codexCardViewFixture.pendingOauthStartCount() === 1', 'fourth start');
  await evaluate("codexCardViewFixture.resolveOauthStart('oauth-session-4'), true");
  await waitFor('codexCardViewFixture.openedUrls().length === 3', 'fourth browser open');
  await evaluate("codexCardViewFixture.emitOauthStatus('oauth-session-4', 'error', 'Synthetic OAuth failure'), true");
  await waitFor("document.body.textContent.includes('Synthetic OAuth failure')", 'error shown');
  await clickNamed('重试登录');
  await waitFor('codexCardViewFixture.pendingOauthStartCount() === 1', 'retry start');
  check('retry invokes a fresh start', await evaluate(starts), 5);
  await evaluate("codexCardViewFixture.resolveOauthStart('oauth-session-5'), true");
  await waitFor('codexCardViewFixture.openedUrls().length === 4', 'retry browser open');
  await evaluate("codexCardViewFixture.emitOauthStatus('oauth-session-5', 'expired'), true");
  await waitFor("document.body.textContent.includes('登录已超时')", 'expiry shown');
  await cancelModal();
  for (const mode of ['dark', 'system']) {
    await send('Page.navigate', { url: `${baseUrl}/?browserOauth=1&theme=${mode}&runId=oauth-${mode}` });
    await waitFor(`window.codexCardViewFixture?.state.runId === 'oauth-${mode}' && document.querySelector('[data-provider-id=provider-c]')`, 'themed Codex page');
    await openLogin();
    await waitFor('codexCardViewFixture.pendingOauthStartCount() === 1', 'themed start');
    await evaluate(`codexCardViewFixture.resolveOauthStart('oauth-${mode}'), true`);
    await waitFor("document.body.textContent.includes('剩余')", 'themed waiting state');
    check(mode + ' theme has readable text and background', await evaluate(`(() => {
      const modal = [...document.querySelectorAll('.ant-modal')].find(el => el.offsetParent !== null && el.querySelector('.ant-modal-title')?.textContent === 'Codex 浏览器登录');
      const text = modal.querySelector('.ant-typography');
      const surface = modal.querySelector('.ant-modal-container, .ant-modal-content') || modal;
      return getComputedStyle(text).color !== getComputedStyle(surface).backgroundColor;
    })()`));
    await evaluate(`codexCardViewFixture.emitOauthStatus('oauth-${mode}', 'error', 'Long connection failure: ' + 'proxy/path/'.repeat(90)), true`);
    await waitFor("document.body.textContent.includes('Long connection failure:')", 'long error');
    check(mode + ' theme contains long errors inside the viewport', await evaluate(`(() => {
      const modal = [...document.querySelectorAll('.ant-modal')].find(el => el.offsetParent !== null && el.querySelector('.ant-modal-title')?.textContent === 'Codex 浏览器登录');
      return modal.getBoundingClientRect().right <= innerWidth && modal.scrollWidth <= modal.clientWidth + 1;
    })()`));
    if (screenshot) await screenshot('codex-browser-oauth-' + mode);
    await cancelModal();
  }

  await send('Page.navigate', { url: `${baseUrl}/?browserOauth=1&runId=device-auth-check` });
  await waitFor("window.codexCardViewFixture?.state.runId === 'device-auth-check' && document.querySelector('[data-provider-id=provider-c]')", 'device auth page');
  const openDevice = async () => {
    await clickNamed('OAuth 登录');
    await waitFor("document.body.textContent.includes('选择登录方式')", 'device login chooser');
    await clickNamed('设备码登录');
    await waitFor("document.body.textContent.includes('Codex 设备码登录')", 'device auth modal');
  };
  const deviceCancels = "codexCardViewFixture.state.requests.filter(r => r.command === 'cancel_codex_official_account_device_auth')";
  await openDevice();
  await waitFor('codexCardViewFixture.pendingDeviceStartCount() === 1', 'device start pending');
  check('device listener precedes start result', await evaluate('codexCardViewFixture.oauthListenerInstalled()'));
  check('device modal opens during code request', await evaluate("document.body.textContent.includes('正在获取设备码')"));
  const deviceListsBefore = await evaluate("codexCardViewFixture.state.requests.filter(r => r.command === 'list_codex_official_accounts').length");
  await evaluate("codexCardViewFixture.emitOauthStatus('device-session-1', 'authorized'), codexCardViewFixture.resolveDeviceStart(), true");
  await waitFor(`codexCardViewFixture.state.requests.filter(r => r.command === 'list_codex_official_accounts').length > ${deviceListsBefore}`, 'early device completion reload');
  check('early device authorization closes modal', await evaluate("!document.querySelector('.ant-modal-wrap:not([style*=\"display: none\"]) .ant-modal-title')?.textContent.includes('Codex 设备码登录')"));

  await openDevice();
  await waitFor('codexCardViewFixture.pendingDeviceStartCount() === 1', 'device start before cancel');
  await cancelModal();
  await evaluate("codexCardViewFixture.resolveDeviceStart('device-session-2'), true");
  await waitFor(`${deviceCancels}.length === 1`, 'device cancel intent');
  check('closing before device start resolves cancels its session', await evaluate(`${deviceCancels}[0].args.sessionId`), 'device-session-2');

  await openDevice();
  await waitFor('codexCardViewFixture.pendingDeviceStartCount() === 1', 'device exchange scenario');
  await evaluate("codexCardViewFixture.resolveDeviceStart('device-session-3'), true");
  await waitFor("document.body.textContent.includes('CODE-123')", 'device code shown');
  await evaluate("codexCardViewFixture.emitOauthStatus('device-session-3', 'exchanging'), true");
  await waitFor("document.body.textContent.includes('正在保存账号')", 'device exchange status');
  await cancelModal();
  const listsBeforeDeviceUpdate = await evaluate("codexCardViewFixture.state.requests.filter(r => r.command === 'list_codex_official_accounts').length");
  await evaluate('codexCardViewFixture.emitAccountsChanged(), true');
  await waitFor(`codexCardViewFixture.state.requests.filter(r => r.command === 'list_codex_official_accounts').length > ${listsBeforeDeviceUpdate}`, 'device background save reload');
  check('device account update reloads after dialog closes', true);
  return checks;
}
