import type { ProviderCardState } from './types';

/**
 * The card's accent chrome — border and background — for the two "this one is
 * special" states, as plain CSS values.
 *
 * Extracted because there are now two card *shells* (`CardShell` for the three
 * list styles, `ProviderCompactCard` for the card-grid view) and they must
 * agree pixel for pixel: a provider that is applied looks the same whichever
 * layout the user picked. This is the repo's recorded failure mode — when the
 * four bespoke cards were migrated onto the shared shell, the accent rules did
 * not come along and every migrated card silently turned grey, with no error
 * anywhere (see `providerCardVariants/AGENTS.md`).
 *
 * Precedence, from the bespoke cards:
 *   batch selection > gateway primary (P0) > applied
 * The gateway role wins over `applied` because a provider can be both and the
 * gateway role is the more specific fact; selection wins over both because the
 * user is acting on the selection, and a selected card that does not look
 * selected is the worse failure.
 */
export interface ProviderAccentChrome {
  borderColor: string;
  background: string | undefined;
}

export function resolveProviderAccentChrome(
  accent: ProviderCardState['accent'],
  isSelected: boolean,
): ProviderAccentChrome {
  const borderColor = isSelected || accent === 'applied'
    ? 'var(--ant-color-primary)'
    : accent === 'gatewayPrimary'
      ? 'var(--color-status-success)'
      : 'var(--color-border-card)';

  const background = isSelected
    ? undefined
    : accent === 'gatewayPrimary'
      ? 'linear-gradient(135deg, color-mix(in srgb, var(--color-status-success) 12%, var(--color-bg-container)), var(--color-bg-container))'
      : accent === 'applied'
        ? 'var(--color-bg-selected)'
        : undefined;

  return { borderColor, background };
}

/**
 * The card's own hover surface, shared for the same reason as the accent.
 *
 * Returned as a pair so the two shells cannot drift into "one lifts on hover
 * and the other does not".
 */
export const PROVIDER_CARD_SHADOW = 'var(--shadow-card-sm)';
export const PROVIDER_CARD_SHADOW_HOVER = 'var(--shadow-card-sm-hover)';
