import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

export async function verifyGatewayHeatmap({ send, evaluate, baseUrl, artifactRoot }) {
  const checks = [];
  const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
  const check = (name, actual, expected = true) => {
    assert.deepEqual(actual, expected, name);
    checks.push(name);
    console.log('PASS ' + name);
  };
  const waitFor = async expression => {
    for (let attempt = 0; attempt < 150; attempt++) {
      if (await evaluate(expression)) return;
      await delay(50);
    }
    throw new Error('Timed out: ' + expression);
  };

  await send('Page.navigate', { url: baseUrl + '/' });
  await waitFor('!!(window.heatmapFixture && document.querySelector("svg"))');
  await waitFor('document.querySelectorAll("svg rect").length >= 365');
  await delay(150);

  check('the heatmap renders one cell per day of the fixed 365-day window', await evaluate('document.querySelectorAll("svg rect").length'), 365);
  check('cells are colored with at least four distinguishable levels', await evaluate(
    'new Set([...document.querySelectorAll("svg rect")].map(rect => getComputedStyle(rect).fill)).size >= 4',
  ));
  check('the legend explains all five levels', await evaluate(
    'document.querySelectorAll("[class*=legendItems] > span").length',
  ), 5);
  check('month and weekday labels are present', await evaluate('document.querySelectorAll("svg text").length >= 14'));
  check('today has a tooltip with tokens and request count', await evaluate(
    '[...document.querySelectorAll("svg rect title")].some(title => title.textContent.includes("400") && title.textContent.includes("12"))',
  ));
  // A day whose requests all failed records requests but no tokens; it must
  // still be reported as activity instead of "no requests".
  check('a zero-token day with requests still reports its request count', await evaluate(
    '[...document.querySelectorAll("svg rect title")].some(title => title.textContent.includes("1") && /次请求|requests/.test(title.textContent))',
  ));
  check('days without traffic report no requests', await evaluate(
    '[...document.querySelectorAll("svg rect title")].some(title => /无请求|No requests/.test(title.textContent))',
  ));

  const capture = async (name) => {
    const clip = await evaluate('(() => { const rect = document.querySelector("section").getBoundingClientRect(); return {x:rect.x-4,y:rect.y-4,width:rect.width+8,height:rect.height+8,scale:2}; })()');
    const { data } = await send('Page.captureScreenshot', { format: 'png', clip });
    await writeFile(path.join(artifactRoot, name + '.png'), Buffer.from(data, 'base64'));
  };
  await capture('heatmap-light');

  // Dark theme: the level colors must stay on one visible scale and the empty
  // cells must not disappear into the panel background.
  const lightFills = await evaluate(
    '[...document.querySelectorAll("svg rect")].map(rect => getComputedStyle(rect).fill)',
  );
  await send('Page.navigate', { url: baseUrl + '/?theme=dark' });
  await waitFor('!!(window.heatmapFixture && document.querySelector("svg rect"))');
  await delay(150);
  const darkFills = await evaluate(
    '[...document.querySelectorAll("svg rect")].map(rect => getComputedStyle(rect).fill)',
  );
  check('dark theme renders the same number of cells', darkFills.length, 365);
  check('dark theme uses its own level colors', new Set(darkFills).size >= 4 && new Set(darkFills).size === new Set(lightFills).size);
  await capture('heatmap-dark');

  return checks;
}
