import type { FC } from 'react';
import { Empty, Modal, Tabs, Typography } from 'antd';
import type { TabsProps } from 'antd';
import { useTranslation } from 'react-i18next';
import PlainTextEditor from '@/components/common/PlainTextEditor';
import type { DshRuntimeConfig } from '@/types/dsh';

const { Text } = Typography;

export interface DshConfigPreviewModalProps {
  open: boolean;
  onClose: () => void;
  title?: string;
  data: DshRuntimeConfig | null;
}

const DshConfigPreviewModal: FC<DshConfigPreviewModalProps> = ({
  open,
  onClose,
  title,
  data,
}) => {
  const { t } = useTranslation();

  const configValue = data?.configContent ?? null;
  const credentialsValue = data?.credentialsContent ?? null;
  const promptValue = data?.promptContent ?? null;
  const cordisPatchValue = data?.cordisPatchContent ?? null;
  const editorHeight = 'calc(75vh - 190px)';

  // `configContent` is whatever layer is live: the profile patch since dsh
  // 0.1.7, legacy settings.yaml before that. Label it truthfully.
  const configLabel = data?.profile
    ? t('dsh.preview.profilePatchTitle', {
        defaultValue: 'Profile Patch ({{profile}})',
        profile: data.profile,
      })
    : t('dsh.preview.settingsYamlTitle', { defaultValue: 'settings.yaml' });

  const items: TabsProps['items'] = [];

  if (configValue !== null) {
    items.push({
      key: 'config',
      label: configLabel,
      children: (
        <div style={{ padding: '4px 0' }}>
          <PlainTextEditor value={configValue} readOnly language="yaml" height={editorHeight} />
        </div>
      ),
    });
  }

  if (credentialsValue !== null) {
    items.push({
      key: 'credentials',
      label: t('dsh.preview.credentialsYamlTitle', { defaultValue: '.credentials.yaml' }),
      children: (
        <div style={{ padding: '4px 0' }}>
          <PlainTextEditor value={credentialsValue} readOnly language="yaml" height={editorHeight} />
        </div>
      ),
    });
  }

  if (promptValue !== null) {
    items.push({
      key: 'prompt',
      label: t('dsh.preview.agentsMdTitle', { defaultValue: 'AGENTS.md' }),
      children: (
        <div style={{ padding: '4px 0' }}>
          <PlainTextEditor value={promptValue} readOnly language="markdown" height={editorHeight} />
        </div>
      ),
    });
  }

  if (cordisPatchValue !== null) {
    items.push({
      key: 'cordis-patch',
      // Home-level patch: MCP rows only. Never confuse it with the profile patch
      // tab above, which is the live provider/default-model layer.
      label: t('dsh.preview.cordisPatchTitle', { defaultValue: 'cordis.patch.yml' }),
      children: (
        <div style={{ padding: '4px 0' }}>
          <PlainTextEditor value={cordisPatchValue} readOnly language="yaml" height={editorHeight} />
        </div>
      ),
    });
  }

  const hasAny = items.length > 0;

  return (
    <Modal
      title={
        <span>
          {title || t('common.previewConfig')}{' '}
          <Text type="secondary" style={{ fontSize: 12, fontWeight: 'normal' }}>
            ({t('common.readOnly')})
          </Text>
        </span>
      }
      open={open}
      onCancel={onClose}
      footer={null}
      width={1000}
    >
      {!hasAny ? (
        <Empty description={t('common.noData')} />
      ) : (
        <Tabs
          items={items}
          defaultActiveKey={items[0]?.key}
          destroyOnHidden
        />
      )}
    </Modal>
  );
};

export default DshConfigPreviewModal;
