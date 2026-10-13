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

  /**
   * Which host drew the row.
   *
   * `'card'` — the provider list card, which is the row as it has always been
   * drawn: the grip is always on screen and the two labelled actions hold their
   * width and fade in under the pointer. The card has the room for both.
   *
   * `'detail'` — the detail sidebar, where the row is one line that has to end
   * at the section title above it. There everything optional (the grip, the
   * labelled actions) is out of the flow until the pointer arrives, and the type
   * runs a step smaller. See `.detailRow` in the stylesheet.
   */
  appearance?: 'card' | 'detail';
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
  appearance = 'card',
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
    // Both halves, never the boolean form: dnd-kit reads `disabled: true` as
    // "not draggable, but still droppable" (its backwards-compatibility rule).
    // A row that cannot be reordered is then still a drop target of whatever
    // `DndContext` happens to sit above it — and these rows live inside the
    // provider cards, so dragging a *card* resolved `over` to a model row id,
    // which the card list's `indexOf` does not know and silently drops. The
    // list stopped reordering whenever the pointer passed over a model list.
    disabled: { draggable: !draggable, droppable: !draggable },
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
      className={[styles.container, appearance === 'detail' ? styles.detailRow : undefined]
        .filter(Boolean)
        .join(' ')}
      // The row's own identity, for the host that has to restyle it: a model
      // list inside a tinted card drops the row fill (`transparentRows`), and
      // that rule lives in `ModelListSection` rather than in an inline style
      // here — a CSS-module class name is hashed and cannot be targeted.
      data-model-row
    >
      {draggable && (
        <div
          {...attributes}
          {...listeners}
          className={styles.grip}
          style={{ cursor: 'grab' }}
        >
          <HolderOutlined style={{ fontSize: 14 }} />
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

          On the detail appearance this is the whole row at rest: the grip and
          the labelled actions are out of the flow until the pointer arrives, and
          the icon trio beside them fits in what is left. On a card the grip and
          those actions are always in the flow — see `appearance`. */}
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

      {/* The labelled actions are the ones that need room, so their treatment
          is what the host decides: on the detail appearance they are revealed
          under the pointer and take no width while hidden, on a card they hold
          their width and fade in. The icon trio and the switch beside them stay
          either way. See `.hoverAction`. */}
      <Space className={styles.actions}>
        {!selectionMode && extraActions && (
          <span className={styles.hoverAction}>{extraActions}</span>
        )}
        {showPrimaryAction && (
          <Button
            className={styles.hoverAction}
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
