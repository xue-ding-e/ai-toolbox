import assert from 'node:assert/strict';
import test from 'node:test';

import type { GatewayUsageTrendPoint } from '../../../../../services/proxyGatewayApi.ts';
import {
  buildGatewayTrendBars,
  fillGatewayTrendBuckets,
  formatGatewayCost,
  formatTrendDateLabel,
  gatewayTrendMetricValue,
} from '../../../../../features/coding/gateway/utils/gatewayTrends.ts';

const trendPoint = (
  date: string,
  overrides: Partial<GatewayUsageTrendPoint> = {},
): GatewayUsageTrendPoint => ({
  date,
  request_count: 3,
  total_cost_usd: '1.25',
  total_tokens: 1200,
  input_tokens: 800,
  output_tokens: 400,
  cache_read_tokens: 0,
  cache_creation_tokens: 0,
  ...overrides,
});

test('trend bars map each metric to its own field', () => {
  const point = trendPoint('2026-09-12');

  assert.equal(gatewayTrendMetricValue(point, 'tokens'), 1200);
  assert.equal(gatewayTrendMetricValue(point, 'requests'), 3);
  assert.equal(gatewayTrendMetricValue(point, 'cost'), 1.25);

  assert.equal(buildGatewayTrendBars([point], 'tokens')[0].value, 1200);
  assert.equal(buildGatewayTrendBars([point], 'requests')[0].value, 3);
  assert.equal(buildGatewayTrendBars([point], 'cost')[0].value, 1.25);
});

test('an unparseable cost becomes zero instead of NaN', () => {
  const point = trendPoint('2026-09-12', { total_cost_usd: 'not-a-number' });
  assert.equal(gatewayTrendMetricValue(point, 'cost'), 0);
});

test('bar keys stay unique across years while labels stay compact', () => {
  const rows = buildGatewayTrendBars(
    [trendPoint('2025-09-12'), trendPoint('2026-09-12')],
    'tokens',
  );

  assert.deepEqual(rows.map((row) => row.key), ['2025-09-12', '2026-09-12']);
  assert.deepEqual(rows.map((row) => row.label), ['09/12', '09/12']);
});

test('cost ticks keep meaningful precision at every magnitude', () => {
  assert.equal(formatGatewayCost(0), '$0');
  assert.equal(formatGatewayCost(0.00012), '$0.00012');
  assert.equal(formatGatewayCost(0.004), '$0.0040');
  assert.equal(formatGatewayCost(0.85), '$0.850');
  assert.equal(formatGatewayCost(2), '$2.00');
  assert.equal(formatGatewayCost(1234.5), '$1234.50');
});

test('an hourly window pads the empty hours so the axis keeps its time scale', () => {
  const rows = buildGatewayTrendBars(
    [trendPoint('2026-09-12T07:00:00'), trendPoint('2026-09-12T09:00:00', { total_tokens: 900 })],
    'tokens',
  );
  // Local midnight through 10:00 on the same day.
  const startDate = new Date(2026, 8, 12).getTime() / 1000;
  const endDate = new Date(2026, 8, 12, 10).getTime() / 1000;
  const filled = fillGatewayTrendBuckets(rows, startDate, endDate);

  assert.equal(filled.length, 11);
  assert.equal(filled[0].key, '2026-09-12T00:00:00');
  assert.equal(filled[6].label, '09/12 06:00');
  assert.equal(filled[6].value, 0);
  assert.equal(filled[7].value, 1200);
  assert.equal(filled[8].value, 0);
  assert.equal(filled[9].value, 900);
  assert.deepEqual(filled.map((row) => row.key), [
    '2026-09-12T00:00:00', '2026-09-12T01:00:00', '2026-09-12T02:00:00',
    '2026-09-12T03:00:00', '2026-09-12T04:00:00', '2026-09-12T05:00:00',
    '2026-09-12T06:00:00', '2026-09-12T07:00:00', '2026-09-12T08:00:00',
    '2026-09-12T09:00:00', '2026-09-12T10:00:00',
  ]);
});

test('a multi-day window pads whole days and stops at the end day', () => {
  const rows = buildGatewayTrendBars([trendPoint('2026-09-13')], 'requests');
  const startDate = new Date(2026, 8, 11).getTime() / 1000;
  const endDate = new Date(2026, 8, 15, 18, 30).getTime() / 1000;
  const filled = fillGatewayTrendBuckets(rows, startDate, endDate);

  assert.deepEqual(filled.map((row) => row.label), ['09/11', '09/12', '09/13', '09/14', '09/15']);
  assert.deepEqual(filled.map((row) => row.value), [0, 0, 3, 0, 0]);
});

test('a single-day window keeps hourly buckets even when it is empty', () => {
  const startDate = new Date(2026, 8, 12, 8, 0).getTime() / 1000;
  const endDate = new Date(2026, 8, 12, 11, 59, 59).getTime() / 1000;
  const filled = fillGatewayTrendBuckets([], startDate, endDate);

  assert.deepEqual(filled.map((row) => row.label), [
    '09/12 08:00', '09/12 09:00', '09/12 10:00', '09/12 11:00',
  ]);
  assert.ok(filled.every((row) => row.value === 0));
});

test('day and hour buckets use the same compact tick format', () => {
  assert.equal(formatTrendDateLabel('2026-09-12'), '09/12');
  assert.equal(formatTrendDateLabel('2026-09-12T08:00:00'), '09/12 08:00');
  assert.equal(formatTrendDateLabel('2026-09-12 08:05'), '09/12 08:05');
  // Unrecognized input falls back to the raw value instead of an invalid date.
  assert.equal(formatTrendDateLabel('not-a-date'), 'not-a-date');
});
