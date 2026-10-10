import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const { runBrowserFixture } = await import(pathToFileURL(path.join(process.cwd(), 'scripts/lib/browser-fixture.mjs')).href);
const fixtureDirectory = fileURLToPath(new URL('.', import.meta.url));
await runBrowserFixture({ fixtureDirectory, fixtureFilename: 'Fixture.jsx', artifactPrefix: 'omp-subscription-ui-', verify: async ({ send, evaluate, baseUrl, artifactRoot }) => {
  const checks = [];
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
  const check = (name, actual, expected = true) => { assert.deepEqual(actual, expected, name); checks.push(name); console.log('PASS ' + name); };
  // CDP returnByValue must never serialize a DOM node and its React references.
  const waitFor = async expression => { for(let i=0; i<100;i++){if(await evaluate(`Boolean(${expression})`)) return; await delay(50);} throw Error('Timed out: ' + expression); };
  const perform = expression => evaluate(`(() => { ${expression}; return true; })()`);
  const click = text => perform(`Array.from(document.querySelectorAll('button')).find(button => button.textContent.replace(/\\s/g, '') === ${JSON.stringify(text)}).click()`);
  const capture = async name => { const {data}=await send('Page.captureScreenshot', { format: 'png' }); await writeFile(path.join(artifactRoot, name + '.png'), Buffer.from(data, 'base64')); };
  const pressKey = async (key, code, windowsVirtualKeyCode, modifiers = 0) => {
    await send('Input.dispatchKeyEvent', { type: 'keyDown', key, code, windowsVirtualKeyCode, modifiers });
    await send('Input.dispatchKeyEvent', { type: 'keyUp', key, code, windowsVirtualKeyCode, modifiers });
  };
  const typeModel = async text => {
    const point = await evaluate('(() => { const r = document.querySelector("#defaultModel").getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()');
    await send('Input.dispatchMouseEvent', { type: 'mousePressed', button: 'left', clickCount: 1, ...point });
    await send('Input.dispatchMouseEvent', { type: 'mouseReleased', button: 'left', clickCount: 1, ...point });
    await waitFor('document.activeElement?.id === "defaultModel"');
    await pressKey('a', 'KeyA', 65, 2);
    await send('Input.insertText', { text });
    await waitFor(`document.querySelector("#defaultModel").value === ${JSON.stringify(text)}`);
  };
  for (const [caseName, mode, width, height] of [['light', 'light', 1280, 900], ['dark', 'dark', 1280, 900], ['system', 'system', 480, 900], ['compact', 'dark', 320, 600]]) {
    await send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: false });
    await send('Emulation.setEmulatedMedia', { features: [{name:'prefers-color-scheme',value:'dark'}] });
    await send('Page.navigate',{url:baseUrl+'/?theme='+mode});
    await waitFor(`window.ompFixture?.state.mode === ${JSON.stringify(mode)} && document.querySelector("#defaultModel")`);
    await delay(300);
    check(caseName+' page does not overflow', await evaluate('document.documentElement.scrollWidth <= window.innerWidth'));
    if (width > 600) check(caseName+' model input aligns with provider', await evaluate('Math.abs(document.querySelector("#defaultModel").closest(".ant-select").getBoundingClientRect().top - document.querySelector("#defaultProvider").closest(".ant-select").getBoundingClientRect().top) < 2'));
    check(caseName+' state and override are visible', await evaluate('document.body.innerText.includes("本地状态未知") && document.body.innerText.includes("API Key 覆盖")'));
    await capture(caseName+'-section');
    await click('订阅登录指引');
    await waitFor('document.querySelector(".ant-modal") && document.querySelector(".ant-modal").getBoundingClientRect().height > 0');
    await delay(200);
    const modalGeometry = await evaluate(`(() => {
      const measure = selector => {
        const element = document.querySelector(selector);
        if (!element) return null;
        const rect = element.getBoundingClientRect();
        const style = getComputedStyle(element);
        return {
          rect: { top: rect.top, right: rect.right, bottom: rect.bottom, left: rect.left, width: rect.width, height: rect.height },
          clientWidth: element.clientWidth, clientHeight: element.clientHeight,
          scrollWidth: element.scrollWidth, scrollHeight: element.scrollHeight, scrollTop: element.scrollTop,
          display: style.display, position: style.position, verticalAlign: style.verticalAlign,
          boxSizing: style.boxSizing, width: style.width, height: style.height,
          maxWidth: style.maxWidth, maxHeight: style.maxHeight,
          padding: style.padding, margin: style.margin, top: style.top,
          overflowX: style.overflowX, overflowY: style.overflowY, flex: style.flex,
        };
      };
      const wrap = document.querySelector('.ant-modal-wrap');
      const before = getComputedStyle(wrap, '::before');
      return {
        viewport: { width: innerWidth, height: innerHeight },
        wrap: measure('.ant-modal-wrap'), modal: measure('.ant-modal'),
        container: measure('.ant-modal-container'), body: measure('.ant-modal-body'), footer: measure('.ant-modal-footer'),
        before: { width: before.width, height: before.height, display: before.display, verticalAlign: before.verticalAlign },
      };
    })()`);
    console.log(caseName + ' modal geometry: ' + JSON.stringify(modalGeometry));
    await writeFile(path.join(artifactRoot, caseName + '-modal-geometry.json'), JSON.stringify(modalGeometry, null, 2));
    check(caseName+' modal fits viewport', await evaluate('(() => { const r=document.querySelector(".ant-modal").getBoundingClientRect(); return r.left >= 0 && r.right <= innerWidth; })()'));
    check(caseName+' modal footer remains visible', await evaluate('document.querySelector(".ant-modal-footer").getBoundingClientRect().bottom <= innerHeight'));
    if (width < 600) check(caseName+' tall modal body scrolls within the global height limit', modalGeometry.body.overflowY === 'auto' && modalGeometry.body.scrollHeight > modalGeometry.body.clientHeight);
    check(caseName+' commands wrap inside modal', await evaluate('Array.from(document.querySelectorAll("code")).every(code => code.scrollWidth <= code.clientWidth + 1)'));
    if (height < 700) {
      let refreshReachable = false;
      for (let attempt = 0; attempt < 8; attempt++) {
        await pressKey('Tab', 'Tab', 9);
        refreshReachable = await evaluate('document.activeElement === Array.from(document.querySelectorAll(".ant-modal-footer button")).at(-1)');
        if (refreshReachable) break;
      }
      check(caseName+' footer refresh is reachable by keyboard', refreshReachable);
      check(caseName+' keyboard-focused footer action is fully visible', await evaluate('(() => { const r = document.activeElement.getBoundingClientRect(); return r.height > 0 && r.top >= 0 && r.bottom <= innerHeight; })()'));
    }
    await capture(caseName+'-guide');
    await click('关闭');
    await waitFor('!document.querySelector(".ant-modal") || document.querySelector(".ant-modal").getClientRects().length === 0');
    check(caseName+' cancel performs no IO', await evaluate('window.ompFixture.state.refreshes === 0 && window.ompFixture.state.invocations.length === 0'));
    await typeModel('Custom-Exact-Codex:42');
    await delay(100);
    check(caseName+' typing does not persist partial model',await evaluate('window.ompFixture.state.commits.length'),0);
    await pressKey('Tab', 'Tab', 9);
    await waitFor('document.activeElement?.id !== "defaultModel"');
    await delay(100);
    console.log(caseName + ' model input diagnostics: ' + JSON.stringify(await evaluate('({ events: window.ompFixture.state.inputEvents, componentCommits: window.ompFixture.state.componentCommits, formCommits: window.ompFixture.state.commits, activeElement: document.activeElement?.id })')));
    check(caseName+' blur saves exact manual model',await evaluate('window.ompFixture.state.commits.map(item=>item.values.defaultModel)'),['Custom-Exact-Codex:42']);
    await perform("const button = Array.from(document.querySelectorAll('button')).find(button => button.textContent.replace(/\\s/g, '') === '刷新'); button.click(); button.click()"); await delay(300);
    check(caseName+' repeated refresh updates local state once',await evaluate('document.body.innerText.includes("检测到本地 OAuth 记录") && window.ompFixture.state.refreshes === 1'));
  }
  await perform('window.ompFixture.setOptions([{value:"native-model-exact",label:"native-model-exact"}]); window.ompFixture.setModel("")'); await delay(100);
  await typeModel('native'); await delay(200);
  await pressKey('ArrowDown', 'ArrowDown', 40);
  await pressKey('Enter', 'Enter', 13);
  await delay(200);
  check('keyboard option selection commits exact full model once',await evaluate('window.ompFixture.state.commits.map(item=>item.values.defaultModel)'),['Custom-Exact-Codex:42','native-model-exact']);
  return checks;
} });
