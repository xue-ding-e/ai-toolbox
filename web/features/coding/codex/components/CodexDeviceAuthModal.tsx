import React from 'react';
import { Alert, Button, Modal, Space, Typography, message } from 'antd';
import { CopyOutlined, LinkOutlined } from '@ant-design/icons';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';
import type { CodexDeviceAuthView } from '../hooks/useCodexDeviceAuth';

const { Text } = Typography;

interface CodexDeviceAuthModalProps {
  view: CodexDeviceAuthView | null;
  onClose: () => void;
}

const CodexDeviceAuthModal: React.FC<CodexDeviceAuthModalProps> = ({
  view,
  onClose,
}) => {
  const { t } = useTranslation();
  const authSession = view?.session;
  const [remainingSeconds, setRemainingSeconds] = React.useState(0);

  React.useEffect(() => {
    if (!authSession) {
      setRemainingSeconds(0);
      return;
    }
    const updateRemaining = () => {
      setRemainingSeconds(Math.max(0, authSession.expiresAt - Math.floor(Date.now() / 1000)));
    };
    updateRemaining();
    const timer = window.setInterval(updateRemaining, 1000);
    return () => window.clearInterval(timer);
  }, [authSession]);

  return (
    <Modal
      open={Boolean(view)}
      title={t('codex.provider.deviceAuthTitle')}
      onCancel={onClose}
      footer={<Button onClick={onClose}>{t('common.cancel')}</Button>}
      width={560}
    >
      {view && (
        <Space direction="vertical" size={16} style={{ width: '100%' }}>
          <Text>{authSession ? t('codex.provider.deviceAuthHint')
            : view.status === 'error' ? t('codex.provider.deviceAuthFailed')
              : t('codex.provider.deviceAuthStarting')}</Text>
          {view.error && <Alert type="warning" showIcon message={view.error} />}
          {authSession && (
            <>
              <Space.Compact block>
            <Button
              icon={<LinkOutlined />}
              onClick={() => void openUrl(authSession.verificationUri)}
            >
              {t('codex.provider.deviceAuthOpenBrowser')}
            </Button>
            <Button
              icon={<CopyOutlined />}
              onClick={() => void navigator.clipboard.writeText(authSession.userCode).then(
                () => message.success(t('common.copied')),
              )}
            >
              {t('common.copy')}
            </Button>
              </Space.Compact>
              <Typography.Title level={3} copyable style={{ margin: 0, textAlign: 'center' }}>
                {authSession.userCode}
              </Typography.Title>
              <Text type="secondary">
                {t('codex.provider.deviceAuthStatus', {
                  status: view.status === 'exchanging'
                    ? t('codex.provider.deviceAuthExchanging') : view.status,
                  seconds: remainingSeconds,
                })}
              </Text>
            </>
          )}
        </Space>
      )}
    </Modal>
  );
};

export default CodexDeviceAuthModal;
