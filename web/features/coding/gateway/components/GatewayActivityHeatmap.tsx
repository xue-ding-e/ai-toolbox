import React from 'react';
import { useTranslation } from 'react-i18next';
import type { GatewayUsageTrendPoint } from '@/services';
import {
  buildGatewayHeatmapDays,
  buildGatewayHeatmapLayout,
  gatewayHeatLevels,
  gatewayHeatLevelText,
  type GatewayHeatmapLayout,
} from '../utils/gatewayHeatmap';
import { formatCompactInteger, formatInteger } from '../utils/gatewayFormatters';
import styles from './GatewayActivityHeatmap.module.less';

interface GatewayActivityHeatmapProps {
  trends: GatewayUsageTrendPoint[];
  loading: boolean;
}

const HEATMAP_LEVELS = [0, 1, 2, 3, 4] as const;
const heatLevelClass = (level: number) => styles[`level${level}` as keyof typeof styles];
const heatSwatchClass = (level: number) => styles[`swatch${level}` as keyof typeof styles];

const GatewayActivityHeatmap: React.FC<GatewayActivityHeatmapProps> = ({ trends, loading }) => {
  const { t, i18n } = useTranslation();
  const containerRef = React.useRef<HTMLDivElement | null>(null);
  const [containerWidth, setContainerWidth] = React.useState(0);

  const days = React.useMemo(() => buildGatewayHeatmapDays(trends), [trends]);
  const thresholds = React.useMemo(
    () => gatewayHeatLevels(days.reduce((peak, day) => Math.max(peak, day.tokens), 0)),
    [days],
  );
  const layout: GatewayHeatmapLayout = React.useMemo(
    () => buildGatewayHeatmapLayout(days, containerWidth),
    [containerWidth, days],
  );

  React.useEffect(() => {
    const node = containerRef.current;
    if (!node || typeof ResizeObserver === 'undefined') {
      return undefined;
    }
    // `clientWidth` includes the body's horizontal padding; the SVG must fit
    // the content box or a scrollbar appears even when the grid is complete.
    const measure = () => {
      const style = window.getComputedStyle(node);
      const horizontalPadding =
        Number.parseFloat(style.paddingLeft || '0') + Number.parseFloat(style.paddingRight || '0');
      setContainerWidth(Math.max(0, Math.floor(node.clientWidth - horizontalPadding)));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  const formatTokens = (value: number) => formatCompactInteger(value, i18n.language) ?? '-';
  const dayLabel = (date: string) =>
    new Date(`${date}T00:00:00`).toLocaleDateString(i18n.language);
  const monthLabel = (month: number) =>
    new Date(2000, month, 1).toLocaleDateString(i18n.language, { month: 'short' });

  // The calendar is always dense (365 days), so a window without usage simply
  // renders every cell at level 0. Only the first load shows a placeholder;
  // every later refresh keeps the previous calendar until new data arrives.
  const ready = containerWidth > 0 && layout.cells.length > 0;
  const showPlaceholder = !ready || (loading && trends.length === 0);

  return (
    <section className={styles.heatmapPanel}>
      <div className={styles.panelHeader}>
        <span>{t('gateway.page.statistics.heatmap.title')}</span>
        <span className={styles.panelSubtitle}>{t('gateway.page.statistics.heatmap.subtitle')}</span>
      </div>

      <div className={styles.heatmapBody} ref={containerRef}>
        {!showPlaceholder ? (
          <div className={styles.heatmapScroller}>
            <svg
              className={styles.heatmapChart}
              width={layout.width}
              height={layout.height}
              viewBox={`0 0 ${layout.width} ${layout.height}`}
              role="img"
              aria-label={t('gateway.page.statistics.heatmap.title')}
            >
              {layout.months.map((item) => (
                <text className={styles.axisText} x={item.x} y={10} key={item.key}>
                  {monthLabel(item.month)}
                </text>
              ))}
              {layout.weekdays.map((item) => (
                <text
                  className={styles.axisText}
                  x={item.x}
                  y={item.y}
                  textAnchor="end"
                  dominantBaseline="middle"
                  key={item.key}
                >
                  {t(`gateway.page.statistics.heatmap.weekdays.${item.row}`)}
                </text>
              ))}
              {layout.cells.map((cell) => (
                <rect
                  key={cell.date}
                  className={heatLevelClass(cell.level)}
                  x={cell.x}
                  y={cell.y}
                  width={cell.size}
                  height={cell.size}
                  rx={2}
                  tabIndex={-1}
                >
                  <title>
                    {cell.requests > 0
                      ? t('gateway.page.statistics.heatmap.cellTooltip', {
                          date: dayLabel(cell.date),
                          tokens: formatInteger(cell.tokens),
                          requests: formatInteger(cell.requests),
                        })
                      : t('gateway.page.statistics.heatmap.cellEmptyTooltip', { date: dayLabel(cell.date) })}
                  </title>
                </rect>
              ))}
            </svg>
          </div>
        ) : (
          <div className={styles.heatmapPlaceholder}>
            {loading ? t('common.loading') : t('gateway.page.statistics.heatmap.empty')}
          </div>
        )}
      </div>

      <div className={styles.heatmapFooter}>
        <span className={styles.legendLabel}>{t('gateway.page.statistics.heatmap.legendLabel')}</span>
        <div className={styles.legendItems}>
          {HEATMAP_LEVELS.map((level) => (
            <span className={styles.legendItem} key={level}>
              <span className={`${styles.legendSwatch} ${heatSwatchClass(level)}`} />
              {gatewayHeatLevelText(level, thresholds, formatTokens)}
            </span>
          ))}
        </div>
        <span className={styles.footerNote}>{t('gateway.page.statistics.heatmap.footerNote')}</span>
      </div>
    </section>
  );
};

export default GatewayActivityHeatmap;
