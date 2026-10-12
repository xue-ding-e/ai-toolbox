import React from 'react';
import { Button, Tooltip } from 'antd';
import type { ProviderCardActions } from './types';

/**
 * The two "primary" buttons a provider card can carry, rendered once.
 *
 * Both were inline blocks copied between `ClaudeStyleCard` and
 * `CodexStyleCard` (the primary link), and between `ClaudeStyleCard` and the
 * compact card (the launch button). The card view's detail sidebar would have
 * been a third copy of each — and the whole premise of that sidebar is that it
 * offers the **same** actions as the card it opens, which is only true if there
 * is one renderer rather than three that agree today.
 *
 * Same reasoning as `cardChrome.ts` and `providerCardMenu.tsx` next to this
 * file: the shared chrome moves into the shared module the moment a second
 * consumer appears, or the copies drift and nothing fails.
 */

/** A footer action that launches the CLI with this provider's config. */
export interface ProviderCardLaunchAction {
  label: string;
  icon?: React.ReactNode;
  onClick: () => void;
  disabled?: boolean;
  loading?: boolean;
  /** Shown while `disabled`; the button explains itself on hover. */
  tooltip?: string;
}

/**
 * The card's primary action ("应用" / "设为默认") as a text link.
 *
 * Returns `null` for a missing action so callers can drop it straight into
 * JSX — a provider that is already applied has no primary action, and the
 * card simply renders one button fewer.
 */
export function renderProviderPrimaryAction(
  primary: ProviderCardActions['primaryAction'],
): React.ReactNode {
  if (!primary) {
    return null;
  }

  // `locked`: the action is unavailable but must stay visible — it explains
  // the blocker on hover instead of vanishing (a missing button reads as a
  // missing feature).
  if (primary.locked) {
    return (
      <Tooltip title={primary.tooltip}>
        <span>
          <Button type="link" size="small" icon={primary.icon} disabled>
            {primary.label}
          </Button>
        </span>
      </Tooltip>
    );
  }

  return (
    <Tooltip title={primary.tooltip}>
      <span>
        <Button
          type="link"
          size="small"
          icon={primary.icon}
          onClick={primary.onClick}
          disabled={primary.disabled}
          loading={primary.loading}
        >
          {primary.label}
        </Button>
      </span>
    </Tooltip>
  );
}

/**
 * The CLI-launch button.
 *
 * Quieter than the primary action (`type="text"`, smaller) because launching a
 * one-off CLI session is not the card's main job — applying the provider is.
 *
 * The font size is left to the surface rather than pinned here: the compact
 * card's footer runs this button at the Skills meta-bar scale while the detail
 * sidebar's status line runs it a step larger, and an inline size would win
 * over both.
 */
export function renderProviderLaunchAction(
  launchAction: ProviderCardLaunchAction | undefined,
): React.ReactNode {
  if (!launchAction) {
    return null;
  }

  return (
    <Tooltip title={launchAction.disabled ? launchAction.tooltip : ''}>
      {/* A disabled button swallows hover events, so the tooltip needs a
          wrapper element to attach to. */}
      <span>
        <Button
          type="text"
          size="small"
          icon={launchAction.icon}
          onClick={launchAction.onClick}
          disabled={launchAction.disabled}
          loading={launchAction.loading}
          style={{ padding: '0 6px', height: 24, flexShrink: 0 }}
        >
          {launchAction.label}
        </Button>
      </span>
    </Tooltip>
  );
}
