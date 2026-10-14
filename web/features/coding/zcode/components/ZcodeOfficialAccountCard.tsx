import React from 'react';
import { Button, Dropdown, Space } from 'antd';
import { LinkOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { CardShell } from '@/features/coding/shared/providerCardVariants';
import { OfficialAccountsSection } from '@/features/coding/shared/officialAccounts';
import type { OfficialAccountRowView } from '@/features/coding/shared/officialAccounts';
import {
  OfficialAccountCount,
  OfficialAccountHeadingIcon,
} from '@/features/coding/shared/officialAccounts';
import { ProviderCompactCard } from '@/features/coding/shared/providerCardView';
import type { ProviderViewMode } from '@/features/coding/shared/providerList';
import type { ZcodeOfficialAccount } from '@/types/zcode';

export interface ZcodeOfficialAccountCardProps {
  accounts: ZcodeOfficialAccount[];
  /** A saved account's switch action is in flight. */
  applyingAccountId?: string | null;
  /** The "save current login" action is in flight. */
  savingCurrent?: boolean;
  /** A browser login is waiting; every action is withheld until it resolves. */
  loginPending?: boolean;
  /** The login providers, in the order the menu lists them. */
  loginProviders: readonly { value: string; label: string }[];
  /**
   * Sortable id, so the card can be dragged among the provider cards. Omit to
   * render it pinned with no drag handle.
   */
  sortableId?: string;
  /** Renders the handle inert while batch selection owns the pointer. */
  dragDisabled?: boolean;
  /**
   * Which of the two shapes this member wears. Both are the host's, not this
   * file's choice:
   *
   * - `'list'` — a `CardShell`, the same bordered card its provider neighbours
   *   are, with the block's own heading inside it.
   * - `'card'` — a `ProviderCompactCard`, the same cell its neighbours are. The
   *   cell states the live login and the account manager opens in the cell's
   *   own sidebar, which is where Codex's and Kimi's card views put the same
   *   block. It is a manager (a row per account, each with its own actions), and
   *   a cell holds one line and two actions — the same call the model catalog
   *   got. Everything about the sidebar itself (the drawer, its width, the
   *   panel) belongs to the shared card.
   */
  viewMode?: ProviderViewMode;
  onLogin: (providerId: string) => void;
  onApply: (account: ZcodeOfficialAccount) => void;
  onDelete: (account: ZcodeOfficialAccount) => void;
  onSaveCurrent: () => void;
}

/**
 * The official-account card of the ZCode provider list.
 *
 * ZCode needs its own card because an official login is not a provider: ZCode
 * keeps it in `credentials.json`, outside the provider registry, and the two are
 * switched independently — so there is no provider row for the shared section to
 * sit inside.
 *
 * It is still a peer of the provider cards in the list, and it looks like one:
 * the frame is the host's (the list layout's `CardShell`, the grid's
 * `ProviderCompactCard`), the block inside it is `shared/officialAccounts`, and
 * this file only maps ZCode's own account records onto `OfficialAccountRowView`.
 * Both halves used to be written by hand here, and both drifted — the handle lost
 * its hover feedback and the content lost its left edge, because the handle sat
 * *inside* the heading line instead of in its own column.
 */
const ZcodeOfficialAccountCard: React.FC<ZcodeOfficialAccountCardProps> = ({
  accounts,
  applyingAccountId = null,
  savingCurrent = false,
  loginPending = false,
  loginProviders,
  sortableId,
  dragDisabled = false,
  viewMode = 'list',
  onLogin,
  onApply,
  onDelete,
  onSaveCurrent,
}) => {
  const { t } = useTranslation();

  /**
   * The account list collapses, like Codex's and Kimi's.
   *
   * It starts **open** here: this card's accounts have always been on screen,
   * and the heading above them is what a reader lands on — Codex starts closed
   * only because its card already carries a model section. The ability, not the
   * starting state, is what the three cards share.
   */
  const [accountsCollapsed, setAccountsCollapsed] = React.useState(false);

  const labelForProvider = (providerId: string) =>
    loginProviders.find((provider) => provider.value === providerId)?.label ?? providerId;

  const accountRows = React.useMemo<OfficialAccountRowView[]>(
    () =>
      accounts.map((account) => ({
        id: account.id,
        label: account.email || account.name || t('zcode.officialAccount.unnamed'),
        // The virtual entry is the live login rather than one of the CLI's own
        // providers, so it says "current" instead of naming a provider.
        kindTag: account.isVirtual
          ? t('zcode.officialAccount.currentTag')
          : labelForProvider(account.providerId),
        isApplied: account.isApplied,
        isVirtual: account.isVirtual,
      })),
    // `labelForProvider` reads `loginProviders`, so that is a dependency too.
    [accounts, loginProviders, t],
  );

  // Only the live-but-unsaved row can be saved, and there is at most one.
  const pendingSaveAccountId = accounts.find((account) => account.isVirtual)?.id ?? null;
  const pending = savingCurrent && pendingSaveAccountId
    ? { accountId: pendingSaveAccountId, action: 'save' as const }
    : applyingAccountId
      ? { accountId: applyingAccountId, action: 'apply' as const }
      : null;

  const accountById = (id: string) => accounts.find((account) => account.id === id);

  const loginAction = (
    <Space size={0}>
      <Dropdown
        menu={{
          items: loginProviders.map((provider) => ({
            key: provider.value,
            label: provider.label,
          })),
          onClick: ({ key }) => onLogin(key),
        }}
        disabled={loginPending}
      >
        <Button
          type="text"
          size="small"
          icon={<LinkOutlined />}
          style={{ fontSize: 12 }}
          loading={loginPending}
        >
          {t('zcode.officialAccount.login')}
        </Button>
      </Dropdown>
    </Space>
  );

  /**
   * The block, in the form the host asks for.
   *
   * `heading` is off in the card view's sidebar: the panel's own title is the
   * heading there — built from the same two exports, so the glyph and the count
   * cannot drift — and drawing it twice in one drawer is the "one sentence,
   * rendered twice" defect the fixtures look for.
   *
   * `fold` is off there for the reason Codex's and Kimi's sidebars are open: the
   * sidebar exists to show these rows, so a caret in front of them is a click for
   * nothing. Both forms are the same block; only what its host draws differs.
   */
  const renderAccountSection = ({ heading, fold }: { heading: boolean; fold: boolean }) => (
    <OfficialAccountsSection
      variant="standalone"
      {...(heading ? { headingTitle: t('common.officialAccount.headingTitle') } : {})}
      listTitle={t('common.officialAccount.listTitle')}
      hint={t('zcode.officialAccount.hint')}
      {...(fold
        ? {
            collapsed: accountsCollapsed,
            onToggleCollapsed: () => setAccountsCollapsed((current) => !current),
          }
        : {})}
      applyHint={t('zcode.officialAccount.applyHint')}
      emptyText={t('zcode.officialAccount.empty')}
      accounts={accountRows}
      loginAction={loginAction}
      actionsDisabled={loginPending}
      pending={pending}
      onSaveLocal={() => onSaveCurrent()}
      onApply={(row) => {
        const account = accountById(row.id);
        if (account) {
          onApply(account);
        }
      }}
      onDelete={(row) => {
        const account = accountById(row.id);
        if (account) {
          onDelete(account);
        }
      }}
    />
  );

  if (viewMode === 'card') {
    // The one fact this cell states: which login ZCode is running on. The list
    // layout states it on the account's own row (the 默认 tag), and the label is
    // that same row's label — one account, one way of naming it.
    const appliedRow = accountRows.find((row) => row.isApplied);
    return (
      <ProviderCompactCard
        provider={{
          id: sortableId ?? 'zcode-official-account',
          name: t('common.officialAccount.headingTitle'),
        }}
        // 🔗 官方账号 (n): the heading, built from the same two exports the block
        // draws its own heading from, so the two layouts cannot drift apart.
        namePrefix={<OfficialAccountHeadingIcon />}
        nameTags={<OfficialAccountCount count={accounts.length} />}
        subtitle={appliedRow
          ? [{ label: t('common.officialAccount.currentLogin'), value: appliedRow.label }]
          : undefined}
        // One fact and no action of its own at the card level: switching, saving
        // and deleting are all actions *on an account row*, inside the sidebar.
        actionPlacement="subtitle"
        providerState={{
          draggable: Boolean(sortableId) && !dragDisabled,
          sortableId,
        }}
        detail={{
          title: (
            <Space size={6}>
              <OfficialAccountHeadingIcon />
              {t('common.officialAccount.headingTitle')}
            </Space>
          ),
          statusTags: <OfficialAccountCount count={accounts.length} />,
          // No title on the section: the block is the panel's whole content, and
          // the panel's own title line is already its heading.
          sections: [{
            key: 'officialAccounts',
            content: renderAccountSection({ heading: false, fold: false }),
          }],
        }}
      />
    );
  }

  return (
    <CardShell
      sortableId={sortableId}
      draggable={Boolean(sortableId) && !dragDisabled}
    >
      {renderAccountSection({ heading: true, fold: true })}
    </CardShell>
  );
};

export default ZcodeOfficialAccountCard;