import assert from 'node:assert/strict';
import test from 'node:test';

import type { GatewayUsageTrendPoint } from '../../../../../services/proxyGatewayApi.ts';
import {
  GATEWAY_HEATMAP_DAY_COUNT,
  buildGatewayHeatmapDays,
  buildGatewayHeatmapLayout,
  gatewayHeatLevelOf,
  gatewayHeatLevels,
  gatewayHeatLevelText,
  resolveGatewayHeatmapRange,
} from '../../../../../features/coding/gateway/utils/gatewayHeatmap.ts';

const trendPoint = (date: string, request_count: number, total_tokens: number): GatewayUsageTrendPoint => ({
  date,
  request_count,
  total_cost_usd: '0',
  total_tokens,
  input_tokens: 0,
  output_tokens: 0,
  cache_read_tokens: 0,
  cache_creation_tokens: 0,
});

test('heatmap days always cover 365 local days ending today and fill missing ones with zero', () => {
  const now = new Date(2026, 8, 12, 22, 24, 0).getTime();
  const days = buildGatewayHeatmapDays([
    trendPoint('2026-09-12', 3, 300),
    trendPoint('2026-09-10', 1, 50),
  ], now);

  assert.equal(days.length, GATEWAY_HEATMAP_DAY_COUNT);
  assert.equal(days[0].date, '2025-09-13');
  assert.equal(days[days.length - 1].date, '2026-09-12');
  assert.deepEqual(days[days.length - 1], { date: '2026-09-12', requests: 3, tokens: 300 });
  assert.deepEqual(days.find((day) => day.date === '2026-09-10'), {
    date: '2026-09-10',
    requests: 1,
    tokens: 50,
  });
  assert.deepEqual(days.find((day) => day.date === '2026-09-11'), {
    date: '2026-09-11',
    requests: 0,
    tokens: 0,
  });
});

test('heatmap request range is fixed at 365 days regardless of the statistics filter', () => {
  const now = new Date(2026, 8, 12, 22, 24, 0).getTime();
  assert.deepEqual(resolveGatewayHeatmapRange(now), {
    startDate: Math.floor(new Date(2025, 8, 13).getTime() / 1000),
    endDate: Math.floor(now / 1000),
  });
});

test('heatmap levels split the peak into four readable bands and handle an empty window', () => {
  assert.deepEqual(gatewayHeatLevels(0), [0, 1, 2, 3, Infinity]);
  assert.deepEqual(gatewayHeatLevels(400), [0, 100, 200, 300, Infinity]);
  assert.deepEqual(gatewayHeatLevels(1000), [0, 250, 500, 750, Infinity]);
  // 3.3e8 would be an unreadable equal quarter; the step floors to a round value.
  assert.deepEqual(gatewayHeatLevels(330_000_000), [0, 80_000_000, 160_000_000, 240_000_000, Infinity]);
  // Token totals are integers; the step never drops below 1.
  assert.deepEqual(gatewayHeatLevels(3), [0, 1, 2, 3, Infinity]);

  const thresholds = gatewayHeatLevels(400);
  assert.equal(gatewayHeatLevelOf(0, thresholds), 0);
  assert.equal(gatewayHeatLevelOf(100, thresholds), 1);
  assert.equal(gatewayHeatLevelOf(101, thresholds), 2);
  assert.equal(gatewayHeatLevelOf(400, thresholds), 4);
});

test('heatmap legend text keeps the closed upper bound and the open top band', () => {
  const thresholds = gatewayHeatLevels(400);
  const format = (value: number) => String(value);
  assert.equal(gatewayHeatLevelText(0, thresholds, format), '0');
  assert.equal(gatewayHeatLevelText(1, thresholds, format), '≤100');
  assert.equal(gatewayHeatLevelText(2, thresholds, format), '100–200');
  assert.equal(gatewayHeatLevelText(4, thresholds, format), '>300');
});

test('heatmap layout starts weeks on Monday and pads the first column', () => {
  const now = new Date(2026, 8, 12, 22, 24, 0).getTime();
  const days = buildGatewayHeatmapDays([], now);
  const layout = buildGatewayHeatmapLayout(days, 1000);

  assert.equal(layout.cells.length, GATEWAY_HEATMAP_DAY_COUNT);
  // 2025-09-13 is a Saturday, so the first column has five empty slots (Mon–Fri).
  const firstCell = layout.cells[0];
  assert.equal(firstCell.date, '2025-09-13');
  const firstColumn = layout.cells.filter((cell) => cell.x === firstCell.x);
  assert.equal(firstColumn.length, 2);
  // Monday is the top row, so the first cell sits five rows down.
  assert.ok(firstCell.y > layout.cells.find((cell) => cell.date === '2025-09-15')!.y);

  assert.equal(layout.weekdays.map((item) => item.row).join(','), '0,2,4');
  assert.ok(layout.months.length >= 11);
  assert.equal(layout.months[0].month, 8);
});

test('heatmap layout fills the measured width and reports a level per day', () => {
  const now = new Date(2026, 8, 12, 22, 24, 0).getTime();
  const days = buildGatewayHeatmapDays([trendPoint('2026-09-12', 5, 480)], now);
  const layout = buildGatewayHeatmapLayout(days, 1000);

  const maxRight = Math.max(...layout.cells.map((cell) => cell.x + cell.size));
  assert.ok(maxRight <= 1000);
  assert.ok(maxRight > 900);
  assert.equal(layout.cells[layout.cells.length - 1].level, 4);
  assert.equal(layout.cells[0].level, 0);
  assert.equal(buildGatewayHeatmapLayout([], 640).cells.length, 0);
});
