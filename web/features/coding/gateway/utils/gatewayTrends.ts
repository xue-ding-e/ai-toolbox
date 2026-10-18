import type { GatewayUsageTrendPoint } from '@/services';

/** The three metrics the trend chart and the activity heatmap can render. */
export const GATEWAY_USAGE_METRICS = ['tokens', 'cost', 'requests'] as const;

export type GatewayUsageMetric = (typeof GATEWAY_USAGE_METRICS)[number];

const padTwoDigits = (value: number) => String(value).padStart(2, '0');
const dateOnlyBucketPattern = /^(\d{4})-(\d{2})-(\d{2})$/;
const dateTimeBucketPattern = /^(\d{4})-(\d{2})-(\d{2})[T\s](\d{2}):(\d{2})/;

/**
 * The backend buckets trends by local day, or by hour for windows up to 24h.
 * Both shapes share the same compact `MM/DD[ HH:mm]` tick label.
 */
export const formatTrendDateLabel = (value: string) => {
  const dateTimeMatch = value.match(dateTimeBucketPattern);
  if (dateTimeMatch) {
    const [, , month, day, hour, minute] = dateTimeMatch;
    return `${month}/${day} ${hour}:${minute}`;
  }

  const dateOnlyMatch = value.match(dateOnlyBucketPattern);
  if (dateOnlyMatch) {
    const [, , month, day] = dateOnlyMatch;
    return `${month}/${day}`;
  }

  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }

  const monthDay = `${padTwoDigits(date.getMonth() + 1)}/${padTwoDigits(date.getDate())}`;
  const hasTime = value.includes('T') || /\d{1,2}:\d{2}/.test(value);
  if (!hasTime) {
    return monthDay;
  }
  return `${monthDay} ${padTwoDigits(date.getHours())}:${padTwoDigits(date.getMinutes())}`;
};

export interface GatewayTrendBar {
  /** Raw backend bucket timestamp; unique across years, unlike the display label. */
  key: string;
  label: string;
  value: number;
}

export const gatewayTrendMetricValue = (
  point: GatewayUsageTrendPoint,
  metric: GatewayUsageMetric,
) => {
  if (metric === 'requests') {
    return point.request_count;
  }
  if (metric === 'cost') {
    const cost = Number.parseFloat(point.total_cost_usd);
    return Number.isFinite(cost) ? cost : 0;
  }
  return point.total_tokens;
};

/**
 * Cost values appear on axis ticks, legend bounds and tooltips. Fixed decimals
 * would round every band of a sub-cent window to `$0.0000`, so precision
 * follows the magnitude instead.
 */
export const formatGatewayCost = (value: number) => {
  const magnitude = Math.abs(value);
  if (magnitude === 0) {
    return '$0';
  }
  if (magnitude >= 1) {
    return `$${value.toFixed(2)}`;
  }
  if (magnitude >= 0.01) {
    return `$${value.toFixed(3)}`;
  }
  return `$${value.toPrecision(2)}`;
};

/**
 * Literal `t(...)` calls, one per metric: `scripts/i18n-keys.mjs` only expands
 * object maps declared in the same file, so a shared key map would hide these
 * keys from `i18n:check`.
 */
export const gatewayMetricLabel = (
  t: (key: string) => string,
  metric: GatewayUsageMetric,
) => {
  if (metric === 'cost') {
    return t('gateway.page.statistics.metrics.cost');
  }
  if (metric === 'requests') {
    return t('gateway.page.statistics.metrics.requests');
  }
  return t('gateway.page.statistics.metrics.tokens');
};

export const buildGatewayTrendBars = (
  trends: GatewayUsageTrendPoint[],
  metric: GatewayUsageMetric,
): GatewayTrendBar[] =>
  trends.map((point) => ({
    key: point.date,
    label: formatTrendDateLabel(point.date),
    value: gatewayTrendMetricValue(point, metric),
  }));

const DAY_SECONDS = 24 * 60 * 60;

/**
 * The backend returns only buckets that have rows and switches bucket width at
 * 24h (hourly at or below, daily above). The chart fills the queried range so
 * the X axis keeps a continuous time scale: an empty afternoon hour must still
 * occupy its slot instead of letting the neighbouring bars slide over it.
 *
 * Mirrors the reference dashboard, which pads the same bucket grid server-side.
 */
export const fillGatewayTrendBuckets = (
  rows: GatewayTrendBar[],
  startDate: number,
  endDate: number,
): GatewayTrendBar[] => {
  if (!(startDate > 0) || !(endDate >= startDate)) {
    return rows;
  }
  const byKey = new Map(rows.map((row) => [row.key, row]));
  const hourly = endDate - startDate <= DAY_SECONDS;
  const filled: GatewayTrendBar[] = [];
  if (hourly) {
    // Align to the local hour the backend's `strftime('%Y-%m-%dT%H:00:00')`
    // produces, walking by wall-clock hours so DST cannot skip or repeat one.
    const start = new Date(startDate * 1000);
    const cursor = new Date(start.getFullYear(), start.getMonth(), start.getDate(), start.getHours());
    for (let offset = 0; ; offset += 1) {
      const bucket = new Date(
        cursor.getFullYear(),
        cursor.getMonth(),
        cursor.getDate(),
        cursor.getHours() + offset,
      );
      if (bucket.getTime() > endDate * 1000) {
        break;
      }
      const key = `${bucket.getFullYear()}-${padTwoDigits(bucket.getMonth() + 1)}-${padTwoDigits(bucket.getDate())}T${padTwoDigits(bucket.getHours())}:00:00`;
      filled.push(byKey.get(key) ?? { key, label: formatTrendDateLabel(key), value: 0 });
    }
    return filled;
  }
  const start = new Date(startDate * 1000);
  const end = new Date(endDate * 1000);
  const cursor = new Date(start.getFullYear(), start.getMonth(), start.getDate());
  const lastDay = new Date(end.getFullYear(), end.getMonth(), end.getDate());
  while (cursor.getTime() <= lastDay.getTime()) {
    const key = `${cursor.getFullYear()}-${padTwoDigits(cursor.getMonth() + 1)}-${padTwoDigits(cursor.getDate())}`;
    filled.push(byKey.get(key) ?? { key, label: formatTrendDateLabel(key), value: 0 });
    cursor.setDate(cursor.getDate() + 1);
  }
  return filled;
};
