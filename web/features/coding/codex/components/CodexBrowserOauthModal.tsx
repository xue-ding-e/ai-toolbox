import React from 'react';
import { Alert, Button, Modal, Space, Typography, message } from 'antd';
import { CopyOutlined, LinkOutlined, ReloadOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import type { CodexBrowserOauthView } from '../hooks/useCodexBrowserOauth';

const { Text } = Typography;

interface CodexBrowserOauthModalProps {
  view: CodexBrowserOauthView | null;
  onClose: () => void;
  onRetry: (providerId: string) => void;
  onReopenBrowser: () => void;
}

const CodexBrowserOauthModal: React.FC<CodexBrowserOauthModalProps> = ({
  view,
  onClose,
  onRetry,
  onReopenBrowser,
}) => {
  const { t } = useTranslation();
  const [remainingSeconds, setRemainingSeconds] = React.useState(0);

  React.useEffect(() => {
    if (!view?.session) {
      setRemainingSeconds(0);
      return;
    }
    const updateRemaining = () => {
      setRemainingSeconds(Math.max(0, view.session!.expiresAt - Math.floor(Date.now() / 1000)));
    };
    updateRemaining();
    const timer = window.setInterval(updateRemaining, 1000);
    return () => window.clearInterval(timer);
  }, [view?.session]);

  const terminal = view?.phase === 'error' || view?.phase === 'expired';
  const statusText = view?.phase === 'starting' ? t('codex.provider.browserOauthStarting')
    : view?.phase === 'exchanging' ? t('codex.provider.browserOauthExchanging')
      : view?.phase === 'expired' ? t('codex.provider.browserOauthExpired')
        : view?.phase === 'error' ? t('codex.provider.browserOauthFailed')
          : t('codex.provider.browserOauthWaiting');

  return (
    <Modal
      open={Boolean(view)}
      title={t('codex.provider.browserOauthTitle')}
      onCancel={onClose}
      footer={(
        <Space>
          {terminal && view && (
            <Button icon={<ReloadOutlined />} onClick={() => onRetry(view.providerId)}>
              {t('codex.provider.browserOauthRetry')}
            </Button>
          )}
          <Button onClick={onClose}>{t('common.cancel')}</Button>
        </Space>
      )}
      width={520}
    >
      {view && (
        <Space direction="vertical" size={12} style={{ width: '100%' }}>
          <Text>{statusText}</Text>
          {view.session && !terminal && (
            <Text type="secondary">
              {t('codex.provider.browserOauthRemaining', { seconds: remainingSeconds })}
            </Text>
          )}
          {view.error && <Alert type={terminal ? 'error' : 'warning'} showIcon message={view.error} />}
          {view.session && !terminal && (
            <Space wrap>
              <Button icon={<LinkOutlined />} onClick={onReopenBrowser}>
                {t('codex.provider.browserOauthReopen')}
              </Button>
              <Button
                icon={<CopyOutlined />}
                onClick={() => void navigator.clipboard.writeText(view.session!.authorizeUrl)
                  .then(() => message.success(t('common.copied')))
                  .catch((error) => message.error(error instanceof Error ? error.message : String(error)))}
              >
                {t('codex.provider.browserOauthCopyUrl')}
              </Button>
            </Space>
          )}
        </Space>
      )}
    </Modal>
  );
};

export default CodexBrowserOauthModal;
