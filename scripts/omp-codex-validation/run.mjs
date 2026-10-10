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
  for (const [mode, width] of [['light',1280],['dark',1280],['system',480]]) {
    await send('Emulation.setDeviceMetricsOverride', { width, height: 900, deviceScaleFactor: 1, mobile: false });
    await send('Emulation.setEmulatedMedia', { features: [{name:'prefers-color-scheme',value:'dark'}] });
    await send('Page.navigate',{url:baseUrl+'/?theme='+mode});
    await waitFor(`window.ompFixture?.state.mode === ${JSON.stringify(mode)} && document.querySelector("#defaultModel")`);
    await delay(300);
    check(mode+' page does not overflow', await evaluate('document.documentElement.scrollWidth <= window.innerWidth'));
    check(mode+' state and override are visible', await evaluate('document.body.innerText.includes("本地状态未知") && document.body.innerText.includes("API Key 覆盖")'));
    await capture(mode+'-section');
    await click('订阅登录指引');
    await waitFor('document.querySelector(".ant-modal") && document.querySelector(".ant-modal").getBoundingClientRect().height > 0');
    await delay(200);
    check(mode+' modal fits viewport', await evaluate('(() => { const r=document.querySelector(".ant-modal").getBoundingClientRect(); return r.left >= 0 && r.right <= innerWidth; })()'));
    check(mode+' modal footer remains visible', await evaluate('document.querySelector(".ant-modal-footer").getBoundingClientRect().bottom <= innerHeight'));
    check(mode+' commands wrap inside modal', await evaluate('Array.from(document.querySelectorAll("code")).every(code => code.scrollWidth <= code.clientWidth + 1)'));
    await capture(mode+'-guide');
    await click('关闭');
    await waitFor('!document.querySelector(".ant-modal") || document.querySelector(".ant-modal").getClientRects().length === 0');
    check(mode+' cancel performs no IO', await evaluate('window.ompFixture.state.refreshes === 0 && window.ompFixture.state.invocations.length === 0'));
    await perform('window.ompFixture.setInput("Custom-Exact-Codex:42")');
    await delay(100);
    check(mode+' typing does not persist partial model',await evaluate('window.ompFixture.state.commits.length'),0);
    await perform('document.querySelector("#defaultModel").blur()'); await delay(100);
    check(mode+' blur saves exact manual model',await evaluate('window.ompFixture.state.commits.map(item=>item.values.defaultModel)'),['Custom-Exact-Codex:42']);
    await perform("const button = Array.from(document.querySelectorAll('button')).find(button => button.textContent.replace(/\\s/g, '') === '刷新'); button.click(); button.click()"); await delay(300);
    check(mode+' repeated refresh updates local state once',await evaluate('document.body.innerText.includes("检测到本地 OAuth 记录") && window.ompFixture.state.refreshes === 1'));
  }
  await perform('window.ompFixture.setOptions([{value:"native-model-exact",label:"native-model-exact"}]); window.ompFixture.setModel("")'); await delay(100);
  await perform('window.ompFixture.setInput("native")'); await delay(200);
  await send('Input.dispatchKeyEvent',{ type:'keyDown', key:'ArrowDown',code:'ArrowDown',windowsVirtualKeyCode:40 });
  await send('Input.dispatchKeyEvent',{ type:'keyDown', key:'Enter',code:'Enter',windowsVirtualKeyCode:13 });
  await delay(200);
  check('keyboard option selection commits exact full model once',await evaluate('window.ompFixture.state.commits.map(item=>item.values.defaultModel)'),['Custom-Exact-Codex:42','native-model-exact']);
  return checks;
} });
