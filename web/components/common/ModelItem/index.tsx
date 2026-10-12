import React from 'react';
import { Button, Space, Typography, Popconfirm, Checkbox, Switch, Tooltip } from 'antd';
import { EditOutlined, DeleteOutlined, HolderOutlined, CopyOutlined, CheckCircleOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import type { ModelDisplayData } from '@/components/common/ProviderCard/types';
import { formatModelLimit } from '@/utils/modelLimits';
import styles from './styles.module.less';

const { Text } = Typography;

interface ModelItemProps {
  model: ModelDisplayData;

  /** Whether the item is draggable */
  draggable?: boolean;
  /** Unique ID for sortable (defaults to model.id) */
  sortableId?: string;

  /** Callbacks */
  onEdit?: () => void;
  onCopy?: () => void;
  onDelete?: () => void;
  onSetPrimary?: () => void;
  selectionMode?: boolean;
  selected?: boolean;
  onSelectChange?: (selected: boolean) => void;

  /** Optional row action rendered before the built-in actions (e.g. Codex
   *  "set as auto-review model"). Hidden while a selection mode is active. */
  extraActions?: React.ReactNode;

  /**
   * When true, content fill is transparent so a selected parent card tint shows
   * through. Borders stay. Used by Grok multi-model lists.
   */
  transparentBackground?: boolean;

  /**
   * Whether this model is switched off in the CLI.
   *
   * Only meaningful together with `onToggleDisabled`: a model list that cannot
   * change the flag should not render a switch it cannot honour.
   */
  isDisabled?: boolean;
  onToggleDisabled?: () => void;
}

/**
 * A reusable model list item component with optional drag-and-drop support
 */
const ModelItem: React.FC<ModelItemProps> = ({
  model,
  draggable = false,
  sortableId,
  onEdit,
  onCopy,
  onDelete,
  onSetPrimary,
  selectionMode = false,
  selected = false,
  onSelectChange,
  extraActions,
  transparentBackground = false,
  isDisabled = false,
  onToggleDisabled,
}) => {
  const { t } = useTranslation();

  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({
    id: sortableId || model.id,
    disabled: !draggable,
  });

  const style: React.CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : 1,
    background: transparentBackground ? 'transparent' : 'var(--color-bg-container)',
    border: selected ? '1px solid var(--ant-color-primary)' : '1px solid var(--color-border-secondary)',
    borderRadius: 4,
    padding: '8px 12px',
    display: 'flex',
    // Items centre within their own flex line, and the lines stack from the
    // top — so a wrapped action group sits under the text rather than floating
    // against the row's middle.
    alignItems: 'center',
    // Narrow hosts (the 478px detail sidebar) cannot fit the text and the
    // action group side by side; see `.summary`.
    flexWrap: 'wrap',
    rowGap: 4,
    gap: 8,
  };

  const contextLimit = formatModelLimit(model.contextLimit);
  const showPrimaryAction = !selectionMode && onSetPrimary;

  return (
    <div
      ref={setNodeRef}
      style={style}
      className={styles.container}
    >
      {draggable && (
        <div
          {...attributes}
          {...listeners}
          style={{
            cursor: 'grab',
            display: 'flex',
            alignItems: 'center',
          }}
        >
          <HolderOutlined style={{ fontSize: 14, color: '#bbb' }} />
        </div>
      )}

      {selectionMode && onSelectChange && (
        <Checkbox
          checked={selected}
          onChange={(event) => onSelectChange(event.target.checked)}
          aria-label={t('common.model.selectModel', { name: model.name })}
        />
      )}

      {/* One line: name, id, context limit, default marker. The limits used to
          sit on a second line labelled in full; folding the context window into
          the parentheses keeps the row scannable and drops a line that repeated
          the same information for every model in the list.

          One line by construction, not by luck: the actions beside this block
          are hover-revealed with `visibility`, so they hold their width at all
          times. See `.summary` in the stylesheet for what that costs on a
          narrow host. */}
      <div className={styles.summary}>
        <span className={styles.summaryLine} title={`${model.name} (${model.id}${contextLimit ? ` | ${contextLimit}` : ''})`}>
          <Text strong className={styles.modelName}>
            {model.name}
          </Text>
          <Text type="secondary" className={styles.modelMeta}>
            {contextLimit ? `(${model.id} | ${contextLimit})` : `(${model.id})`}
          </Text>
          {model.isPrimary && (
            <Text type="secondary" className={styles.modelMeta}>
              · {t('common.model.currentPrimary')}
            </Text>
          )}
        </span>
      </div>

      <Space>
        {!selectionMode && extraActions && (
          // Same hover-reveal as the primary action: the row stays quiet until
          // the pointer is over it.
          <span className={styles.primaryAction}>{extraActions}</span>
        )}
        {showPrimaryAction && (
          <Button
            className={styles.primaryAction}
            size="small"
            type="text"
            icon={<CheckCircleOutlined />}
            onClick={onSetPrimary}
            disabled={model.isPrimary}
          >
            {model.isPrimary
              ? t('common.model.alreadyPrimary')
              : t('common.model.setAsPrimary')}
          </Button>
        )}
        {!selectionMode && onEdit && (
          <Button size="small" type="text" icon={<EditOutlined />} onClick={onEdit} />
        )}
        {!selectionMode && onCopy && (
          <Button size="small" type="text" icon={<CopyOutlined />} onClick={onCopy} />
        )}
        {!selectionMode && onDelete && (
          <Popconfirm
            title={t('common.model.deleteModel')}
            description={t('common.model.confirmDelete', { name: model.name })}
            onConfirm={onDelete}
            okText={t('common.confirm')}
            cancelText={t('common.cancel')}
          >
            <Button size="small" type="text" danger icon={<DeleteOutlined />} />
          </Popconfirm>
        )}
        {!selectionMode && onToggleDisabled && (
          // Always visible, unlike the hover-revealed actions above: a switch
          // reports state, and a state you can only see by hovering is one the
          // user cannot scan the list for.
          <Tooltip
            title={
              isDisabled
                ? t('common.model.disabled')
                : t('common.model.enabled')
            }
          >
            <Switch
              size="small"
              checked={!isDisabled}
              onChange={() => onToggleDisabled()}
              aria-label={t('common.model.toggleEnabled', { name: model.name })}
            />
          </Tooltip>
        )}
      </Space>
    </div>
  );
};

export default ModelItem;
