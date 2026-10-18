import type { GatewayUsageTrendPoint } from '@/services';
import type { GatewayUsageMetric } from './gatewayTrends';
import { gatewayTrendMetricValue } from './gatewayTrends';

export const GATEWAY_HEATMAP_DAY_COUNT = 365;

export interface GatewayHeatmapDay {
  date: string;
  requests: number;
  tokens: number;
  cost: number;
}

interface LocalDateKeyInput {
  getFullYear: () => number;
  getMonth: () => number;
  getDate: () => number;
}

const padTwoDigits = (value: number) => String(value).padStart(2, '0');

const localDateKey = (date: LocalDateKeyInput) =>
  `${date.getFullYear()}-${padTwoDigits(date.getMonth() + 1)}-${padTwoDigits(date.getDate())}`;

/** Parses a stable `YYYY-MM-DD` key as local calendar parts (never UTC). */
const parseLocalDateKey = (key: string) => {
  const [year, month, day] = key.split('-').map(Number);
  return new Date(year, month - 1, day);
};

/**
 * Fixed window for the activity heatmap: the last 365 local calendar days
 * including today. It is intentionally independent of the statistics range
 * filter, matching the reference implementation.
 */
export const resolveGatewayHeatmapRange = (nowMs = Date.now()) => {
  const now = new Date(nowMs);
  const endDate = Math.floor(nowMs / 1000);
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate() - (GATEWAY_HEATMAP_DAY_COUNT - 1));
  return { startDate: Math.floor(start.getTime() / 1000), endDate };
};

/**
 * Builds a dense, ascending calendar from the sparse daily trend points.
 * Date arithmetic walks the local calendar through the Date constructor so a
 * daylight-saving transition cannot skip or duplicate a cell.
 */
export const buildGatewayHeatmapDays = (
  trends: GatewayUsageTrendPoint[],
  nowMs = Date.now(),
): GatewayHeatmapDay[] => {
  const trendsByDate = new Map<string, GatewayUsageTrendPoint>();
  for (const trend of trends) {
    trendsByDate.set(trend.date, trend);
  }

  const now = new Date(nowMs);
  const days: GatewayHeatmapDay[] = [];
  for (let offset = GATEWAY_HEATMAP_DAY_COUNT - 1; offset >= 0; offset -= 1) {
    const date = new Date(now.getFullYear(), now.getMonth(), now.getDate() - offset);
    const key = localDateKey(date);
    const trend = trendsByDate.get(key);
    days.push({
      date: key,
      requests: trend?.request_count ?? 0,
      tokens: trend?.total_tokens ?? 0,
      cost: trend ? gatewayTrendMetricValue(trend, 'cost') : 0,
    });
  }
  return days;
};

/** Reads the day value the selected heatmap metric colors by. */
export const gatewayHeatmapDayValue = (day: GatewayHeatmapDay, metric: GatewayUsageMetric) => {
  if (metric === 'requests') {
    return day.requests;
  }
  if (metric === 'cost') {
    return day.cost;
  }
  return day.tokens;
};

/**
 * Readable step sizes for the four non-zero levels. The step is floored to one
 * of these so thresholds stay round numbers while splitting `peak / 4` into
 * finer bands than equal quarters.
 */
const HEAT_STEPS = [1, 1.25, 1.5, 2, 2.5, 3, 4, 5, 6, 8, 10];

const heatStepFloor = (value: number) => {
  if (!(value > 0)) {
    return 1;
  }
  const magnitude = 10 ** Math.floor(Math.log10(value));
  const normalized = value / magnitude;
  let best = 1;
  for (const step of HEAT_STEPS) {
    if (step <= normalized + 1e-9) {
      best = step;
    }
  }
  return best * magnitude;
};

/**
 * Five closed-interval thresholds: `[0, step, 2step, 3step, Infinity]`.
 * The peak is the largest daily value of the selected metric in the current
 * window, so the levels adapt to how much this app is actually used. Tokens and
 * requests are integers (step floor 1); cost is fractional, so it passes a much
 * smaller floor and small daily spend still splits into four visible bands.
 */
const gatewayHeatLevels = (
  peak: number,
  minimumStep = 1,
): [number, number, number, number, number] => {
  const fallbackStep = minimumStep > 0 ? minimumStep : 1;
  if (!(peak > 0)) {
    return [0, fallbackStep, fallbackStep * 2, fallbackStep * 3, Infinity];
  }
  const step = Math.max(minimumStep, heatStepFloor(peak / 4));
  return [0, step, step * 2, step * 3, Infinity];
};

/**
 * Tokens and requests are integers, so their step never drops below 1. Cost is
 * fractional and must scale with the window's own peak (a $0.004 day still
 * splits into four readable bands); an empty cost window falls back to one cent.
 */
export const gatewayHeatMetricLevels = (
  peak: number,
  metric: GatewayUsageMetric,
): [number, number, number, number, number] =>
  gatewayHeatLevels(peak, metric === 'cost' ? (peak > 0 ? 0 : 0.01) : 1);

export const gatewayHeatLevelOf = (
  tokens: number,
  thresholds: readonly number[],
) => thresholds.findIndex((limit) => tokens <= limit);

export const gatewayHeatLevelText = (
  level: number,
  thresholds: readonly number[],
  formatValue: (value: number) => string,
) => {
  if (level === 0) {
    return '0';
  }
  const lower = thresholds[level - 1];
  const upper = thresholds[level];
  if (!Number.isFinite(upper)) {
    return `>${formatValue(lower)}`;
  }
  return lower === 0 ? `≤${formatValue(upper)}` : `${formatValue(lower)}–${formatValue(upper)}`;
};

/** Rows that carry a weekday label: Monday, Wednesday and Friday (0 = Monday). */
export const HEATMAP_WEEKDAY_ROWS = [0, 2, 4] as const;

export interface GatewayHeatmapLayout {
  width: number;
  height: number;
  /** The one threshold set the cells and the legend both render from. */
  thresholds: [number, number, number, number, number];
  months: Array<{ key: string; month: number; x: number }>;
  weekdays: Array<{ key: string; row: number; x: number; y: number }>;
  cells: Array<GatewayHeatmapDay & { level: number; x: number; y: number; size: number }>;
}

/**
 * GitHub-contribution-style layout: 53 columns (weeks) × 7 rows, Monday first.
 * The cell size is derived from the measured container width so the year fills
 * the panel the same way the reference report does; the size is clamped so a
 * very narrow panel scrolls instead of shrinking cells below readability.
 */
export const buildGatewayHeatmapLayout = (
  days: GatewayHeatmapDay[],
  boxWidth: number,
  metric: GatewayUsageMetric = 'tokens',
): GatewayHeatmapLayout => {
  if (days.length === 0) {
    return {
      width: Math.max(0, Math.round(boxWidth)),
      height: 0,
      thresholds: gatewayHeatMetricLevels(0, metric),
      months: [],
      weekdays: [],
      cells: [],
    };
  }

  const leadingBlanks = (parseLocalDateKey(days[0].date).getDay() + 6) % 7;
  const slots: Array<GatewayHeatmapDay | null> = [
    ...new Array<null>(leadingBlanks).fill(null),
    ...days,
  ];
  const columnCount = Math.ceil(slots.length / 7);

  const gap = 3;
  const weekdayLabelWidth = 20;
  const available = Math.max(0, boxWidth - weekdayLabelWidth);
  const fittedSize = Math.floor((available - gap * (columnCount - 1)) / columnCount);
  const size = Math.max(7, Math.min(26, fittedSize));
  const step = size + gap;
  const topHeight = size + 3;
  const gridWidth = weekdayLabelWidth + columnCount * step - gap;
  const offsetX = Math.max(0, Math.round((boxWidth - gridWidth) / 2));
  const width = Math.max(gridWidth, Math.round(boxWidth));
  const height = topHeight + 7 * step - gap + 2;

  const thresholds = gatewayHeatMetricLevels(
    days.reduce((peak, day) => Math.max(peak, gatewayHeatmapDayValue(day, metric)), 0),
    metric,
  );
  const cells: GatewayHeatmapLayout['cells'] = [];
  for (let column = 0; column < columnCount; column += 1) {
    for (let row = 0; row < 7; row += 1) {
      const day = slots[column * 7 + row];
      if (!day) {
        continue;
      }
      cells.push({
        ...day,
        level: gatewayHeatLevelOf(gatewayHeatmapDayValue(day, metric), thresholds),
        x: offsetX + weekdayLabelWidth + column * step,
        y: topHeight + row * step,
        size,
      });
    }
  }

  const months: GatewayHeatmapLayout['months'] = [];
  let lastMonth = -1;
  let lastLabelColumn = -9;
  for (let column = 0; column < columnCount; column += 1) {
    const head = slots.slice(column * 7, column * 7 + 7).find(Boolean);
    if (!head) {
      continue;
    }
    const month = parseLocalDateKey(head.date).getMonth();
    if (month === lastMonth) {
      continue;
    }
    lastMonth = month;
    // Adjacent labels under three columns apart would overlap.
    if (column - lastLabelColumn < 3) {
      continue;
    }
    lastLabelColumn = column;
    months.push({
      key: `${month}-${column}`,
      month,
      x: offsetX + weekdayLabelWidth + column * step,
    });
  }

  const weekdays = HEATMAP_WEEKDAY_ROWS.map((row) => ({
    key: String(row),
    row,
    x: offsetX + weekdayLabelWidth - 6,
    y: topHeight + row * step + size / 2,
  }));

  return { width, height, thresholds, months, weekdays, cells };
};
