import React from 'react';
import { Button, Space, Tag, Typography, message } from 'antd';
import { CopyOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import type { ClaudeCodeProvider } from '@/types/claudecode';
import ProviderNameLink from '@/components/common/ProviderNameLink';
import AppliedTag from '@/components/common/AppliedTag';
import { middleEllipsis } from '../utils/middleEllipsis';
import styles from './ClaudeProviderDetailPanel.module.less';

const { Text, Title } = Typography;

/**
 * The copy affordance beside a connection field.
 *
 * The endpoint and the key exist to be pasted somewhere else — into a curl, a
 * bug report, another tool's config — and both are long enough that selecting
 * them by hand is the slowest part of that. Lives here rather than in a shared
 * helper because it is the panel's own field styling that it has to line up
 * with, and no other surface renders these fields.
 */
const FieldCopyButton: React.FC<{ field: string; value: string }> = ({ field, value }) => {
  const { t } = useTranslation();

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(value);
      message.success(t('common.copied'));
    } catch {
      // The clipboard can be unavailable (no permission, no secure context).
      // Saying so beats a button that appears to do nothing.
      message.error(t('common.copyFailed'));
    }
  };

  return (
    <Button
      type="text"
      size="small"
      className={styles.copyButton}
      data-provider-detail-copy={field}
      icon={<CopyOutlined />}
      onClick={handleCopy}
      title={t('common.copy')}
      aria-label={t('common.copy')}
    />
  );
};

export interface ClaudeProviderDetailBinding {
  label: string;
  model: string;
}

interface ClaudeProviderDetailPanelProps {
  provider: ClaudeCodeProvider;
  /** Endpoint and credential, stated in full. */
  baseUrl: string;
  /**
   * The credential, verbatim.
   *
   * It is **not** masked, and the masking is not an oversight: a key you cannot
   * read cannot be checked against the provider's console, and this panel is
   * already behind a deliberate click on the card body. It is shortened for
   * display when it is long (`middleEllipsis`), but what it *states* is the
   * whole value — unlike the list card's `xxxx…abcd`, which is all a single
   * line of a list can afford.
   */
  apiKey: string;
  /**
   * The wire protocol, already resolved to a readable name by the mapping
   * layer; `null` when the provider has no protocol of its own (an official
   * subscription) and the row is simply not rendered.
   */
  apiFormatLabel: string | null;
  /** Every configured role binding, in the list card's order. */
  modelBindings: ClaudeProviderDetailBinding[];
  showRuntimeApplied: boolean;
  isOfficialProvider: boolean;
  isLocalProvider: boolean;
  onClose: () => void;
  /**
   * The card's own action row (launch / gateway / apply), already computed by
   * the mapping layer.
   *
   * It sits on the title's tag line rather than in the footer: these are what
   * you came to the sidebar *to do*, and on a provider with several model
   * bindings the footer is far enough down to need a scroll. The footer keeps
   * the quieter row (test / edit / copy / share / delete), which reads as the
   * "and also" set beneath them.
   */
  primaryActions: React.ReactNode;
  /** The secondary action row (test / edit / copy / share / delete). */
  secondaryActions: React.ReactNode;
}

/**
 * The provider detail sidebar the card view opens.
 *
 * Why it exists: a grid cell can state the title, its role models and the two
 * primary actions — and nothing else. Everything a provider *is* (endpoint,
 * every role binding, notes) and every other thing you can *do* to it has to
 * live somewhere reachable, or the card view would be a strictly worse list.
 * This is that somewhere.
 *
 * It is purely presentational: the owning card computes the values and passes
 * the action nodes in, so the panel and the list card render the *same*
 * computed state and cannot drift apart.
 */
const ClaudeProviderDetailPanel: React.FC<ClaudeProviderDetailPanelProps> = ({
  provider,
  baseUrl,
  apiKey,
  apiFormatLabel,
  modelBindings,
  showRuntimeApplied,
  isOfficialProvider,
  isLocalProvider,
  onClose,
  primaryActions,
  secondaryActions,
}) => {
  const { t } = useTranslation();

  return (
    <div className={styles.panel} data-provider-detail-panel>
      <header className={styles.header}>
        <div className={styles.titleBlock}>
          <Title level={5} className={styles.title}>
            <ProviderNameLink name={provider.name} baseUrl={baseUrl} />
          </Title>
          <Space size={6} wrap className={styles.statusLine} data-provider-detail-status>
            {isLocalProvider && (
              <Text type="secondary" style={{ fontSize: 11 }}>
                ({t('claudecode.localConfigHint')})
              </Text>
            )}
            {isOfficialProvider && <Tag>{t('claudecode.provider.modeOfficial')}</Tag>}
            {showRuntimeApplied && (
              // The same `AppliedTag` the card and the list card draw, not a
              // blue antd `Tag`: the drawer was the only surface stating this
              // state in a different colour, and it states it about the very
              // provider whose card you just clicked.
              <AppliedTag>{t('claudecode.provider.applied')}</AppliedTag>
            )}
            {primaryActions}
          </Space>
        </div>
        <Button
          type="text"
          size="small"
          onClick={onClose}
          className={styles.closeButton}
          data-provider-detail-close
        >
          {t('common.close')}
        </Button>
      </header>

      <div className={styles.body}>
        <section className={styles.section} data-provider-detail-section>
          <div className={styles.sectionTitle} data-provider-detail-section-title>{t('claudecode.detail.connectionTitle')}</div>
          <div className={styles.fieldRow}>
            <span className={styles.fieldLabel}>{t('claudecode.provider.baseUrl')}</span>
            <span className={styles.fieldValue}>
              {baseUrl
                ? <Text code>{baseUrl}</Text>
                : <Text type="secondary">{t('claudecode.detail.notConfigured')}</Text>}
              {baseUrl && <FieldCopyButton field="baseUrl" value={baseUrl} />}
            </span>
          </div>
          {apiFormatLabel && (
            <div className={styles.fieldRow}>
              <span className={styles.fieldLabel}>{t('claudecode.provider.apiFormat')}</span>
              <span className={styles.fieldValue}>{apiFormatLabel}</span>
            </div>
          )}
          <div className={styles.fieldRow}>
            <span className={styles.fieldLabel}>{t('claudecode.provider.apiKey')}</span>
            <span className={styles.fieldValue}>
              {apiKey
                ? (
                  <Text code className={styles.keyValue} title={apiKey} data-provider-detail-key>
                    {middleEllipsis(apiKey)}
                  </Text>
                )
                : <Text type="secondary">{t('claudecode.detail.notConfigured')}</Text>}
              {apiKey && <FieldCopyButton field="apiKey" value={apiKey} />}
            </span>
          </div>
        </section>

        <section className={styles.section} data-provider-detail-section>
          <div className={styles.sectionTitle} data-provider-detail-section-title>{t('claudecode.model.mappingTitle')}</div>
          {modelBindings.length > 0 ? (
            modelBindings.map((binding) => (
              <div key={binding.label} className={styles.fieldRow}>
                <span className={styles.fieldLabel}>{binding.label}</span>
                <span className={styles.fieldValue}>
                  <Text code>{binding.model}</Text>
                </span>
              </div>
            ))
          ) : (
            <Text type="secondary" style={{ fontSize: 12 }}>
              {t('claudecode.detail.noModels')}
            </Text>
          )}
        </section>

        {provider.notes && (
          <section className={styles.section} data-provider-detail-section>
            <div className={styles.sectionTitle} data-provider-detail-section-title>{t('common.provider.notes')}</div>
            <Text type="secondary" style={{ fontSize: 12 }}>{provider.notes}</Text>
          </section>
        )}
      </div>

      <footer className={styles.footer} data-provider-detail-footer>
        <div className={styles.secondaryActions}>{secondaryActions}</div>
      </footer>
    </div>
  );
};

export default ClaudeProviderDetailPanel;
