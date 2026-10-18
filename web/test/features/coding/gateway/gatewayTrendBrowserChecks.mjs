import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

export async function verifyGatewayTrendChart({ send, evaluate, baseUrl, artifactRoot }) {
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
  await waitFor('!!window.trendFixture && document.querySelectorAll(".recharts-bar-rectangle").length >= 4');
  await delay(150);

  // The fixture covers midnight..14:00 with four busy hours. The reference
  // dashboard keeps every hour slot, so the first/last tick must span the whole
  // queried range instead of starting at the first non-empty bucket.
  // Recharts 3 renders tick labels into a separate z-index layer, so they are
  // not descendants of `.recharts-xAxis`; anchor on the axis-specific labels
  // group instead.
  const axisTicks = await evaluate(
    '[...document.querySelectorAll(".recharts-xAxis-tick-labels .recharts-cartesian-axis-tick-value")].map(node => node.textContent)',
  );
  const fixtureDay = await evaluate(
    '(() => { const d = new Date(window.trendFixture.startDate * 1000); return `${String(d.getMonth() + 1).padStart(2, "0")}/${String(d.getDate()).padStart(2, "0")}`; })()',
  );
  // The axis previously began at the first non-empty bucket (07:00). Recharts
  // may thin ticks, so only the two ends are asserted, not the tick count.
  check('the hourly axis covers the range start', axisTicks[0], `${fixtureDay} 00:00`);
  check('the hourly axis covers the range end', axisTicks[axisTicks.length - 1], `${fixtureDay} 14:00`);

  // A single-metric bar chart needs less height than the old 410px dual-axis
  // panel, but still reads better than cc-switch's 132px card at full width.
  check('the trend panel keeps the compact 220px body', await evaluate(
    '(() => { const body = document.querySelector("[class*=chartBody]"); const h = body.getBoundingClientRect().height; return h >= 210 && h <= 230; })()',
  ));
  check('the metric switch offers exactly the three dimensions', await evaluate(
    'document.querySelectorAll("[role=radio]").length',
  ), 3);
  check('bars render without a second Y axis', await evaluate(
    'document.querySelectorAll(".recharts-yAxis").length',
  ), 1);

  // Hover the tallest bar to read the tooltip. The bar must be named after the
  // metric ("Token"), not the internal `value` data key, and the number must
  // use the compact form the overview uses.
  const hoverBar = async index => {
    await evaluate(
      `(() => { const bar = document.querySelectorAll(".recharts-bar-rectangle")[${index}];` +
      ' const rect = bar.getBoundingClientRect();' +
      ' bar.dispatchEvent(new MouseEvent("mouseover", { bubbles: true, clientX: rect.x + rect.width / 2, clientY: rect.y + 4 }));' +
      ' bar.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, clientX: rect.x + rect.width / 2, clientY: rect.y + 4 }));' +
      ' return true; })()',
    );
    await delay(120);
    return evaluate('document.querySelector(".recharts-tooltip-wrapper")?.textContent ?? ""');
  };

  const tokenTooltip = await hoverBar(1);
  check('the token tooltip names the metric instead of "value"', /Tokens?|Token/.test(tokenTooltip) && !/value/i.test(tokenTooltip));
  // Matches the overview's "Token 总量" pairing: exact count plus the compact value.
  check('the token tooltip pairs the exact count with the compact unit',
    tokenTooltip.includes('2,103,100') && /2\.1M|210\.3万/.test(tokenTooltip));

  await evaluate(
    '[...document.querySelectorAll("[role=radio]")].find(button => /费用|Cost/.test(button.textContent)).click()',
  );
  await delay(200);
  const costTooltip = await hoverBar(1);
  check('the cost tooltip switches label and currency',
    /费用|Cost/.test(costTooltip) && costTooltip.includes('$0.830'));

  await evaluate(
    '[...document.querySelectorAll("[role=radio]")].find(button => /请求|Requests/.test(button.textContent)).click()',
  );
  await delay(200);
  const requestTooltip = await hoverBar(1);
  check('the request tooltip shows the raw count', /请求|Requests/.test(requestTooltip) && requestTooltip.includes('12'));

  // Capture the panel for visual review, then repeat in dark mode.
  const capture = async name => {
    const clip = await evaluate('(() => { const rect = document.querySelector("section").getBoundingClientRect(); return {x:rect.x-4,y:rect.y-4,width:rect.width+8,height:rect.height+8,scale:2}; })()');
    const { data } = await send('Page.captureScreenshot', { format: 'png', clip });
    await writeFile(path.join(artifactRoot, name + '.png'), Buffer.from(data, 'base64'));
  };
  await capture('trend-light');

  await send('Page.navigate', { url: baseUrl + '/?theme=dark' });
  await waitFor('!!window.trendFixture && document.querySelectorAll(".recharts-bar-rectangle").length >= 4');
  await delay(150);
  check('dark theme renders the same bars', await evaluate(
    'document.querySelectorAll(".recharts-bar-rectangle").length',
  ), 4);
  await capture('trend-dark');

  return checks;
}
