import React from 'react';
import { Button, Space, Typography, message } from 'antd';
import { CopyOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { middleEllipsis } from '@/utils/middleEllipsis';
import styles from './ProviderDetailPanel.module.less';

const { Text, Title } = Typography;

/**
 * A connection fact, as one `label: value` row.
 *
 * The row is data rather than a node because both CLIs state the same four
 * kinds of thing (endpoint, credential, protocol, model ids) and the layout —
 * the fixed label column, the monospaced value, the copy button on the right —
 * is the part that must not drift between them.
 */
export interface ProviderDetailField {
  /**
   * Identifies the field: keys the React row and, when `copyable`, becomes the
   * `data-provider-detail-copy` handle the browser fixtures drive.
   */
  key: string;
  label: string;
  /** The value. Empty renders `emptyText`, or nothing at all when that is unset. */
  value?: string;
  /**
   * What to say when there is no value.
   *
   * Omitting it makes the row **conditional**: a field the caller only has for
   * some providers renders nothing rather than an empty row with a placeholder
   * — which is how the protocol row disappears for official subscriptions.
   */
  emptyText?: string;
  /** Adds the copy button, which copies `value` verbatim. */
  copyable?: boolean;
  /**
   * Shortens the value in the middle when it does not fit one line.
   *
   * Credentials only, so far: an endpoint reads fine wrapped and a model id is
   * short, while a long key loses its identity when elided at the end.
   */
  elideMiddle?: boolean;
}

export interface ProviderDetailSection {
  key: string;
  title?: string;
  /** Rows, rendered in order. */
  fields?: ProviderDetailField[];
  /**
   * Free-form content under the title — a model catalog, an account block.
   *
   * A node rather than more fields: those sections are interactive components
   * with their own toolbars, and the panel has nothing to add beyond the place
   * to put them.
   */
  content?: React.ReactNode;
}

interface ProviderDetailPanelProps {
  /** The provider's name, usually a `ProviderNameLink`. */
  title: React.ReactNode;
  /** State tags on the title's line (applied / official / local). */
  statusTags?: React.ReactNode;
  /**
   * The card's own action row (launch / gateway / apply).
   *
   * It sits on the title's state line rather than in the footer: these are what
   * you came to the sidebar *to do*, and on a provider with several model
   * bindings the footer is far enough down to need a scroll. The footer keeps
   * the quieter row (test / edit / copy / share / delete), which reads as the
   * "and also" set beneath them.
   */
  primaryActions?: React.ReactNode;
  /** The secondary action row (test / edit / copy / share / delete). */
  secondaryActions?: React.ReactNode;
  sections: ProviderDetailSection[];
  onClose: () => void;
}

/**
 * The copy affordance beside a field.
 *
 * The endpoint and the key exist to be pasted somewhere else — into a curl, a
 * bug report, another tool's config — and both are long enough that selecting
 * them by hand is the slowest part of that.
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

const FieldRow: React.FC<{ field: ProviderDetailField }> = ({ field }) => {
  const value = field.value?.trim();

  if (!value && !field.emptyText) {
    return null;
  }

  return (
    <div className={styles.fieldRow}>
      <span className={styles.fieldLabel}>{field.label}</span>
      <span className={styles.fieldValue} data-provider-detail-field={field.key}>
        {value
          ? (
            <Text
              code
              className={field.elideMiddle ? styles.keyValue : undefined}
              title={field.elideMiddle ? value : undefined}
            >
              {field.elideMiddle ? middleEllipsis(value) : value}
            </Text>
          )
          : <Text type="secondary">{field.emptyText}</Text>}
        {value && field.copyable && <FieldCopyButton field={field.key} value={value} />}
      </span>
    </div>
  );
};

/**
 * The provider detail sidebar the card view opens.
 *
 * Why it exists: a grid cell can state the title, a model line and the two
 * primary actions — and nothing else. Everything a provider *is* and every
 * other thing you can *do* to it has to live somewhere reachable, or the card
 * view would be a strictly worse list.
 *
 * It is purely presentational: each CLI's mapping layer computes the values and
 * passes the action nodes in, so the panel and that CLI's cards render the
 * *same* computed state and cannot drift apart. The panel shared the whole
 * layout since the second CLI arrived — Claude and Codex state the same kinds
 * of fact (endpoint, credential, model bindings) and only the section list and
 * the extra content differ.
 */
const ProviderDetailPanel: React.FC<ProviderDetailPanelProps> = ({
  title,
  statusTags,
  primaryActions,
  secondaryActions,
  sections,
  onClose,
}) => {
  const { t } = useTranslation();

  /**
   * The fields of a section that will actually render.
   *
   * `FieldRow` drops a row with neither a value nor an `emptyText` (that is how
   * a conditional field disappears), so the card has to ask the same question:
   * a section whose only field is conditional would otherwise be a bordered box
   * with 12px of padding around nothing.
   */
  const renderableFields = (fields?: ProviderDetailField[]) =>
    (fields ?? []).filter((field) => field.value?.trim() || field.emptyText);

  return (
    <div className={styles.panel} data-provider-detail-panel>
      <header className={styles.header}>
        <div className={styles.titleBlock}>
          <Title level={5} className={styles.title}>{title}</Title>
          <Space size={6} wrap className={styles.statusLine} data-provider-detail-status>
            {statusTags}
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
        {sections.map((section) => {
          const fields = renderableFields(section.fields);

          return (
            <section key={section.key} className={styles.section} data-provider-detail-section>
              {section.title && (
                <div className={styles.sectionTitle} data-provider-detail-section-title>
                  {section.title}
                </div>
              )}
              {fields.length > 0 && (
                <div className={styles.fieldCard}>
                  {fields.map((field) => <FieldRow key={field.key} field={field} />)}
                </div>
              )}
              {section.content}
            </section>
          );
        })}
      </div>

      <footer className={styles.footer} data-provider-detail-footer>
        <div className={styles.secondaryActions}>{secondaryActions}</div>
      </footer>
    </div>
  );
};

export default ProviderDetailPanel;