import React from 'react';
import { createRoot } from 'react-dom/client';
import { App, ConfigProvider, Form, Select, Typography, theme } from 'antd';
import i18next from 'i18next';
import { I18nextProvider } from 'react-i18next';
import OmpCodexSubscriptionSection from '@/features/coding/oh_my_pi/components/OmpCodexSubscriptionSection';
import OmpCodexModelInput from '@/features/coding/oh_my_pi/components/OmpCodexModelInput';
import styles from '@/features/coding/oh_my_pi/pages/OhMyPiPage.module.less';
import zh from '@/i18n/locales/zh-CN.json';
import '@/App.css';

const mode = new URLSearchParams(location.search).get('theme') || 'light';
const resolvedTheme = mode === 'system' ? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light') : mode;
document.documentElement.dataset.theme = resolvedTheme;
await i18next.init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: zh } } });
const state = { mode, commits: [], componentCommits: [], inputEvents: [], refreshes: 0, invocations: [] };
for (const type of ['focus', 'beforeinput', 'input', 'change', 'blur']) {
  document.addEventListener(type, event => {
    if (event.target.id === 'defaultModel') {
      state.inputEvents.push({ type, trusted: event.isTrusted, value: event.target.value });
    }
  }, true);
}
window.__TAURI_INTERNALS__ = { invoke: async (command, input) => { state.invocations.push({ command, input }); if (command !== 'copy_text_to_clipboard') throw Error('Unexpected command ' + command); } };
const longRoot = '/tmp/omp user/非常长的路径/' + 'project-'.repeat(28);
const subscription = { status: 'unknown', hasApiKeyOverride: true, shell: 'posix', wslDistro: 'Ubuntu-24.04', loginCommand: `PI_CODING_AGENT_DIR='${longRoot}' omp --profile default login openai-codex`, modelsCommand: `PI_CODING_AGENT_DIR='${longRoot}' omp --profile default models openai-codex --json --no-extensions` };
function Fixture() {
  const [current, setCurrent] = React.useState(subscription);
  const [options, setOptions] = React.useState([]);
  const [form] = Form.useForm();
  window.ompFixture = { state, setCurrent, setOptions, setModel: value => form.setFieldValue('defaultModel', value) };
  return <main style={{ padding: 16, margin: '0 auto', maxWidth: 1000 }}>
    <div className={styles.modelCard}>
      <Typography.Title level={5} className={styles.modelCardTitle}>模型设置</Typography.Title>
      <div className={styles.modelCardContent}>
        <Form form={form} layout="vertical" initialValues={{ defaultProvider: 'openai-codex' }} onValuesChange={(changed, values) => state.commits.push({ changed, values })}>
          <div className={styles.modelSettingsGrid}>
            <Form.Item label="默认供应商" name="defaultProvider"><Select options={[{ value: 'openai-codex', label: 'OpenAI Codex 订阅 (openai-codex)' }]} /></Form.Item>
            <Form.Item label="默认模型" name="defaultModel">
              <OmpCodexModelInput options={options} placeholder={i18next.t('ohMyPi.codexSubscription.modelPlaceholder')} aria-describedby="omp-codex-model-hint" onChange={value => state.componentCommits.push(value)} />
            </Form.Item>
          </div>
        </Form>
        <p id="omp-codex-model-hint" className={styles.modelSettingsHint}>{i18next.t('ohMyPi.codexSubscription.modelHint')}</p>
        <OmpCodexSubscriptionSection subscription={current} onRefresh={async () => { state.refreshes++; await new Promise(resolve => setTimeout(resolve, 150)); setCurrent(previous => ({ ...previous, status: 'configured' })); }} />
      </div>
    </div>
  </main>;
}
createRoot(document.getElementById('root')).render(<ConfigProvider theme={{ algorithm: resolvedTheme === 'dark' ? theme.darkAlgorithm : theme.defaultAlgorithm, token: { colorPrimary: '#0958d9' } }}><App><I18nextProvider i18n={i18next}><Fixture /></I18nextProvider></App></ConfigProvider>);
