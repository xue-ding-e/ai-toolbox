import React from 'react';
import { Dropdown } from 'antd';
import { HolderOutlined, MoreOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import ProviderNameLink from '@/components/common/ProviderNameLink';
import ProviderConnectivityStatus from '@/features/coding/shared/providerConnectivity/ProviderConnectivityStatus';
import { ManagementCheckbox } from '@/features/coding/shared/management';
import {
  resolveProviderAccentChrome,
} from '@/features/coding/shared/providerCardVariants/cardChrome';
import {
  renderProviderLaunchAction,
  renderProviderPrimaryAction,
  type ProviderCardLaunchAction,
} from '@/features/coding/shared/providerCardVariants/providerCardActions';
import {
  buildProviderCardMenuItems,
  dispatchProviderCardMenuClick,
} from '@/features/coding/shared/providerCardVariants/providerCardMenu';
import type {
  ProviderCardActions,
  ProviderCardModel,
  ProviderCardState,
} from '@/features/coding/shared/providerCardVariants/types';
import styles from './ProviderCompactCard.module.less';

/**
 * The compact provider card used by the card-grid layout.
 *
 * ```
 * ⠿  Name                                            ⋮   ← ⋮ on hover
 *    Opus: claude-opus-4-8
 *    ─────────────────────────────────────────────────────
 *    CLI 启动                              网关代理  应用
 * ```
 *
 * Clicking the body — anywhere that is not itself a control — opens the
 * detail sidebar the caller renders; the grid cell states one model and the two
 * primary actions, and everything else about the provider lives in there.
 *
 * ## Why this is not a fourth `providerCardVariants` style
 *
 * The three styles in `providerCardVariants/` are chosen by the **shape of a
 * CLI's provider model** (does it own a model catalog? does it have a single
 * active provider?). This card is chosen by **how dense the user wants the
 * list** — the same provider, the same CLI, rendered two ways. Mixing it into
 * the variants would make "which style" answerable by a display preference,
 * which is exactly the conflation that module exists to prevent.
 *
 * ## Why it reuses `ProviderCardState` / `ProviderCardActions`
 *
 * The mapping layer already computes both for the list card. Reusing them means
 * a provider's actions, tags and highlight cannot differ between the two
 * layouts — the caller passes one object to whichever card it renders. That is
 * the whole point: "the buttons behave the same in both views" is structural
 * here, not a thing to keep in sync by hand.
 *
 * ## What it owns
 *
 * Everything `CardShell` owns for the list styles: sortable registration, the
 * selection checkbox, `data-provider-id`, and the accent chrome. The guard in
 * `scripts/verify-provider-card-layout.mjs` scopes to `*ProviderCard.tsx` /
 * `*OfficialAccountCard.tsx` mapping layers; this file is a shared shell like
 * `CardShell`, which is why its name deliberately does not match that suffix.
 */
/**
 * Re-exported under the name the card view has always used, so callers and the
 * module barrel keep their import path while the shape itself lives beside the
 * renderer that draws it.
 */
export type ProviderCompactCardLaunchAction = ProviderCardLaunchAction;

interface ProviderCompactCardProps {
  /** Title line. */
  provider: ProviderCardModel;
  /**
   * The second line: every `Role: model` pair the CLI states, in order.
   *
   * It is one truncated line rather than a wrapped block — the card's height
   * has to stay uniform across the grid, and a provider with six bindings must
   * not make its row taller than its neighbours'. Omit it (or pass an empty
   * array) to render no second line at all.
   */
  subtitle?: { label: string; value: string }[];
  /** Same object the list card receives. */
  providerState?: ProviderCardState;
  /** Same object the list card receives. */
  actions?: ProviderCardActions;
  /**
   * State tags beside the title — 已应用 / 官方 / 代理.
   *
   * The same slot the list card takes, minus the endpoint: the mapping layer
   * builds the list from whichever facts its layout states, so "this provider
   * is applied" cannot be true in one layout and invisible in the other. The
   * border alone was doing that job here, and a coloured border does not say
   * *why* it is coloured.
   */
  nameTags?: React.ReactNode;
  /** Footer's left side. */
  launchAction?: ProviderCompactCardLaunchAction;
  /**
   * Extra entries for the "more" menu, inserted after 分享 and before the
   * divider. The card view folds the list card's inline 连通性测试 here.
   */
  extraMenuItems?: Parameters<typeof buildProviderCardMenuItems>[0]['extraItems'];
  /**
   * Opens the detail sidebar for this provider.
   *
   * The grid cell states one model and carries the two primary actions; every
   * other fact about the provider (endpoint, all role bindings, notes, the
   * remaining actions) lives in that sidebar. Clicking anywhere on the card
   * that is not itself a control opens it — the same "the card is the way in"
   * convention the Skills and MCP cards use.
   */
  onOpenDetail?: () => void;
}

const ProviderCompactCard: React.FC<ProviderCompactCardProps> = ({
  provider,
  subtitle,
  providerState,
  actions,
  nameTags,
  launchAction,
  extraMenuItems,
  onOpenDetail,
}) => {
  const { t } = useTranslation();
  const {
    isDisabled,
    onToggleDisabled,
    connectivityStatus,
    selectable = false,
    selected = false,
    onSelectChange,
    dimmed = false,
    draggable = false,
    sortableId,
    accent,
  } = providerState ?? {};

  // `useSortable` must run on every render, so a card without an id still
  // registers — under a placeholder, disabled. Same rule as `CardShell`.
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: sortableId ?? 'provider-compact-card',
    disabled: !draggable,
  });

  const isSelected = selectable && selected;
  const { borderColor, background } = resolveProviderAccentChrome(accent, isSelected);

  const style: React.CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : dimmed ? 0.6 : 1,
    // Read by `ProviderListSection`'s locate ring, whose radius must follow the
    // card's own (10px here, 8px on the list cards) or the flash draws square
    // corners around a rounded card.
    '--locate-flash-radius': '10px',
  } as React.CSSProperties;

  const showHandle = draggable && !selectable;
  const primary = actions?.primaryAction;
  const menuItems = buildProviderCardMenuItems({
    t,
    actions,
    isDisabled,
    onToggleDisabled,
    extraItems: extraMenuItems,
  });

  /**
   * The card body opens the detail sidebar; anything that is itself a control
   * handles its own click. Without this guard, dragging the grip, ticking the
   * checkbox or pressing a footer button would also open the sidebar.
   *
   * Batch-selection mode suppresses the body click entirely, the same way
   * `SkillCard` does: in that mode a click on the body means "select this
   * card", and covering the list with a drawer instead would make selecting a
   * run of cards impossible.
   */
  const handleCardClick: React.MouseEventHandler<HTMLDivElement> = (event) => {
    if (!onOpenDetail || selectable) {
      return;
    }
    const target = event.target as HTMLElement;
    if (target.closest('button, a, input, [role="menuitem"], [role="checkbox"]')) {
      return;
    }
    onOpenDetail();
  };

  return (
    // The ref is attached even when dragging is off: dnd-kit measures the node
    // to compute transforms, and an unregistered node cannot be dragged the
    // moment the parent re-enables it.
    <div ref={setNodeRef} data-provider-id={sortableId} className={styles.cardOuter} style={style}>
      <div
        className={styles.card}
        style={{ borderColor, background }}
        data-provider-card-clickable={onOpenDetail && !selectable ? '' : undefined}
        onClick={onOpenDetail && !selectable ? handleCardClick : undefined}
      >
        <div className={styles.headerRow}>
          {selectable ? (
            <ManagementCheckbox
              checked={selected}
              ariaLabel={t('common.batch.selectItem')}
              onChange={onSelectChange ?? (() => {})}
            />
          ) : showHandle ? (
            <span
              {...attributes}
              {...listeners}
              className={styles.dragHandle}
              data-provider-card-handle
              title={t('common.provider.reorderHint')}
              aria-label={t('common.provider.reorderHint')}
            >
              <HolderOutlined style={{ fontSize: 14 }} />
            </span>
          ) : null}
          <ProviderConnectivityStatus item={connectivityStatus} />
          <ProviderNameLink
            name={provider.name}
            baseUrl={provider.baseUrl}
            className={styles.name}
            style={{ fontSize: 14, fontWeight: 600 }}
          />
          {nameTags && <span className={styles.nameTags} data-provider-card-tags>{nameTags}</span>}
          {menuItems.length > 0 && (
            // Hover-revealed. Opacity only: the button keeps its box at all
            // times, so the title cannot reflow as the pointer crosses the card.
            <span className={styles.hoverActions} data-provider-card-actions>
              <Dropdown
                trigger={['click']}
                menu={{
                  items: menuItems,
                  onClick: ({ key }) => dispatchProviderCardMenuClick(key, actions),
                }}
              >
                <button type="button" className={styles.miniBtn} title={t('common.more')} aria-label={t('common.more')}>
                  <MoreOutlined />
                </button>
              </Dropdown>
            </span>
          )}
        </div>

        <div className={styles.body}>
          {subtitle && subtitle.length > 0 && (
            // One line of `Label: value` pairs, the values monospaced — the
            // same shape the list card's binding line uses, so a role is
            // readable at a glance instead of being an unexplained model name.
            //
            // The pairs are inline spans, not flex items: a flex row would clip
            // at the last whole item and show no ellipsis, while inline content
            // lets the browser place one at the truncation point. The `title`
            // carries the full line, which is the only way to read what the
            // ellipsis hid.
            <div
              className={styles.subtitle}
              data-provider-subtitle
              title={subtitle.map((entry) => `${entry.label}: ${entry.value}`).join('  ')}
            >
              {subtitle.map((entry) => (
                <span key={entry.label} className={styles.subtitleEntry}>
                  <span className={styles.subtitleLabel}>{entry.label}:</span>
                  <span className={styles.subtitleValue}>{entry.value}</span>
                </span>
              ))}
            </div>
          )}
        </div>

        <div className={styles.footerRow} data-provider-card-footer>
          {renderProviderLaunchAction(launchAction)}
          <span className={styles.footerActions}>
            {actions?.gatewayActions}
            {renderProviderPrimaryAction(primary)}
          </span>
        </div>
      </div>
    </div>
  );
};

export default ProviderCompactCard;
