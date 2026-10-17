import type { CcSwitchProviderCandidate } from '@/services/ccSwitchApi';
import type { OpenCodeAllApiHubProvider } from '@/services/opencodeApi';
import type { ZcodeSettingsConfig } from '@/types/zcode';

import { buildZcodeProviderId } from './zcodeSettingsConfig.ts';

/**
 * A provider pulled from an external source, reduced to what ZCode stores.
 *
 * Every import source carries the same three facts in a different envelope
 * (CC Switch nests them under `env`, All API Hub under `providerConfig.options`),
 * so each extractor ends here and one builder owns the ZCode shape.
 */
export interface ZcodeImportedProvider {
  name: string;
  /** API format the source speaks; ZCode stores this verbatim. */
  apiType: string;
  baseUrl?: string;
  apiKey?: string;
  notes?: string;
}

/**
 * CC Switch's `settingsConfig` is a Claude-format blob whose credentials live
 * under `env`. Returns `null` when it holds neither endpoint nor key — nothing
 * usable to import.
 */
export function extractZcodeProviderFromCcSwitch(
  candidate: CcSwitchProviderCandidate,
): ZcodeImportedProvider | null {
  let env: Record<string, unknown> = {};
  try {
    const settings =
      typeof candidate.settingsConfig === 'string'
        ? JSON.parse(candidate.settingsConfig)
        : candidate.settingsConfig;
    if (settings?.env && typeof settings.env === 'object') {
      env = settings.env as Record<string, unknown>;
    }
  } catch {
    return null;
  }

  const baseUrl = typeof env.ANTHROPIC_BASE_URL === 'string' ? env.ANTHROPIC_BASE_URL : undefined;
  const apiKey =
    (typeof env.ANTHROPIC_AUTH_TOKEN === 'string' ? env.ANTHROPIC_AUTH_TOKEN : undefined) ||
    (typeof env.ANTHROPIC_API_KEY === 'string' ? env.ANTHROPIC_API_KEY : undefined);
  if (!baseUrl && !apiKey) {
    return null;
  }

  return {
    name: candidate.name,
    // CC Switch only stores Claude-shaped providers, and its blob carries no
    // format field — Anthropic Messages is what these are.
    apiType: 'anthropic-messages',
    baseUrl,
    apiKey,
    notes: candidate.notes,
  };
}

/** All API Hub exposes an OpenAI-compatible endpoint plus its key. */
export function extractZcodeProviderFromAllApiHub(
  item: OpenCodeAllApiHubProvider,
): ZcodeImportedProvider | null {
  const options = item.providerConfig?.options ?? {};
  const baseUrl = typeof options.baseURL === 'string' ? options.baseURL.trim() : '';
  const apiKey = typeof options.apiKey === 'string' ? options.apiKey.trim() : '';
  if (!baseUrl && !apiKey) {
    return null;
  }

  return {
    name: item.name,
    // All API Hub aggregates OpenAI-compatible relays; an unknown format would
    // be rejected, so this is the format to assume.
    apiType: 'openai-chat-completions',
    ...(baseUrl ? { baseUrl } : {}),
    ...(apiKey ? { apiKey } : {}),
  };
}

/**
 * Builds the provider's stored `settingsConfig`.
 *
 * Both import sources hand over an endpoint and a key, never a catalog, so the
 * model list starts empty and the user fills it with 获取模型 — the same place
 * every other CLI's import leaves them.
 *
 * The provider id is derived from the name here so the row's identity is known
 * before it is written: the backend reuses it as the row id (with a uniqueness
 * suffix when the name is taken), which is what keeps "set as default" able to
 * find the registry entry. The backend still fills a blank id as a guard for
 * rows whose settings predate this.
 */
export function buildZcodeSettingsConfigFromImport(imported: ZcodeImportedProvider): string {
  const settings: ZcodeSettingsConfig = {
    providerId: buildZcodeProviderId(imported.name),
    providerName: imported.name,
    config: {
      access: {
        type: 'api-key',
        ...(imported.apiKey ? { apiKey: imported.apiKey } : {}),
      },
      api: {
        type: imported.apiType,
        ...(imported.baseUrl ? { baseUrl: imported.baseUrl } : {}),
      },
    },
    models: [],
  };

  return JSON.stringify(settings);
}