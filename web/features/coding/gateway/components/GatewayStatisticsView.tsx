import React from 'react';
import { DatePicker, Empty, Select, Table } from 'antd';
import dayjs from 'dayjs';
import type { ColumnsType } from 'antd/es/table';
import {
  AlertCircle,
  CalendarDays,
  Clock,
  DollarSign,
  FolderTree,
  Gauge,
  RefreshCw,
  Server,
  Terminal,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { ManagementSegmented } from '../../shared/management';
import {
  getGatewayUsageRange,
  getProxyGatewayModelStats,
  getProxyGatewayProjectStats,
  getProxyGatewayProviderStats,
  getProxyGatewayUsageSummary,
  getProxyGatewayUsageTrends,
  saveGatewayUsageRange,
  type GatewayUsageRangeValue,
  type GatewayUsageTool,
  GATEWAY_USAGE_TOOLS,
  type GatewayModelStats,
  type GatewayProjectStats,
  type GatewayProviderStats,
  type GatewayUsageSummary,
  type GatewayUsageTrendPoint,
  type ProxyGatewayStatus,
} from '@/services';
import {
  formatCacheHitRate,
  formatCompactInteger,
  formatDuration,
  formatGatewayError,
  formatInteger,
  gatewayProjectName,
  formatUsd,
  gatewayUsageRangeKey,
  gatewayUsageRangeToStored,
  getGatewayRequestsPerMinute,
  GATEWAY_USAGE_RANGE_PRESETS,
  isGatewayUsageRangePreset,
  resolveGatewayUsageRange,
  unpricedModelIds,
  type GatewayUsageRangePreset,
  type GatewayUsageRangeSelection,
  type ResolvedGatewayUsageRange,
} from '../utils/gatewayFormatters';
import ModelPricingModal from './ModelPricingModal';
import GatewayUsageOverview from './GatewayUsageOverview';
import GatewayActivityHeatmap from './GatewayActivityHeatmap';
import GatewayUsageTrendChart from './GatewayUsageTrendChart';
import { resolveGatewayHeatmapRange } from '../utils/gatewayHeatmap';
import type { GatewayUsageMetric } from '../utils/gatewayTrends';
import styles from './GatewayStatisticsView.module.less';

const { RangePicker } = DatePicker;

type GatewayCustomRange = [ReturnType<typeof dayjs>, ReturnType<typeof dayjs>];

/**
 * Restore a persisted range into the picker's value type. Relative presets
 * need no bounds; `custom` rebuilds dayjs objects from the stored unix seconds.
 */
const restoreRangeSelection = (stored: GatewayUsageRangeValue): GatewayUsageRangeSelection => {
  const preset: GatewayUsageRangePreset = isGatewayUsageRangePreset(stored.preset)
    ? stored.preset
    : 'today';
  if (preset !== 'custom') {
    return { preset };
  }
  const start = stored.start_date ? dayjs(stored.start_date * 1000) : null;
  const end = stored.end_date ? dayjs(stored.end_date * 1000) : null;
  return {
    preset: 'custom',
    customRange: (start || end ? [start, end] : null) as GatewayCustomRange | null,
  };
};

type GatewayCliFilter = 'all' | GatewayUsageTool;
type StatsTabKey = 'providers' | 'models' | 'projects';

interface GatewayStatisticsViewProps {
  refreshKey?: number;
  gatewayStatus?: ProxyGatewayStatus | null;
}

interface StatisticsState {
  summary: GatewayUsageSummary | null;
  trends: GatewayUsageTrendPoint[];
  // The range the displayed trends were queried with. The chart pads empty
  // buckets up to these bounds, so it must be the range that produced the
  // data, not one re-resolved from the preset at render time.
  trendRange: ResolvedGatewayUsageRange | null;
  providerStats: GatewayProviderStats[];
  modelStats: GatewayModelStats[];
  projectStats: GatewayProjectStats[];
}

const emptyState: StatisticsState = {
  summary: null,
  trends: [],
  trendRange: null,
  providerStats: [],
  modelStats: [],
  projectStats: [],
};

const cliOptions: GatewayCliFilter[] = ['all', ...GATEWAY_USAGE_TOOLS];
/** Model ids are named inline in a 10px hint line; past a few the count suffices. */
const UNPRICED_MODEL_NAME_LIMIT = 3;

const toCliKey = (value: GatewayCliFilter): GatewayUsageTool | undefined =>
  value === 'all' ? undefined : value;

const statusColor = (rate: number) => {
  if (rate >= 95) {
    return 'var(--color-status-success)';
  }
  if (rate >= 80) {
    return 'var(--color-status-warning)';
  }
  return 'var(--color-status-error)';
};

const providerDisplayName = (
  t: ReturnType<typeof useTranslation>['t'],
  providerId: string,
  providerName: string | null,
) => {
  if (providerName) {
    return providerName;
  }
  if (providerId === 'session') {
    return t('gateway.page.requests.localSession');
  }
  if (providerId === 'unknown') {
    return t('gateway.page.statistics.providerUnselected');
  }
  return providerId;
};

const providerDisplayMeta = (
  t: ReturnType<typeof useTranslation>['t'],
  cliKey: GatewayUsageTool,
  providerId: string,
) => {
  const cliLabel = t(`settings.gateway.cli.${cliKey}`);
  if (providerId === 'unknown' || providerId === 'session') {
    return cliLabel;
  }
  return `${cliLabel} · ${providerId}`;
};

const GatewayStatisticsView: React.FC<GatewayStatisticsViewProps> = ({ refreshKey = 0, gatewayStatus }) => {
  const { t } = useTranslation();
  const [cliFilter, setCliFilter] = React.useState<GatewayCliFilter>('all');
  const [range, setRange] = React.useState<GatewayUsageRangeSelection>({ preset: 'today' });
  // The saved range is restored before the first query so the page never
  // briefly loads "today" and then jumps to the user's actual window (and the
  // tray never sees that intermediate range written back).
  const [rangeRestored, setRangeRestored] = React.useState(false);
  const lastSavedRangeKeyRef = React.useRef<string | null>(null);
  const [activeStatsTab, setActiveStatsTab] = React.useState<StatsTabKey>('providers');
  const [refreshIntervalMs, setRefreshIntervalMs] = React.useState(30_000);
  const [showPricingModal, setShowPricingModal] = React.useState(false);
  // The trend and the heatmap use different time windows, so each panel keeps
  // its own metric instead of sharing one selection.
  const [trendMetric, setTrendMetric] = React.useState<GatewayUsageMetric>('tokens');
  const [heatmapMetric, setHeatmapMetric] = React.useState<GatewayUsageMetric>('tokens');
  const [state, setState] = React.useState<StatisticsState>(emptyState);
  const [heatmapTrends, setHeatmapTrends] = React.useState<GatewayUsageTrendPoint[]>([]);
  const [loading, setLoading] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const statisticsRequestIdRef = React.useRef(0);
  const heatmapRequestIdRef = React.useRef(0);

  const effectiveCliKey = toCliKey(cliFilter);
  const unpricedModels = React.useMemo(
    () => unpricedModelIds(state.modelStats),
    [state.modelStats],
  );

  const loadStatistics = React.useCallback(async () => {
    const requestId = ++statisticsRequestIdRef.current;
    setLoading(true);
    setError(null);
    try {
      const resolvedRange = resolveGatewayUsageRange(range);
      const [summary, trends, providerStats, modelStats, projectStats] = await Promise.all([
        getProxyGatewayUsageSummary(
          resolvedRange.startDate,
          resolvedRange.endDate,
          effectiveCliKey,
        ),
        getProxyGatewayUsageTrends(
          resolvedRange.startDate,
          resolvedRange.endDate,
          effectiveCliKey,
        ),
        getProxyGatewayProviderStats(
          resolvedRange.startDate,
          resolvedRange.endDate,
          effectiveCliKey,
        ),
        getProxyGatewayModelStats(
          resolvedRange.startDate,
          resolvedRange.endDate,
          effectiveCliKey,
        ),
        getProxyGatewayProjectStats(
          resolvedRange.startDate,
          resolvedRange.endDate,
          effectiveCliKey,
        ),
      ]);
      if (requestId === statisticsRequestIdRef.current) {
        setState({
          summary,
          trends,
          trendRange: resolvedRange,
          providerStats,
          modelStats,
          projectStats,
        });
      }
    } catch (loadError) {
      if (requestId === statisticsRequestIdRef.current) {
        setError(t('gateway.page.statistics.loadFailed', { error: formatGatewayError(loadError) }));
      }
    } finally {
      if (requestId === statisticsRequestIdRef.current) {
        setLoading(false);
      }
    }
  }, [effectiveCliKey, range, t]);

  // The heatmap window is fixed at 365 days and independent of the range
  // filter, so it only follows the CLI filter and the refresh triggers.
  const loadHeatmap = React.useCallback(async () => {
    const requestId = ++heatmapRequestIdRef.current;
    const heatmapRange = resolveGatewayHeatmapRange();
    try {
      const trends = await getProxyGatewayUsageTrends(
        heatmapRange.startDate,
        heatmapRange.endDate,
        effectiveCliKey,
      );
      if (requestId === heatmapRequestIdRef.current) {
        setHeatmapTrends(trends);
      }
    } catch {
      // A failed refresh keeps the previous calendar instead of blanking the
      // panel, and stays out of the statistics error banner: the heatmap is a
      // secondary view whose own range is not part of the main filter.
    }
  }, [effectiveCliKey]);

  // Restore the persisted range once, before the first statistics query. The
  // effect below only saves ranges the user actually picked, so this read-back
  // can never echo a default value over the stored one.
  React.useEffect(() => {
    let disposed = false;
    void (async () => {
      try {
        const stored = await getGatewayUsageRange();
        if (disposed) return;
        const restored = restoreRangeSelection(stored);
        lastSavedRangeKeyRef.current = gatewayUsageRangeKey(restored);
        setRange(restored);
      } catch {
        // Keeping the page default (`today`) is safe: the tray already falls
        // back to "today" for a missing field, so both stay consistent. Treat
        // it as already saved so this failed read alone never writes a row.
        if (!disposed) {
          lastSavedRangeKeyRef.current = gatewayUsageRangeKey({ preset: 'today' });
        }
      } finally {
        if (!disposed) setRangeRestored(true);
      }
    })();
    return () => {
      disposed = true;
    };
  }, []);

  // Mirror the user's range into settings so the tray usage block follows it.
  // The tray rebuild is triggered by the backend command.
  React.useEffect(() => {
    if (!rangeRestored) return;
    const key = gatewayUsageRangeKey(range);
    if (key === lastSavedRangeKeyRef.current) return;
    lastSavedRangeKeyRef.current = key;
    void saveGatewayUsageRange(gatewayUsageRangeToStored(range)).catch((saveError) => {
      console.error('Failed to save gateway usage range:', saveError);
    });
  }, [range, rangeRestored]);

  React.useEffect(() => {
    // A new filter must not display the previous filter's totals while loading.
    setState(emptyState);
  }, [effectiveCliKey, range]);

  React.useEffect(() => {
    setHeatmapTrends([]);
  }, [effectiveCliKey]);

  React.useEffect(() => {
    // Wait for the restore so the first query already uses the saved window;
    // otherwise the page flashes today's totals before the read-back lands.
    if (!rangeRestored) return undefined;
    void loadStatistics();
    return () => {
      statisticsRequestIdRef.current += 1;
    };
  }, [loadStatistics, refreshKey, rangeRestored]);

  React.useEffect(() => {
    void loadHeatmap();
    return () => {
      heatmapRequestIdRef.current += 1;
    };
  }, [loadHeatmap, refreshKey]);

  React.useEffect(() => {
    if (refreshIntervalMs <= 0) {
      return undefined;
    }
    const timer = window.setInterval(() => {
      void loadStatistics();
      void loadHeatmap();
    }, refreshIntervalMs);
    return () => window.clearInterval(timer);
  }, [loadStatistics, loadHeatmap, refreshIntervalMs]);

  const requestRate = getGatewayRequestsPerMinute(gatewayStatus, effectiveCliKey);

  const providerColumns: ColumnsType<GatewayProviderStats> = [
    {
      title: t('gateway.page.statistics.columns.provider'),
      dataIndex: 'provider_name',
      render: (_, record) => (
        <div className={styles.tableMainCell}>
          <strong title={providerDisplayName(t, record.provider_id, record.provider_name)}>{providerDisplayName(t, record.provider_id, record.provider_name)}</strong>
          <small>{providerDisplayMeta(t, record.cli_key, record.provider_id)}</small>
        </div>
      ),
    },
    {
      title: t('gateway.page.statistics.columns.requests'),
      dataIndex: 'request_count',
      width: 110,
      align: 'right',
      render: (value: number) => formatInteger(value),
    },
    {
      title: t('gateway.page.statistics.columns.tokens'),
      dataIndex: 'total_tokens',
      width: 130,
      align: 'right',
      render: (value: number) => formatCompactInteger(value),
    },
    {
      title: <span title={t('gateway.page.statistics.cacheHitRateHint')}>{t('gateway.page.statistics.columns.cacheHitRate')}</span>,
      dataIndex: 'cache_hit_rate',
      width: 120,
      align: 'right',
      render: (value: number | null) => formatCacheHitRate(value),
    },
    {
      title: t('gateway.page.statistics.columns.successRate'),
      dataIndex: 'success_rate',
      width: 110,
      align: 'right',
      render: (value: number, record) => record.provider_id === 'session' ? '-' : (
        <span style={{ color: statusColor(value) }}>{value.toFixed(1)}%</span>
      ),
    },
    {
      title: <span title={t('gateway.page.statistics.avgLatencyHint')}>{t('gateway.page.statistics.columns.latency')}</span>,
      dataIndex: 'avg_latency_ms',
      width: 110,
      align: 'right',
      render: (value: number | null) => value == null ? '-' : formatDuration(value),
    },
    {
      title: t('gateway.page.statistics.columns.cost'),
      dataIndex: 'total_cost_usd',
      width: 120,
      align: 'right',
      render: (value: string) => formatUsd(value, 6),
    },
  ];

  const modelColumns: ColumnsType<GatewayModelStats> = [
    {
      title: t('gateway.page.statistics.columns.model'),
      dataIndex: 'model',
      render: (value: string, record) => (
        <div className={styles.tableMainCell}>
          <strong title={value === 'unknown' ? undefined : value}>
            {value === 'unknown' ? t('gateway.page.statistics.modelUnavailable') : value}
          </strong>
          <small>{t(`settings.gateway.cli.${record.cli_key}`)}</small>
          {record.has_pricing === false && value !== 'unknown' ? (
            <small className={styles.unpricedTag}>
              {t('gateway.page.statistics.unpricedModelTag')}
            </small>
          ) : null}
        </div>
      ),
    },
    {
      title: t('gateway.page.statistics.columns.requests'),
      dataIndex: 'request_count',
      width: 110,
      align: 'right',
      render: (value: number) => formatInteger(value),
    },
    {
      title: t('gateway.page.statistics.columns.tokens'),
      dataIndex: 'total_tokens',
      width: 130,
      align: 'right',
      render: (value: number) => formatCompactInteger(value),
    },
    {
      title: <span title={t('gateway.page.statistics.cacheHitRateHint')}>{t('gateway.page.statistics.columns.cacheHitRate')}</span>,
      dataIndex: 'cache_hit_rate',
      width: 120,
      align: 'right',
      render: (value: number | null) => formatCacheHitRate(value),
    },
    {
      title: t('gateway.page.statistics.columns.successRate'),
      dataIndex: 'success_rate',
      width: 110,
      align: 'right',
      render: (value: number | null) => value == null ? '-' : (
        <span style={{ color: statusColor(value) }}>{value.toFixed(1)}%</span>
      ),
    },
    {
      title: <span title={t('gateway.page.statistics.avgLatencyHint')}>{t('gateway.page.statistics.columns.latency')}</span>,
      dataIndex: 'avg_latency_ms',
      width: 110,
      align: 'right',
      render: (value: number | null) => value == null ? '-' : formatDuration(value),
    },
    {
      title: t('gateway.page.statistics.columns.cost'),
      dataIndex: 'total_cost_usd',
      width: 120,
      align: 'right',
      render: (value: string) => formatUsd(value, 6),
    },
  ];

  const projectColumns: ColumnsType<GatewayProjectStats> = [
    {
      title: t('gateway.page.statistics.columns.project'),
      dataIndex: 'project_path',
      render: (value: string, record) => {
        const name = gatewayProjectName(value);
        return (
          <div className={styles.tableMainCell}>
            <strong title={name ? value : undefined}>
              {name ?? t('gateway.page.statistics.unknownProject')}
            </strong>
            <small title={record.cli_keys.map((key) => t(`settings.gateway.cli.${key}`)).join(' · ')}>
              {record.cli_keys.length
                ? record.cli_keys.map((key) => t(`settings.gateway.cli.${key}`)).join(' · ')
                : t('gateway.page.statistics.unknownProjectHint')}
            </small>
            {name ? <small>{value}</small> : null}
          </div>
        );
      },
    },
    {
      title: t('gateway.page.statistics.columns.sessions'),
      dataIndex: 'session_count',
      width: 110,
      align: 'right',
      render: (value: number) => formatInteger(value),
    },
    {
      title: t('gateway.page.statistics.columns.requests'),
      dataIndex: 'request_count',
      width: 110,
      align: 'right',
      render: (value: number) => formatInteger(value),
    },
    {
      title: t('gateway.page.statistics.columns.tokens'),
      dataIndex: 'total_tokens',
      width: 130,
      align: 'right',
      render: (value: number) => formatCompactInteger(value),
    },
    {
      title: t('gateway.page.statistics.columns.topModel'),
      dataIndex: 'top_model',
      width: 170,
      render: (value: string | null) => value ?? '-',
    },
    {
      title: t('gateway.page.statistics.columns.lastActive'),
      dataIndex: 'last_active',
      width: 120,
      align: 'right',
      render: (value: string | null) => value ?? '-',
    },
    {
      title: t('gateway.page.statistics.columns.cost'),
      dataIndex: 'total_cost_usd',
      width: 120,
      align: 'right',
      render: (value: string) => formatUsd(value, 6),
    },
  ];

  return (
    <div className={styles.viewStack} aria-busy={loading}>
      {error ? (
        <div className={styles.inlineAlert} role="alert">
          <AlertCircle size={14} aria-hidden="true" />
          <span>{error}</span>
        </div>
      ) : null}

      <div className={styles.filterBar}>
        <div className={styles.filterSection}>
          <Terminal className={styles.filterIcon} size={14} aria-hidden="true" />
          <Select
            variant="borderless"
            size="small"
            className={styles.filterSelect}
            popupMatchSelectWidth={false}
            value={cliFilter}
            options={cliOptions.map((option) => ({
              value: option,
              label: option === 'all' ? t('gateway.page.statistics.filters.all') : t(`settings.gateway.cli.${option}`),
            }))}
            onChange={(value) => setCliFilter(value as GatewayCliFilter)}
          />
        </div>

        <span className={styles.filterDivider} aria-hidden="true" />

        <div className={styles.filterSection}>
          <CalendarDays className={styles.filterIcon} size={14} aria-hidden="true" />
          <Select
            variant="borderless"
            size="small"
            className={styles.filterSelect}
            popupMatchSelectWidth={false}
            value={range.preset}
            options={GATEWAY_USAGE_RANGE_PRESETS.map((option) => ({
              value: option,
              label: t(`gateway.page.statistics.range.${option}`),
            }))}
            onChange={(value) =>
              setRange((currentRange) => ({
                preset: value as GatewayUsageRangePreset,
                customRange: value === 'custom' ? currentRange.customRange : undefined,
              }))
            }
          />
          {range.preset === 'custom' ? (
            <RangePicker
              showTime
              variant="borderless"
              size="small"
              className={styles.customRangePicker}
              value={range.customRange as never}
              onChange={(dates) =>
                setRange({ preset: 'custom', customRange: dates as GatewayCustomRange | null })
              }
            />
          ) : null}
        </div>

        <div className={styles.filterActions}>
          <div className={styles.filterSection}>
            <Clock className={styles.filterIcon} size={14} aria-hidden="true" />
            <Select
              variant="borderless"
              size="small"
              className={styles.filterSelect}
              popupMatchSelectWidth={false}
              value={refreshIntervalMs}
              options={[
                { value: 0, label: t('gateway.page.statistics.refresh.off') },
                { value: 5_000, label: '5s' },
                { value: 10_000, label: '10s' },
                { value: 30_000, label: '30s' },
                { value: 60_000, label: '60s' },
              ]}
              onChange={(value) => setRefreshIntervalMs(Number(value))}
            />
          </div>
          <button
            type="button"
            className={styles.refreshButton}
            onClick={() => void loadStatistics()}
            disabled={loading}
            aria-label={t('common.refresh')}
            title={t('common.refresh')}
          >
            <RefreshCw size={14} className={loading ? styles.spin : undefined} aria-hidden="true" />
          </button>
          <button
            type="button"
            className={styles.refreshButton}
            onClick={() => setShowPricingModal(true)}
            aria-label={t('gateway.page.pricing.open')}
            title={t('gateway.page.pricing.open')}
          >
            <DollarSign size={14} aria-hidden="true" />
          </button>
        </div>
      </div>

      <GatewayUsageOverview summary={state.summary} requestsPerMinute={requestRate} />
      <div className={styles.hintGroup}>
        <p className={styles.usageNote}>{t('gateway.page.requests.nativeUsage.overviewHint')}</p>
        {unpricedModels.length > 0 ? (
          <>
            <p className={styles.usageNote}>
              {t('gateway.page.statistics.unpricedModelsHint', { count: unpricedModels.length })}
              {' '}
              {unpricedModels.slice(0, UNPRICED_MODEL_NAME_LIMIT).map((model) => (
                <code key={model} className={styles.unpricedModelName}>
                  {model}
                </code>
              ))}
              {unpricedModels.length > UNPRICED_MODEL_NAME_LIMIT
                ? t('gateway.page.statistics.unpricedModelsMore', {
                    count: unpricedModels.length - UNPRICED_MODEL_NAME_LIMIT,
                  })
                : ''}
            </p>
            <button
              type="button"
              className={styles.hintAction}
              onClick={() => setShowPricingModal(true)}
            >
              {t('gateway.page.statistics.unpricedModelsAction')}
            </button>
          </>
        ) : null}
      </div>

      <GatewayUsageTrendChart
        trends={state.trends}
        startDate={state.trendRange?.startDate ?? 0}
        endDate={state.trendRange?.endDate ?? 0}
        loading={loading}
        metric={trendMetric}
        onMetricChange={setTrendMetric}
        rangeLabel={
          range.preset === 'custom'
            ? t('gateway.page.statistics.range.custom')
            : t(`gateway.page.statistics.range.${range.preset}`)
        }
      />

      <section className={styles.dataPanel}>
        <div className={styles.panelHeader}>
          <span>
            {activeStatsTab === 'providers' ? (
              <Server className={styles.panelIcon} size={14} aria-hidden="true" />
            ) : activeStatsTab === 'models' ? (
              <Gauge className={styles.panelIcon} size={14} aria-hidden="true" />
            ) : (
              <FolderTree className={styles.panelIcon} size={14} aria-hidden="true" />
            )}
            {t('gateway.page.statistics.breakdown')}
          </span>
          <ManagementSegmented<StatsTabKey>
            value={activeStatsTab}
            options={[
              { value: 'providers', label: t('gateway.page.statistics.providerStats') },
              { value: 'models', label: t('gateway.page.statistics.modelStats') },
              { value: 'projects', label: t('gateway.page.statistics.projectStats') },
            ]}
            onChange={setActiveStatsTab}
            ariaLabel={t('gateway.page.statistics.breakdown')}
          />
        </div>
        {activeStatsTab === 'providers' ? (
          <Table
            rowKey={(record) => `${record.cli_key}:${record.provider_id}`}
            size="small"
            tableLayout="fixed"
            columns={providerColumns}
            dataSource={state.providerStats}
            loading={loading}
            pagination={false}
            scroll={{ x: 880 }}
          />
        ) : activeStatsTab === 'models' ? (
          <Table
            rowKey={(record) => `${record.cli_key}:${record.model}`}
            size="small"
            tableLayout="fixed"
            columns={modelColumns}
            dataSource={state.modelStats}
            loading={loading}
            pagination={false}
            scroll={{ x: 880 }}
            expandable={{
              // The prices are the ones the backend actually billed, so the
              // expanded row never disagrees with the amount beside it.
              expandedRowRender: (record) => (
                <div className={styles.pricingDetail}>
                  {record.pricing ? (
                    <>
                      <span className={styles.pricingDetailLabel}>
                        {t('gateway.page.statistics.pricingUnit')}
                      </span>
                      <span>
                        {t('gateway.page.statistics.pricingInput', {
                          value: formatUsd(record.pricing.input_cost_per_million, 4),
                        })}
                      </span>
                      <span>
                        {t('gateway.page.statistics.pricingOutput', {
                          value: formatUsd(record.pricing.output_cost_per_million, 4),
                        })}
                      </span>
                      <span>
                        {t('gateway.page.statistics.pricingCacheRead', {
                          value: formatUsd(record.pricing.cache_read_cost_per_million, 4),
                        })}
                      </span>
                      <span>
                        {t('gateway.page.statistics.pricingCacheCreation', {
                          value: formatUsd(record.pricing.cache_creation_cost_per_million, 4),
                        })}
                      </span>
                    </>
                  ) : (
                    <span>{t('gateway.page.statistics.pricingUnavailable')}</span>
                  )}
                </div>
              ),
            }}
          />
        ) : (
          <Table
            rowKey={(record) => record.project_path}
            size="small"
            tableLayout="fixed"
            columns={projectColumns}
            dataSource={state.projectStats}
            loading={loading}
            pagination={false}
            scroll={{ x: 960 }}
            locale={{
              emptyText: (
                <Empty
                  image={Empty.PRESENTED_IMAGE_SIMPLE}
                  description={
                    loading
                      ? t('common.loading')
                      : t('gateway.page.statistics.projectEmpty')
                  }
                />
              ),
            }}
          />
        )}
      </section>

      <GatewayActivityHeatmap
        trends={heatmapTrends}
        loading={loading}
        metric={heatmapMetric}
        onMetricChange={setHeatmapMetric}
      />

      <ModelPricingModal
        open={showPricingModal}
        onClose={() => setShowPricingModal(false)}
      />
    </div>
  );
};

export default GatewayStatisticsView;
