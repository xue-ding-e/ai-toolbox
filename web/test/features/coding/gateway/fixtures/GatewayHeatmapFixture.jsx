import { createRoot } from 'react-dom/client';
import { App, ConfigProvider, theme } from 'antd';
import i18n from '@/i18n';
import GatewayActivityHeatmap from '@/features/coding/gateway/components/GatewayActivityHeatmap';
import '@/App.css';

const parameters = new URLSearchParams(location.search);
const mode = parameters.get('theme') || 'light';
const resolvedTheme = mode === 'system' ? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light') : mode;
document.documentElement.dataset.theme = resolvedTheme;
await i18n.changeLanguage(parameters.get('language') || 'zh-CN');

const today = new Date();
const dayKey = (offset) => {
  const date = new Date(today.getFullYear(), today.getMonth(), today.getDate() - offset);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
};

const trends = [
  { date: dayKey(0), request_count: 12, total_cost_usd: '0', total_tokens: 400, input_tokens: 400, output_tokens: 0, cache_read_tokens: 0, cache_creation_tokens: 0 },
  { date: dayKey(3), request_count: 4, total_cost_usd: '0', total_tokens: 200, input_tokens: 200, output_tokens: 0, cache_read_tokens: 0, cache_creation_tokens: 0 },
  { date: dayKey(40), request_count: 1, total_cost_usd: '0', total_tokens: 0, input_tokens: 0, output_tokens: 0, cache_read_tokens: 0, cache_creation_tokens: 0 },
  { date: dayKey(100), request_count: 2, total_cost_usd: '0', total_tokens: 50, input_tokens: 50, output_tokens: 0, cache_read_tokens: 0, cache_creation_tokens: 0 },
];

const root = createRoot(document.getElementById('root'));
root.render(
  <ConfigProvider
    theme={{ algorithm: resolvedTheme === 'dark' ? theme.darkAlgorithm : theme.defaultAlgorithm, token: { colorPrimary: '#1890ff' } }}
  >
    <App>
      <div style={{ width: 1100, padding: 24 }}>
        <GatewayActivityHeatmap trends={trends} loading={false} />
      </div>
    </App>
  </ConfigProvider>,
);

window.heatmapFixture = { trends, dayKey };
