import React from 'react';
import { Empty } from 'antd';
import { BarChart3 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  Bar,
  BarChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts';
import type { GatewayUsageTrendPoint } from '@/services';
import { ManagementSegmented } from '../../shared/management';
import {
  buildGatewayTrendBars,
  fillGatewayTrendBuckets,
  formatGatewayCost,
  gatewayMetricLabel,
  GATEWAY_USAGE_METRICS,
  type GatewayUsageMetric,
} from '../utils/gatewayTrends';
import { formatCompactInteger, formatInteger } from '../utils/gatewayFormatters';
import styles from './GatewayStatisticsView.module.less';

interface GatewayUsageTrendChartProps {
  trends: GatewayUsageTrendPoint[];
  /** Inclusive query range (unix seconds); empty buckets inside it are filled. */
  startDate: number;
  endDate: number;
  loading: boolean;
  metric: GatewayUsageMetric;
  onMetricChange: (metric: GatewayUsageMetric) => void;
  /** Localised range name shown beside the title, like the reference header. */
  rangeLabel: string;
}

/**
 * Single-metric trend panel. Only the selected metric is drawn: no multi-series
 * stacking, no second Y axis and no clickable legend, matching the reference
 * dashboard. The metric switch sits in the panel header.
 */
const GatewayUsageTrendChart: React.FC<GatewayUsageTrendChartProps> = ({
  trends,
  startDate,
  endDate,
  loading,
  metric,
  onMetricChange,
  rangeLabel,
}) => {
  const { t, i18n } = useTranslation();
  const rows = React.useMemo(
    () => fillGatewayTrendBuckets(buildGatewayTrendBars(trends, metric), startDate, endDate),
    [trends, metric, startDate, endDate],
  );
  // XAxis categories are the raw bucket timestamps; the visible tick keeps the
  // short `MM/DD` form and never merges two years' same-named days.
  const labelByKey = React.useMemo(
    () => new Map(rows.map((row) => [row.key, row.label])),
    [rows],
  );

  return (
    <section className={styles.chartPanel}>
      <div className={styles.panelHeader}>
        <span>
          <BarChart3 className={styles.panelIcon} size={14} aria-hidden="true" />
          {t('gateway.page.statistics.trendsWithRange', { range: rangeLabel })}
        </span>
        <ManagementSegmented<GatewayUsageMetric>
          value={metric}
          options={GATEWAY_USAGE_METRICS.map((value) => ({
            value,
            label: gatewayMetricLabel(t, value),
          }))}
          onChange={onMetricChange}
          ariaLabel={t('gateway.page.statistics.metrics.trendLabel')}
        />
      </div>
      <div className={styles.chartBody}>
        {trends.length ? (
          <ResponsiveContainer width="100%" height="100%">
            <BarChart data={rows} margin={{ top: 6, right: 8, left: 0, bottom: 0 }}>
              <CartesianGrid stroke="var(--color-border)" strokeDasharray="4 4" vertical={false} />
              <XAxis
                dataKey="key"
                axisLine={false}
                tickLine={false}
                tick={{ fill: 'var(--color-text-tertiary)', fontSize: 11 }}
                tickMargin={6}
                minTickGap={12}
                tickFormatter={(value) => labelByKey.get(String(value)) ?? String(value)}
              />
              <YAxis
                width={52}
                axisLine={false}
                tickLine={false}
                tick={{ fill: 'var(--color-text-tertiary)', fontSize: 11 }}
                tickCount={3}
                tickFormatter={(value) =>
                  metric === 'cost'
                    ? formatGatewayCost(Number(value))
                    // Same locale as the tooltip and the heatmap legend, so the
                    // unit never flips between `40M` and `4000万` when the OS
                    // language differs from the app language.
                    : formatCompactInteger(Number(value), i18n.language)
                }
              />
              <Tooltip
                contentStyle={{
                  background: 'var(--color-bg-elevated)',
                  border: '1px solid var(--color-border)',
                  color: 'var(--color-text-primary)',
                }}
                cursor={{ fill: 'var(--color-bg-hover)' }}
                itemStyle={{ color: 'var(--color-text-primary)' }}
                labelFormatter={(value) => labelByKey.get(String(value)) ?? String(value)}
                formatter={(value) => (
                  <span className={styles.tooltipValue}>
                    {metric === 'cost'
                      ? formatGatewayCost(Number(value))
                      : metric === 'tokens'
                        // Same pairing as the overview's "Token 总量": the exact
                        // count first, the compact approximation beside it.
                        ? t('gateway.page.statistics.metrics.tokenValue', {
                            exact: formatInteger(Number(value)),
                            compact: formatCompactInteger(Number(value), i18n.language),
                          })
                        : formatInteger(Number(value))}
                  </span>
                )}
              />
              <Bar
                dataKey="value"
                name={gatewayMetricLabel(t, metric)}
                fill="var(--ant-color-primary)"
                radius={[3, 3, 0, 0]}
                maxBarSize={36}
                isAnimationActive={false}
              />
            </BarChart>
          </ResponsiveContainer>
        ) : (
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description={loading ? t('common.loading') : t('gateway.page.statistics.empty')}
          />
        )}
      </div>
    </section>
  );
};

export default GatewayUsageTrendChart;
