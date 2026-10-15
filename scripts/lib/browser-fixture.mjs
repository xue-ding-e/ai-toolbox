import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { access, mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { build } from 'vite';

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');

// Terminate a headless browser and everything it spawned.
//
// Teardown used to be `await send('Browser.close')` followed by `browser.kill()`.
// Neither step is guaranteed while the browser is unresponsive — which is the
// state that produces the failure — and `browser.kill()` only reaps the process
// Node spawned, not the renderer/GPU/utility children Chromium gave it. So stop
// trusting the handshake: allow a grace period, then take the whole tree down.
//
// What this cannot cover: a script killed from outside (command timeout, Ctrl+C,
// a truncated pipe) never reaches teardown at all, and the whole browser survives.
// The recognition and cleanup recipe for those orphans is in the root AGENTS.md.
async function killBrowserTree(browser) {
  if (!browser || browser.exitCode !== null || browser.signalCode !== null) return;
  const exited = new Promise(resolve => browser.once('exit', () => resolve(true)));
  let graceTimer;
  const gracePeriod = new Promise(resolve => { graceTimer = setTimeout(() => resolve(false), 2000); });
  browser.kill();
  const didExit = await Promise.race([exited, gracePeriod]);
  // A pending timer holds the event loop open, so a healthy exit must not leave
  // the fixture script sitting here for the rest of the grace period.
  clearTimeout(graceTimer);
  if (didExit) return;
  if (process.platform === 'win32') {
    // Windows does not cascade a terminated parent onto its children, so ask for
    // the tree explicitly; `/T` is what reaches the stuck renderer/GPU pair.
    await new Promise(resolve => {
      const killer = spawn('taskkill', ['/PID', String(browser.pid), '/T', '/F'], { stdio: 'ignore' });
      killer.once('exit', resolve);
      killer.once('error', resolve);
    });
    return;
  }
  try { browser.kill('SIGKILL'); } catch { /* Already gone. */ }
}

// Shared Chromium/Vite fixture runner. All HTTP, profiles and artifacts are local.
export async function runBrowserFixture({ fixtureDirectory, fixtureFilename, artifactPrefix, verify, fixtureAliases = [] }) {
  const { values } = parseArgs({ options: { browser: { type: 'string' }, output: { type: 'string' } } });
  const outputParent = path.resolve(values.output ?? tmpdir());
  await mkdir(outputParent, { recursive: true });
  const artifactRoot = await mkdtemp(path.join(outputParent, artifactPrefix));
  const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));

  async function findBrowser() {
    const candidates = [
      values.browser,
      process.env['PROGRAMFILES(X86)'] && path.join(process.env['PROGRAMFILES(X86)'], 'Microsoft/Edge/Application/msedge.exe'),
      process.env.PROGRAMFILES && path.join(process.env.PROGRAMFILES, 'Google/Chrome/Application/chrome.exe'),
      '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
      '/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge',
      '/usr/bin/chromium', '/usr/bin/chromium-browser', '/usr/bin/google-chrome',
    ].filter(Boolean);
    for (const candidate of candidates) {
      try { await access(candidate); return candidate; } catch { /* Try the next installed browser. */ }
    }
    throw new Error('No Chromium browser found. Pass --browser with an installed Chrome or Edge executable.');
  }

  const browserPath = await findBrowser();
  const packageJson = JSON.parse(await readFile(path.join(projectRoot, 'package.json'), 'utf8'));
  const aliases = [
    ...fixtureAliases,
    ...['', '/react', '/vanilla'].map(suffix => ({
      find: new RegExp('^zustand' + suffix + '$'),
      replacement: path.join(projectRoot, 'node_modules/zustand/esm', (suffix.slice(1) || 'index') + '.mjs'),
    })),
    { find: '@', replacement: path.join(projectRoot, 'web') },
    ...Object.keys(packageJson.dependencies).map(name => ({
      find: new RegExp('^' + name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') + '(?=/|$)'),
      replacement: path.join(projectRoot, 'node_modules', name),
    })),
  ];
  await writeFile(path.join(artifactRoot, 'entry.jsx'), 'import ' + JSON.stringify(path.join(fixtureDirectory, fixtureFilename).replaceAll('\\', '/')) + ';');
  await writeFile(path.join(artifactRoot, 'index.html'), '<!DOCTYPE html><html><head><meta charset="UTF-8"></head><body><div id="root"></div><script type="module" src="./entry.jsx"></script></body></html>');
  await build({
    configFile: false, root: artifactRoot, base: './', resolve: { alias: aliases }, logLevel: 'error',
    css: { preprocessorOptions: { less: { javascriptEnabled: true } } }, esbuild: { jsx: 'automatic' },
    build: { outDir: path.join(artifactRoot, 'dist'), emptyOutDir: false, reportCompressedSize: false, target: 'esnext',
      rollupOptions: { onwarn(warning, warn) { if (warning.code !== 'MODULE_LEVEL_DIRECTIVE') warn(warning); } },
    },
  });

  async function openBrowser() {
    const portReservation = createServer();
    await new Promise(resolve => portReservation.listen(0, '127.0.0.1', resolve));
    const debugPort = portReservation.address().port;
    await new Promise(resolve => portReservation.close(resolve));
    const profilePath = await mkdtemp(path.join(artifactRoot, 'browser-profile-'));
    const browser = spawn(browserPath, [
      '--headless=new', '--remote-debugging-port=' + debugPort, '--user-data-dir=' + profilePath,
      '--no-first-run', '--no-default-browser-check', '--disable-extensions', '--disable-background-networking',
      '--window-size=1280,900', '--force-device-scale-factor=1', 'about:blank',
    ], { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
    let browserErrors = '';
    browser.stderr.on('data', data => { browserErrors += data; });
    let socket;
    try {
      // Wait for the page target, not merely for the DevTools HTTP endpoint.
      // The endpoint can answer before the first page exists, and reading that
      // early list crashes on `.webSocketDebuggerUrl` of an absent page. A
      // loaded CI runner has also needed ~12s from launch to answer at all, so
      // the budget is as generous as the CDP command timeout below.
      let pageTarget;
      for (let attempt = 0; attempt < 600 && !pageTarget; attempt++) {
        try {
          const targets = await (await fetch('http://127.0.0.1:' + debugPort + '/json/list')).json();
          pageTarget = targets.find(target => target.type === 'page' && target.webSocketDebuggerUrl);
        } catch { /* Endpoint not up yet. */ }
        if (!pageTarget) await delay(100);
      }
      if (!pageTarget) throw new Error('Browser did not start: ' + browserErrors.slice(-1500));
      socket = new WebSocket(pageTarget.webSocketDebuggerUrl);
      await new Promise((resolve, reject) => {
        socket.addEventListener('open', resolve, { once: true });
        socket.addEventListener('error', reject, { once: true });
      });
      const pendingRequests = new Map();
      const exceptions = [];
      let nextRequestId = 0;
      socket.addEventListener('message', event => {
        const payload = JSON.parse(event.data);
        if (payload.method === 'Runtime.exceptionThrown') exceptions.push(payload.params.exceptionDetails);
        const request = pendingRequests.get(payload.id);
        if (!request) return;
        pendingRequests.delete(payload.id); clearTimeout(request.timeout);
        payload.error ? request.reject(new Error(JSON.stringify(payload.error))) : request.resolve(payload.result);
      });
      const send = (method, params = {}) => new Promise((resolve, reject) => {
        const id = ++nextRequestId;
        // 60s, not 30s: on a busy dev machine (the app itself, other browser
        // fixtures in the same `pnpm test` run) a page boot can hold the
        // renderer long enough to exceed a 30s budget, and a command timeout
        // reads as a product failure. Nothing legitimate takes 60s; a real hang
        // still fails, just later.
        const timeout = setTimeout(() => { pendingRequests.delete(id); reject(new Error('Browser command timed out: ' + method)); }, 60000);
        pendingRequests.set(id, { resolve, reject, timeout });
        socket.send(JSON.stringify({ id, method, params }));
      });
      const evaluate = async expression => {
        const result = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
        if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
        return result.result.value;
      };
      await send('Page.enable'); await send('Runtime.enable');
      await send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });
      return { send, evaluate, exceptions,
        async close() {
          try { await send('Browser.close'); }
          // The handshake is best effort: a browser that ignores it still has to
          // go, children included, or the next fixture inherits its memory.
          finally { socket.close(); await killBrowserTree(browser); }
        },
      };
    } catch (error) { socket?.close(); await killBrowserTree(browser); throw error; }
  }

  const fixtureRoot = path.join(artifactRoot, 'dist');
  const server = createServer(async (request, response) => {
    const url = new URL(request.url, 'http://localhost');
    const filePath = path.resolve(fixtureRoot, '.' + (url.pathname === '/' ? '/index.html' : url.pathname));
    if (!filePath.startsWith(fixtureRoot + path.sep)) { response.writeHead(403); response.end(); return; }
    try {
      response.setHeader('Content-Type', filePath.endsWith('.js') ? 'text/javascript' : filePath.endsWith('.css') ? 'text/css' : 'text/html');
      response.end(await readFile(filePath));
    } catch { response.writeHead(404); response.end(); }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  let browser;
  console.log('Artifacts: ' + artifactRoot);
  try {
    browser = await openBrowser();
    const checks = await verify({ ...browser, baseUrl: 'http://127.0.0.1:' + server.address().port, artifactRoot });
    if (browser.exceptions.length) throw new Error('Unhandled browser exceptions: ' + JSON.stringify(browser.exceptions));
    await writeFile(path.join(artifactRoot, 'results.json'), JSON.stringify({ checks, browserExceptions: browser.exceptions }, null, 2));
    console.log(`Passed ${checks.length} browser checks.`);
  } catch (error) {
    if (browser) {
      try {
        console.error('Browser exceptions:', JSON.stringify(browser.exceptions));
        await writeFile(path.join(artifactRoot, 'failure.html'), await browser.evaluate('document.documentElement.outerHTML'));
        const { data } = await browser.send('Page.captureScreenshot', { format: 'png' });
        await writeFile(path.join(artifactRoot, 'failure.png'), Buffer.from(data, 'base64'));
      } catch (diagnosticsError) {
        // Diagnostics are best-effort. Letting one fail here would replace the
        // actual assertion error with "Browser command timed out", which is
        // what a reader needs least.
        console.error('Failed to capture failure diagnostics:', diagnosticsError.message);
      }
    }
    throw error;
  } finally {
    // Teardown must never rewrite the verdict — a slow browser exit used to
    // surface as a passing run failing on `Browser.close`.
    try { if (browser) await browser.close(); }
    catch (closeError) { console.error('Browser did not close cleanly:', closeError.message); }
    // `close` waits for open connections, and a keep-alive one would hold this
    // promise forever — turning "the browser went away" into a hung run.
    server.closeAllConnections?.();
    await new Promise(resolve => server.close(resolve));
  }

}
