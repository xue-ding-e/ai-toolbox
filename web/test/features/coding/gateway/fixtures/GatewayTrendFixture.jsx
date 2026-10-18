import React from 'react';
import { createRoot } from 'react-dom/client';
import { App, ConfigProvider, theme } from 'antd';
import i18n from '@/i18n';
import GatewayUsageTrendChart from '@/features/coding/gateway/components/GatewayUsageTrendChart';
import '@/App.css';

const parameters = new URLSearchParams(location.search);
const mode = parameters.get('theme') || 'light';
const resolvedTheme = mode === 'system' ? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light') : mode;
document.documentElement.dataset.theme = resolvedTheme;
await i18n.changeLanguage(parameters.get('language') || 'zh-CN');

const today = new Date();
const hourKey = (offset) => {
  const date = new Date(today.getFullYear(), today.getMonth(), today.getDate(), offset, 0, 0);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}T${String(date.getHours()).padStart(2, '0')}:00:00`;
};

const trends = [6, 7, 8, 9].map((hour, index) => ({
  date: hourKey(hour),
  request_count: [3, 12, 7, 9][index],
  total_cost_usd: ['0.21', '0.83', '0.44', '0.52'][index],
  total_tokens: [640000, 2103100, 1050000, 1240000][index],
  input_tokens: 0,
  output_tokens: 0,
  cache_read_tokens: 0,
  cache_creation_tokens: 0,
}));

// Midnight through 14:00 so the fixture reproduces the reference layout: the
// morning and mid-day hours without traffic must still occupy their slots.
const dayStart = new Date(today.getFullYear(), today.getMonth(), today.getDate()).getTime();
const dayHour = hour => new Date(today.getFullYear(), today.getMonth(), today.getDate(), hour);
const startDate = Math.floor(dayStart / 1000);
const endDate = Math.floor(dayHour(14).getTime() / 1000);

const TrendHost = () => {
  const [metric, setMetric] = React.useState('tokens');
  return (
    <div style={{ width: 1100 }}>
      <GatewayUsageTrendChart
        trends={trends}
        startDate={startDate}
        endDate={endDate}
        loading={false}
        metric={metric}
        onMetricChange={setMetric}
        rangeLabel="今天"
      />
    </div>
  );
};

const root = createRoot(document.getElementById('root'));
root.render(
  <ConfigProvider
    theme={{ algorithm: resolvedTheme === 'dark' ? theme.darkAlgorithm : theme.defaultAlgorithm, token: { colorPrimary: '#1890ff' } }}
  >
    <App>
      <div style={{ padding: 24 }}>
        <TrendHost />
      </div>
    </App>
  </ConfigProvider>,
);

window.trendFixture = { trends, startDate, endDate };
