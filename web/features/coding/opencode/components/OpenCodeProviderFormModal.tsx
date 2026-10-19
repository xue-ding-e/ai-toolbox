import React from 'react';
import {
  App,
  Button,
  Checkbox,
  Divider,
  Form,
  Input,
  InputNumber,
  Modal,
  Radio,
  Select,
  Space,
  Switch,
  Typography,
} from 'antd';
import { DownOutlined, RightOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import HeadersEditor from '@/components/common/HeadersEditor';
import JsonEditor from '@/components/common/JsonEditor';
import { PROVIDER_TYPES } from '@/constants/providerTypes';
import ProviderFormSections from '@/features/coding/shared/providerConfig/ProviderFormSections';

const { Text } = Typography;

/**
 * Form values for the OpenCode provider editor.
 *
 * These are the fields the editor owns; the page maps them onto
 * `OpenCodeProvider` (`npm` / `options` / `whitelist` / `blacklist`). Keeping
 * the shape here means the page no longer imports it from the legacy shared
 * modal.
 */
export interface OpenCodeProviderFormValues {
  id: string;
  name: string;
  sdkType: string;
  baseUrl: string;
  apiKey?: string;
  headers?: string | Record<string, string>;
  timeout?: number;
  disableTimeout?: boolean;
  setCacheKey?: boolean;
  extraOptions?: Record<string, unknown>;
  filterMode?: 'blacklist' | 'whitelist';
  filterModels?: string[];
}

interface OpenCodeProviderFormModalProps {
  open: boolean;
  isEdit?: boolean;
  initialValues?: Partial<OpenCodeProviderFormValues>;
  /** Existing provider ids for the duplicate check (only when adding). */
  existingIds?: string[];
  onCancel: () => void;
  onSuccess: (values: OpenCodeProviderFormValues) => void;
  onDuplicateId?: (id: string) => void;
  /** Models offered by the whitelist / blacklist selector. */
  modelOptions?: { label: string; value: string }[];
}

const BASE_URL_PLACEHOLDERS: Record<string, string> = {
  '@ai-sdk/openai-compatible': 'https://api.example.com/v1',
  '@ai-sdk/openai': 'https://api.openai.com/v1',
  '@ai-sdk/anthropic': 'https://api.anthropic.com/v1',
  '@ai-sdk/google': 'https://generativelanguage.googleapis.com/v1beta',
};

const PROVIDERS_NEED_V1 = ['@ai-sdk/anthropic', '@ai-sdk/openai-compatible'];
const PROVIDERS_NEED_V1_OR_V1BETA = ['@ai-sdk/google'];

const hasContent = (value: unknown): boolean => (
  typeof value === 'object' && value !== null && Object.keys(value).length > 0
);

/**
 * The provider editor, following the shared form shape (horizontal labels,
 * top fields, then `ProviderFormSections`).
 *
 * Two OpenCode-specific facts drive the layout:
 *
 * - Everything that is not the top connection fields lives in the advanced
 *   block: request headers, timeout, cache key, extra options and the model
 *   whitelist/blacklist. They are OpenCode config keys, not generic provider
 *   metadata, so they are edited here and written into `provider.options` /
 *   `provider.whitelist` / `provider.blacklist`.
 * - The gateway-owned sections (billing / custom headers / model rewrites /
 *   service tiers) are deliberately absent: OpenCode is not in
 *   `GatewayCliKey::supported_mvp()`, so those values would have no reader
 *   (SOP §4.0.3). Notes are hidden for the same reason — an OpenCode provider
 *   record has no notes field.
 */
const OpenCodeProviderFormModal: React.FC<OpenCodeProviderFormModalProps> = ({
  open,
  isEdit = false,
  initialValues,
  existingIds = [],
  onCancel,
  onSuccess,
  onDuplicateId,
  modelOptions = [],
}) => {
  const { t, i18n } = useTranslation();
  const { message } = App.useApp();
  const [form] = Form.useForm<OpenCodeProviderFormValues>();
  const [loading, setLoading] = React.useState(false);
  const [showApiKey, setShowApiKey] = React.useState(false);
  const [headersValid, setHeadersValid] = React.useState(true);
  const [extraOptionsValid, setExtraOptionsValid] = React.useState(true);
  const [advancedExpanded, setAdvancedExpanded] = React.useState(false);

  const labelCol = { span: i18n.language === 'zh-CN' ? 4 : 6 };
  const wrapperCol = { span: 20 };
  const getKey = (key: string) => `opencode.provider.${key}`;

  React.useEffect(() => {
    if (!open) {
      return;
    }
    if (initialValues) {
      form.setFieldsValue(initialValues);
      setAdvancedExpanded(
        hasContent(initialValues.headers)
        || initialValues.disableTimeout === true
        || initialValues.timeout !== undefined
        || initialValues.setCacheKey === true
        || hasContent(initialValues.extraOptions)
        || (initialValues.filterModels?.length ?? 0) > 0,
      );
    } else {
      form.resetFields();
      setAdvancedExpanded(false);
    }
    setShowApiKey(false);
    setHeadersValid(true);
    setExtraOptionsValid(true);
  }, [open, initialValues, form]);

  const submit = (values: OpenCodeProviderFormValues) => {
    onSuccess(values);
    form.resetFields();
    setLoading(false);
  };

  const handleSubmit = async () => {
    let values: OpenCodeProviderFormValues;
    try {
      values = await form.validateFields();
    } catch {
      // antd renders the field errors itself.
      return;
    }

    if (!headersValid) {
      message.error(t('settings.provider.invalidHeaders'));
      return;
    }
    if (!extraOptionsValid) {
      message.error(t('opencode.provider.invalidExtraOptions'));
      return;
    }

    if (!isEdit && existingIds.includes(values.id)) {
      onDuplicateId?.(values.id);
      return;
    }

    if (values.baseUrl?.endsWith('/')) {
      values.baseUrl = values.baseUrl.slice(0, -1);
    }

    // OpenCode resolves the endpoint from the SDK family; most families only
    // work with a `/v1` (or `/v1beta`) suffix, so a missing one is confirmed
    // rather than rejected — custom gateways do sometimes serve another path.
    let confirmMessageKey = '';
    if (values.baseUrl) {
      if (PROVIDERS_NEED_V1.includes(values.sdkType) && !values.baseUrl.endsWith('/v1')) {
        confirmMessageKey = 'opencode.provider.baseUrlConfirmV1';
      } else if (
        PROVIDERS_NEED_V1_OR_V1BETA.includes(values.sdkType)
        && !values.baseUrl.endsWith('/v1')
        && !values.baseUrl.endsWith('/v1beta')
      ) {
        confirmMessageKey = 'opencode.provider.baseUrlConfirmV1Beta';
      }
    }

    if (confirmMessageKey) {
      Modal.confirm({
        title: t('common.confirm'),
        content: t(confirmMessageKey),
        okText: t('common.confirm'),
        cancelText: t('common.cancel'),
        onOk: () => submit(values),
      });
      return;
    }

    setLoading(true);
    try {
      submit(values);
    } finally {
      setLoading(false);
    }
  };

  return (
    <Modal
      title={isEdit ? t('opencode.editProvider') : t('opencode.addProvider')}
      open={open}
      onCancel={onCancel}
      footer={[
        <Button key="cancel" onClick={onCancel}>
          {t('common.cancel')}
        </Button>,
        <Button key="submit" type="primary" loading={loading} onClick={() => void handleSubmit()}>
          {t('common.save')}
        </Button>,
      ]}
      width="min(960px, 92vw)"
    >
      <Form
        form={form}
        layout="horizontal"
        labelCol={labelCol}
        wrapperCol={wrapperCol}
        style={{ marginTop: 24 }}
      >
        <Form.Item
          label={t(getKey('id'))}
          name="id"
          rules={[{ required: true, message: t(getKey('idPlaceholder')) }]}
        >
          <Input placeholder={t(getKey('idPlaceholder'))} disabled={isEdit} />
        </Form.Item>

        <Form.Item
          label={t(getKey('name'))}
          name="name"
          rules={[{ required: true, message: t(getKey('namePlaceholder')) }]}
        >
          <Input placeholder={t(getKey('namePlaceholder'))} />
        </Form.Item>

        <Form.Item
          label={t(getKey('npm'))}
          name="sdkType"
          rules={[{ required: true }]}
          initialValue="@ai-sdk/openai-compatible"
        >
          <Select
            placeholder={t(getKey('npmPlaceholder'))}
            showSearch
            optionFilterProp="label"
            optionRender={(option) => (
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <span>{option.label}</span>
                <Text type="secondary" style={{ fontSize: 12 }}>{option.value}</Text>
              </div>
            )}
            options={PROVIDER_TYPES}
          />
        </Form.Item>

        <Form.Item noStyle shouldUpdate={(prev, curr) => prev.sdkType !== curr.sdkType}>
          {({ getFieldValue }) => {
            const sdkType = getFieldValue('sdkType') as string | undefined;
            const example = sdkType ? BASE_URL_PLACEHOLDERS[sdkType] : undefined;
            return (
              <Form.Item
                label={t(getKey('baseURL'))}
                name="baseUrl"
                extra={(
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    {t(getKey('baseUrlHint'))}
                  </Text>
                )}
              >
                <Input
                  placeholder={example
                    ? `${t(getKey('baseURLPlaceholder'))}, ${t(getKey('baseURLExample'))} ${example}`
                    : t(getKey('baseURLPlaceholder'))}
                />
              </Form.Item>
            );
          }}
        </Form.Item>

        <Form.Item label={t(getKey('apiKey'))} name="apiKey">
          <Input
            type={showApiKey ? 'text' : 'password'}
            placeholder={t(getKey('apiKeyPlaceholder'))}
            suffix={(
              <Button
                type="text"
                size="small"
                icon={(
                  <span aria-hidden>
                    {showApiKey
                      ? <span style={{ fontSize: 12 }}>{t('common.provider.hideApiKey')}</span>
                      : <span style={{ fontSize: 12 }}>{t('common.provider.showApiKey')}</span>}
                  </span>
                )}
                onClick={() => setShowApiKey((value) => !value)}
                style={{ marginRight: -8 }}
              />
            )}
          />
        </Form.Item>

        <div style={{ marginBottom: advancedExpanded ? 16 : 0 }}>
          <Button
            type="link"
            onClick={() => setAdvancedExpanded((value) => !value)}
            style={{ padding: 0, height: 'auto' }}
          >
            {advancedExpanded ? <DownOutlined /> : <RightOutlined />}
            <span style={{ marginLeft: 4 }}>{t('common.advancedSettings')}</span>
          </Button>
        </div>

        {advancedExpanded && (
          <>
            <Form.Item
              label={t(getKey('headers'))}
              name="headers"
              extra={(
                <Text type="secondary" style={{ fontSize: 12 }}>
                  {t('settings.provider.headersHint')}
                </Text>
              )}
            >
              <HeadersEditor
                outputFormat="object"
                height={200}
                onValidationChange={setHeadersValid}
                placeholder={`{
    "Helicone-Cache-Enabled": "true",
    "Helicone-User-Id": "opencode"
}`}
              />
            </Form.Item>

            <Form.Item
              label={t('opencode.provider.timeout')}
              extra={(
                <Text type="secondary" style={{ fontSize: 12 }}>
                  {t('opencode.provider.timeoutHint')}
                </Text>
              )}
            >
              <Form.Item noStyle shouldUpdate={(prev, curr) => prev.disableTimeout !== curr.disableTimeout}>
                {({ getFieldValue }) => (
                  <Space style={{ width: '100%' }}>
                    {!getFieldValue('disableTimeout') && (
                      <Form.Item name="timeout" noStyle>
                        <InputNumber
                          style={{ width: 200 }}
                          placeholder={t('opencode.provider.timeoutPlaceholder')}
                          min={0}
                          addonAfter="ms"
                        />
                      </Form.Item>
                    )}
                    <Form.Item name="disableTimeout" valuePropName="checked" noStyle>
                      <Checkbox>{t('opencode.provider.disableTimeout')}</Checkbox>
                    </Form.Item>
                  </Space>
                )}
              </Form.Item>
            </Form.Item>

            <Form.Item
              label={t('opencode.provider.setCacheKey')}
              name="setCacheKey"
              valuePropName="checked"
              extra={(
                <Text type="secondary" style={{ fontSize: 12 }}>
                  {t('opencode.provider.setCacheKeyHint')}
                </Text>
              )}
            >
              <Switch />
            </Form.Item>

            <Form.Item
              label={t('opencode.provider.extraOptions')}
              name="extraOptions"
              extra={(
                <Text type="secondary" style={{ fontSize: 12 }}>
                  {t('opencode.provider.extraOptionsHint')}
                </Text>
              )}
              getValueFromEvent={(value: unknown, isValid: boolean) => {
                setExtraOptionsValid(isValid);
                // Keep the user's text even while it is invalid; submit is
                // blocked by the flag above instead of silently clearing it.
                return value;
              }}
            >
              <JsonEditor
                value={form.getFieldValue('extraOptions')}
                height={150}
                minHeight={100}
                maxHeight={300}
                resizable
                placeholder={`{
    "organization": "my-org",
    "project": "my-project"
}`}
              />
            </Form.Item>

            <Divider style={{ margin: '8px 0 16px' }} />

            <Form.Item noStyle shouldUpdate={(prev, curr) => prev.filterMode !== curr.filterMode}>
              {({ getFieldValue }) => {
                const filterMode = (getFieldValue('filterMode') as string | undefined) || 'whitelist';
                const hintKey = filterMode === 'whitelist'
                  ? 'opencode.modelFilter.modeHintWhitelist'
                  : 'opencode.modelFilter.modeHintBlacklist';
                return (
                  <Form.Item
                    label={t('opencode.modelFilter.modeLabel')}
                    name="filterMode"
                    initialValue="whitelist"
                    extra={(
                      <Text type="secondary" style={{ fontSize: 12 }}>
                        {t(hintKey)}
                      </Text>
                    )}
                  >
                    <Radio.Group>
                      <Radio value="whitelist">{t('opencode.modelFilter.modeWhitelist')}</Radio>
                      <Radio value="blacklist">{t('opencode.modelFilter.modeBlacklist')}</Radio>
                    </Radio.Group>
                  </Form.Item>
                );
              }}
            </Form.Item>

            <Form.Item label={t('opencode.modelFilter.modelsLabel')} name="filterModels">
              <Select
                mode="multiple"
                placeholder={t('opencode.modelFilter.modelsPlaceholder')}
                options={modelOptions}
                showSearch
                optionFilterProp="label"
                allowClear
              />
            </Form.Item>
          </>
        )}

        <ProviderFormSections
          editable
          showBilling={false}
          showCustomHeaders={false}
          showModelRewrites={false}
          showModelServiceTiers={false}
          showNotes={false}
          notesResetKey="opencode-provider-form-notes"
        />
      </Form>
    </Modal>
  );
};

export default OpenCodeProviderFormModal;
