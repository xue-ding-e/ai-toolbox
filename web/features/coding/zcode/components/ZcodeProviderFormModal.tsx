import React from 'react';
import { Button, Form, Input, Modal, Select, Typography, message } from 'antd';
import { EyeInvisibleOutlined, EyeOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import {
  createZcodeProvider,
  listZcodeProviderTemplates,
  saveZcodeProvider,
  updateZcodeProvider,
} from '@/services/zcodeApi';
import ProviderFormSections from '@/features/coding/shared/providerConfig/ProviderFormSections';
import type { ZcodeProvider, ZcodeProviderTemplate, ZcodeSettingsConfig } from '@/types/zcode';
import { ZCODE_API_TYPES } from '@/types/zcode';
import {
  buildZcodeProviderId,
  parseZcodeProviderSettings,
} from '../utils/zcodeSettingsConfig';
import styles from './ZcodeProviderFormModal.module.less';

const { Text } = Typography;

interface ZcodeProviderFormModalProps {
  open: boolean;
  provider: ZcodeProvider | null;
  /**
   * 表单预填 `provider` 但保存为**新**记录（复制）。provider id 与名称各加
   * `_copy` 后缀；id 可编辑，让用户改成自己要的。
   */
  isCopy?: boolean;
  onCancel: () => void;
  onSaved: () => Promise<void> | void;
}

interface ZcodeProviderFormValues {
  name: string;
  providerId: string;
  templateId?: string;
  apiType: string;
  baseUrl: string;
  apiKey?: string;
  notes?: string;
}

/**
 * ZCode resolves a provider's request shape from `templateId` when set, so a
 * template-driven provider only needs the credentials that differ from it.
 */
const ZcodeProviderFormModal: React.FC<ZcodeProviderFormModalProps> = ({
  open,
  provider,
  isCopy = false,
  onCancel,
  onSaved,
}) => {
  const { t, i18n } = useTranslation();
  const [form] = Form.useForm<ZcodeProviderFormValues>();
  // Same label/wrapper split as every other provider form (kimi / openclaw /
  // codex): labels sit in a left column rather than above the field.
  const labelCol = { span: i18n.language === 'zh-CN' ? 4 : 6 };
  const wrapperCol = { span: 20 };
  const [templates, setTemplates] = React.useState<ZcodeProviderTemplate[]>([]);
  const [showApiKey, setShowApiKey] = React.useState(false);
  const [saving, setSaving] = React.useState(false);
  // 复制走的是「新增」路径（新 id），只是预填了来源 provider 的值。
  const isEditing = Boolean(provider) && !isCopy;

  React.useEffect(() => {
    if (!open) {
      return;
    }
    void listZcodeProviderTemplates()
      .then(setTemplates)
      .catch((error) => console.error('Failed to load ZCode templates:', error));
  }, [open]);

  React.useEffect(() => {
    if (!open) {
      return;
    }
    if (provider) {
      const settings = parseZcodeProviderSettings(provider.settingsConfig);
      const sourceProviderId = settings?.providerId ?? provider.id;
      form.setFieldsValue({
        name: isCopy ? `${provider.name}_copy` : provider.name,
        providerId: isCopy ? `${sourceProviderId}_copy` : sourceProviderId,
        // The "custom" option is the empty string; a provider without a stored
        // template must land on it rather than showing a blank select.
        templateId: settings?.templateId ?? '',
        apiType: settings?.config?.api?.type ?? 'anthropic-messages',
        baseUrl: settings?.config?.api?.baseUrl ?? '',
        apiKey: settings?.config?.access?.apiKey ?? '',
        notes: provider.notes ?? '',
      });
    } else {
      form.resetFields();
      form.setFieldsValue({ apiType: 'anthropic-messages', templateId: '' });
    }
  }, [open, provider, form]);

  // Selecting a channel seeds the format and base URL from it; picking
  // "custom" leaves whatever the user has typed alone.
  const handleTemplateChange = (templateId: string | undefined) => {
    const template = templates.find((item) => item.templateId === templateId);
    if (!template) {
      return;
    }
    form.setFieldsValue({
      apiType: template.apiType ?? form.getFieldValue('apiType'),
      baseUrl: template.baseUrl ?? form.getFieldValue('baseUrl'),
    });
  };

  const handleSubmit = async () => {
    let values: ZcodeProviderFormValues;
    try {
      values = await form.validateFields();
    } catch {
      return;
    }
    // Models are edited on the provider card, not here. Reuse whatever the
    // stored provider already has so saving the form does not wipe the catalog.
    const existingModels = provider
      ? (parseZcodeProviderSettings(provider.settingsConfig)?.models ?? [])
      : [];

    const providerId = values.providerId?.trim() || buildZcodeProviderId(values.name);
    if (providerId.startsWith('builtin:') || providerId.startsWith('account:')) {
      void message.error(
        t('zcode.form.reservedId'),
      );
      return;
    }

    const settings: ZcodeSettingsConfig = {
      providerId,
      providerName: values.name,
      templateId: values.templateId || undefined,
      config: {
        api: {
          type: values.apiType,
          baseUrl: values.baseUrl,
        },
        access: values.apiKey
          ? { type: 'api-key', apiKey: values.apiKey }
          : undefined,
      },
      models: existingModels,
      defaultModelId: existingModels.find((model) => model.isDefault)?.modelId,
    };

    setSaving(true);
    try {
      // No `meta` builder here: the billing / header / rewrite subsets are all
      // gateway-only and ZCode is not a gateway CLI, so their sections are
      // hidden and nothing writes those keys. `meta` is carried through
      // unchanged to preserve anything a future version adds.
      const payload = {
        name: values.name,
        category: 'custom',
        settingsConfig: JSON.stringify(settings),
        notes: values.notes ?? null,
        meta: provider?.meta ?? undefined,
      };
      // The DB row alone does not reach ZCode; every save must also project the
      // provider into the registry so the desktop app can see the new values.
      // Copy mode carries the *source* provider in `provider`, so branch on the
      // edit flag: a copy must create a new row, never overwrite its source.
      if (isEditing && provider) {
        const updated = await updateZcodeProvider({
          ...provider,
          name: payload.name,
          settingsConfig: payload.settingsConfig,
          notes: payload.notes,
          meta: payload.meta,
        });
        await saveZcodeProvider({
          id: updated.id,
          name: updated.name,
          category: updated.category,
          settingsConfig: updated.settingsConfig,
          notes: updated.notes,
        });
      } else {
        const created = await createZcodeProvider({
          ...payload,
          id: providerId,
        });
        await saveZcodeProvider({
          id: created.id,
          name: created.name,
          category: created.category,
          settingsConfig: created.settingsConfig,
          notes: created.notes,
        });
      }
      await onSaved();
    } catch (error) {
      console.error('Failed to save ZCode provider:', error);
      void message.error(String(error));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      open={open}
      title={
        isEditing
          ? t('zcode.form.editTitle')
          : t('zcode.form.addTitle')
      }
      width={720}
      onCancel={onCancel}
      onOk={() => void handleSubmit()}
      confirmLoading={saving}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
      destroyOnHidden
    >
      <Form form={form} layout="horizontal" labelCol={labelCol} wrapperCol={wrapperCol}>
        {/* Channel row: built-in channel on the left, API format on the right —
            the same two-column shape the Codex form uses. */}
        <Form.Item
          label={t('common.provider.providerChannel')}
          required
          extra={
            <Text type="secondary" style={{ fontSize: 12 }}>
              {t('common.provider.providerChannelHint')}
            </Text>
          }
        >
          <div className={styles.providerChannelRow}>
            <Form.Item name="templateId" noStyle>
              <Select
                onChange={handleTemplateChange}
                options={[
                  // Explicit "custom" entry, as on the Codex form: leaving the
                  // channel unset is a real choice, not an empty state, so it
                  // gets a named option instead of relying on a placeholder.
                  { value: '', label: t('common.provider.providerChannelCustom') },
                  ...templates.map((template) => ({
                    value: template.templateId,
                    label: template.name,
                  })),
                ]}
              />
            </Form.Item>
            <Form.Item
              name="apiType"
              noStyle
              rules={[{ required: true, message: t('common.error') }]}
            >
              <Select options={ZCODE_API_TYPES.map((item) => ({ ...item }))} />
            </Form.Item>
          </div>
        </Form.Item>
        <Form.Item
          name="name"
          label={t('zcode.form.name')}
          rules={[{ required: true, message: t('common.required') }]}
        >
          <Input placeholder="DeepSeek" />
        </Form.Item>
        <Form.Item
          name="providerId"
          label="Provider ID"
          extra={
            <Text type="secondary" style={{ fontSize: 11 }}>
              {t('zcode.form.providerIdHint')}
            </Text>
          }
        >
          <Input placeholder="custom:deepseek" disabled={isEditing} />
        </Form.Item>
        <Form.Item
          name="baseUrl"
          label="Base URL"
          rules={[{ required: true, message: t('common.required') }]}
        >
          <Input placeholder="https://api.deepseek.com/anthropic" />
        </Form.Item>
        <Form.Item name="apiKey" label="API Key">
          <Input
            type={showApiKey ? 'text' : 'password'}
            placeholder="sk-..."
            addonAfter={
              <Button
                type="text"
                size="small"
                icon={showApiKey ? <EyeInvisibleOutlined /> : <EyeOutlined />}
                onClick={() => setShowApiKey(!showApiKey)}
              >
                {showApiKey ? t('common.provider.hideApiKey') : t('common.provider.showApiKey')}
              </Button>
            }
          />
        </Form.Item>
        {/* Billing, custom headers and model rewrites are all consumed by the
            local gateway, which cannot take ZCode over — showing them would
            collect values nothing reads. Only notes apply here. */}
        <ProviderFormSections
          editable
          showBilling={false}
          showCustomHeaders={false}
          showModelRewrites={false}
          notesRows={2}
          notesResetKey={`zcode-provider-notes-${provider?.id ?? 'new'}`}
        />
      </Form>
    </Modal>
  );
};

export default ZcodeProviderFormModal;
