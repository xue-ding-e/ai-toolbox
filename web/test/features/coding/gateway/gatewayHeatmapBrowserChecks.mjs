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

  // Hovering a cell opens a labelled card: one metric per row instead of the
  // run-on native SVG title. Tokens use the same compact unit as the bar chart.
  const todayRect = await evaluate(
    '(() => { const rect = [...document.querySelectorAll("svg rect")].find(node => (node.getAttribute("aria-label") || "").includes("$0.0042")); if (!rect) return null; const box = rect.getBoundingClientRect(); return { x: box.x + box.width / 2, y: box.y + box.height / 2 }; })()',
  );
  await evaluate(
    `(() => { const rect = [...document.querySelectorAll("svg rect")].find(node => (node.getAttribute("aria-label") || "").includes("$0.0042")); rect.dispatchEvent(new MouseEvent("mouseover", { bubbles: true, clientX: ${todayRect.x}, clientY: ${todayRect.y} })); return true; })()`,
  );
  await delay(120);
  const tooltipText = await evaluate('document.querySelector("[role=tooltip]")?.textContent ?? ""');
  check('the hover card lists one labelled row per metric', await evaluate(
    'document.querySelectorAll("[role=tooltip] dt").length',
  ), 3);
  check('the hover card keeps every value readable', /400/.test(tooltipText) && tooltipText.includes('12') && tooltipText.includes('$0.0042'));
  check('the hover card separates the labels from the values', await evaluate(
    '[...document.querySelectorAll("[role=tooltip] dt")].map(node => node.textContent).join("|")',
  ), 'Token|请求|费用');
  await evaluate('window.heatmapHoverCard = document.querySelector("[role=tooltip]")?.textContent ?? ""');

  // A day whose requests all failed records requests but no tokens; it must
  // still be reported as activity instead of "no requests".
  check('a zero-token day with requests still reports its request count', await evaluate(
    '[...document.querySelectorAll("svg rect")].some(rect => /\\$0/.test(rect.getAttribute("aria-label") || "") && /次请求|requests/.test(rect.getAttribute("aria-label") || ""))',
  ));
  check('days without traffic report no requests', await evaluate(
    '[...document.querySelectorAll("svg rect")].some(rect => /无请求|No requests/.test(rect.getAttribute("aria-label") || ""))',
  ));

  // The metric switch must recolor the grid, rewrite the legend and keep the
  // tooltip complete: the same three values, only the scale changes.
  const metricButtons = '[...document.querySelectorAll("[role=radio]")]';
  const legendText = 'document.querySelector("[class*=legendItems]").textContent';
  const legendBefore = await evaluate(legendText);
  const fillsBefore = await evaluate(
    '[...document.querySelectorAll("svg rect")].map(rect => getComputedStyle(rect).fill).join(",")',
  );
  await evaluate(
    metricButtons + '.find(button => /请求|Requests/.test(button.textContent)).click()',
  );
  await delay(150);
  const legendAfterRequests = await evaluate(legendText);
  const fillsAfter = await evaluate(
    '[...document.querySelectorAll("svg rect")].map(rect => getComputedStyle(rect).fill).join(",")',
  );
  check('switching to requests rewrites the legend scale', legendAfterRequests !== legendBefore);
  check('switching to requests recolors the cells', fillsAfter !== fillsBefore);
  check('the hover card still carries every metric after the switch', await evaluate(
    'document.querySelectorAll("[role=tooltip] dt").length',
  ), 3);

  await evaluate(
    metricButtons + '.find(button => /费用|Cost/.test(button.textContent)).click()',
  );
  await delay(150);
  check('the cost switch labels the legend in dollars', await evaluate(
    'document.querySelector("[class*=legendItems]").textContent.includes("$")',
  ));

  const capture = async (name) => {
    const clip = await evaluate('(() => { const rect = document.querySelector("section").getBoundingClientRect(); return {x:rect.x-4,y:rect.y-4,width:rect.width+8,height:rect.height+8,scale:2}; })()');
    const { data } = await send('Page.captureScreenshot', { format: 'png', clip });
    await writeFile(path.join(artifactRoot, name + '.png'), Buffer.from(data, 'base64'));
  };
  // Keep the hover card inside the captured area so the artifact shows it.
  const captureWithTooltip = async (name) => {
    const clip = await evaluate('(() => { const card = document.querySelector("[role=tooltip]"); const rect = card ? card.getBoundingClientRect() : null; const section = document.querySelector("section").getBoundingClientRect(); const x = Math.min(section.x, rect?.x ?? section.x) - 4; const y = Math.min(section.y, rect?.y ?? section.y) - 4; const right = Math.max(section.right, rect?.right ?? section.right) + 4; const bottom = Math.max(section.bottom, rect?.bottom ?? section.bottom) + 4; return {x, y, width: right - x, height: bottom - y, scale: 2}; })()');
    const { data } = await send('Page.captureScreenshot', { format: 'png', clip });
    await writeFile(path.join(artifactRoot, name + '.png'), Buffer.from(data, 'base64'));
  };
  await captureWithTooltip('heatmap-light');

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
