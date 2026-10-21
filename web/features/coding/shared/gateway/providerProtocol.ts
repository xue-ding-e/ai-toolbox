import type { GatewayCliTakeoverStatus } from '@/services';
import { parse as parseToml } from 'smol-toml';

export type GatewayApiFormat =
  | 'anthropic_messages'
  | 'openai_responses'
  | 'openai_chat'
  | 'gemini_native';

const normalizeFormatKey = (value: string) =>
  value.trim().toLowerCase().replace(/[/-]/g, '_');

export const normalizeGatewayApiFormat = (
  value?: string | null,
): GatewayApiFormat | null => {
  if (!value) {
    return null;
  }

  switch (normalizeFormatKey(value)) {
    case 'anthropic':
    case 'anthropic_messages':
    case 'claude':
    case 'claude_messages':
      return 'anthropic_messages';
    case 'openai_responses':
    case 'responses':
    case 'response':
      return 'openai_responses';
    case 'openai_chat':
    case 'chat_completions':
    case 'chat':
      return 'openai_chat';
    case 'gemini_native':
    case 'gemini':
      return 'gemini_native';
    default:
      return null;
  }
};

export const firstGatewayApiFormat = (
  ...values: Array<string | null | undefined>
): GatewayApiFormat | null => {
  for (const value of values) {
    const normalized = normalizeGatewayApiFormat(value);
    if (normalized) {
      return normalized;
    }
  }
  return null;
};

export const providerNeedsGatewayProxy = (
  targetFormat: string | null | undefined,
  nativeFormat: string,
) => {
  const normalizedTargetFormat = normalizeGatewayApiFormat(targetFormat);
  const normalizedNativeFormat = normalizeGatewayApiFormat(nativeFormat);
  return Boolean(
    normalizedTargetFormat &&
    normalizedNativeFormat &&
    normalizedTargetFormat !== normalizedNativeFormat,
  );
};

/**
 * Grok CLI natively supports openai_responses, openai_chat (chat_completions)
 * and anthropic_messages. Only Gemini Native still needs Gateway conversion.
 * Do NOT compare against a single native format like Codex/Claude.
 */
export const grokProviderNeedsGatewayProxy = (
  targetFormat: string | null | undefined,
) => normalizeGatewayApiFormat(targetFormat) === 'gemini_native';

export const isGatewayConfigFlagEnabled = (value: unknown) => {
  if (typeof value === 'boolean') {
    return value;
  }
  if (typeof value === 'number') {
    return value !== 0;
  }
  if (typeof value === 'string') {
    const normalizedValue = value.trim().toLowerCase();
    return normalizedValue === 'true' ||
      normalizedValue === '1' ||
      normalizedValue === 'yes' ||
      normalizedValue === 'on';
  }
  return false;
};

export const canApplyProviderWithGatewayProxy = (
  status?: GatewayCliTakeoverStatus | null,
) => Boolean(status?.can_takeover);

// ---- Gateway proxy mode guards ----
//
// `aggregate` is a third takeover mode: it keeps every selected site as a
// candidate and routes by the `<site_id><sep><model>` prefix. It is NOT a
// failover variant, so keep the two predicates separate instead of folding
// aggregate into a failover check.

/** Any mode that rewrites the CLI runtime config and routes through the gateway. */
export const isGatewayProxyMode = (
  mode?: GatewayCliTakeoverStatus['mode'] | null,
): boolean => mode === 'single' || mode === 'failover' || mode === 'aggregate';

/** Failover specifically: P0 pinned first, P1+ tried in provider order. */
export const isGatewayFailoverMode = (
  mode?: GatewayCliTakeoverStatus['mode'] | null,
): boolean => mode === 'failover';

/** Aggregate specifically: cross-site model list with per-site model prefixes. */
export const isGatewayAggregateMode = (
  mode?: GatewayCliTakeoverStatus['mode'] | null,
): boolean => mode === 'aggregate';

export const codexWireApiFormatFromConfig = (config?: string | null) => {
  if (!config) {
    return null;
  }

  try {
    const parsed = parseToml(config) as Record<string, unknown>;
    const selectedProvider = typeof parsed.model_provider === 'string'
      ? parsed.model_provider.trim()
      : '';
    const modelProviders = parsed.model_providers && typeof parsed.model_providers === 'object'
      && !Array.isArray(parsed.model_providers)
      ? parsed.model_providers as Record<string, unknown>
      : undefined;
    const provider = selectedProvider && modelProviders?.[selectedProvider]
      && typeof modelProviders[selectedProvider] === 'object'
      && !Array.isArray(modelProviders[selectedProvider])
      ? modelProviders[selectedProvider] as Record<string, unknown>
      : undefined;
    const selectedValue = provider?.wire_api ?? provider?.api_format;
    if (typeof selectedValue === 'string' && selectedValue.trim()) {
      return selectedValue.trim();
    }
    const rootValue = parsed.wire_api ?? parsed.api_format;
    if (typeof rootValue === 'string' && rootValue.trim()) {
      return rootValue.trim();
    }
    for (const value of Object.values(modelProviders ?? {})) {
      if (!value || typeof value !== 'object' || Array.isArray(value)) continue;
      const providerValue = (value as Record<string, unknown>).wire_api
        ?? (value as Record<string, unknown>).api_format;
      if (typeof providerValue === 'string' && providerValue.trim()) {
        return providerValue.trim();
      }
    }
    return null;
  } catch {
    const match = config.match(/^\s*(?:wire_api|api_format)\s*=\s*["']([^"']+)["']/m);
    return match?.[1] ?? null;
  }
};

export const grokWireApiFormatFromConfig = (config?: string | null) => {
  if (!config) {
    return null;
  }

  try {
    const parsed = parseToml(config) as Record<string, unknown>;
    const models = parsed.models && typeof parsed.models === 'object' && !Array.isArray(parsed.models)
      ? parsed.models as Record<string, unknown>
      : undefined;
    const defaultModelKey = typeof models?.default === 'string' ? models.default.trim() : '';
    const modelTables = parsed.model && typeof parsed.model === 'object' && !Array.isArray(parsed.model)
      ? parsed.model as Record<string, unknown>
      : undefined;
    const selectedModel = defaultModelKey && modelTables?.[defaultModelKey]
      && typeof modelTables[defaultModelKey] === 'object' && !Array.isArray(modelTables[defaultModelKey])
      ? modelTables[defaultModelKey] as Record<string, unknown>
      : undefined;
    const selectedValue = selectedModel?.api_backend ?? selectedModel?.api_format;
    if (typeof selectedValue === 'string' && selectedValue.trim()) {
      return selectedValue.trim();
    }
    return null;
  } catch {
    const match = config.match(/^\s*(?:api_backend|api_format)\s*=\s*["']([^"']+)["']/m);
    return match?.[1] ?? null;
  }
};

export const openAiApiFormatFromBaseUrl = (baseUrl?: string | null) => {
  const normalizedBaseUrl = baseUrl?.trim().toLowerCase();
  if (!normalizedBaseUrl) {
    return null;
  }
  return normalizedBaseUrl.endsWith('/chat/completions') ||
    normalizedBaseUrl.includes('/chat/completions?')
    ? 'openai_chat'
    : null;
};

// ---- Claude-safe model id checks (mirror of backend config_writer.rs) ----

const CLAUDE_ROUTE_PREFIX = 'claude-';
const ANTHROPIC_CLAUDE_ROUTE_PREFIX = 'anthropic/claude-';
const ONE_M_CONTEXT_MARKER = '[1m]';

/** Mirror of the backend `is_claude_safe_model_id` (config_writer.rs): Claude
 * Desktop's 3P profile only accepts `claude-<role>-<id>` (optionally prefixed
 * with `anthropic/`) and rejects `[1m]` markers or degraded ids like
 * `claude-sonnet-`. */
export const isClaudeSafeModelId = (model: string): boolean => {
  const normalized = model.trim().toLowerCase();
  if (normalized.includes(ONE_M_CONTEXT_MARKER)) {
    return false;
  }
  const routeTail = normalized.startsWith(ANTHROPIC_CLAUDE_ROUTE_PREFIX)
    ? normalized.slice(ANTHROPIC_CLAUDE_ROUTE_PREFIX.length)
    : normalized.startsWith(CLAUDE_ROUTE_PREFIX)
      ? normalized.slice(CLAUDE_ROUTE_PREFIX.length)
      : '';
  return ['sonnet-', 'opus-', 'haiku-', 'fable-'].some((prefix) =>
    routeTail.startsWith(prefix) && routeTail.length > prefix.length,
  );
};

/** True when any configured model name is not a claude-* id Claude Desktop
 * accepts directly; such a provider needs local gateway routing. */
export const hasNonClaudeModelIds = (modelIds: string[]): boolean =>
  modelIds.some((modelId) => !isClaudeSafeModelId(modelId));

/**
 * Whether a provider's role routes need the local gateway.
 *
 * Mirrors the backend's `has_routing_models` (claude_desktop
 * `config_writer.rs`): Direct mode forbids model mapping, so a provider whose
 * upstream model differs from its route id — `claude-sonnet-5` mapped to
 * `claude-opus-5`, say — cannot be written directly even though both names are
 * claude-safe. Checking only `hasNonClaudeModelIds` read such a provider as
 * "direct", and its 应用 button silently wrote a gateway profile without the
 * gateway being engaged (user-reported 2026-10-21, AnyRouter).
 */
export const hasClaudeDesktopRoutingModels = (
  routes: Record<string, { model?: string }> | null | undefined,
): boolean =>
  Boolean(routes) &&
  Object.entries(routes ?? {}).some(([routeId, route]) => {
    const model = route?.model?.trim() ?? '';
    return model !== '' && model !== routeId.trim();
  });

export type GatewayProxyReason = 'protocol' | 'model' | 'both' | null;

/** Classify why a provider needs the local gateway: a protocol mismatch, an
 * un-safe model id (Claude Desktop only), or both. Returns null when the
 * provider can run in direct mode. */
export const gatewayProxyReason = (
  targetFormat: string | null | undefined,
  nativeFormat: string,
  modelIds?: string[],
): GatewayProxyReason => {
  const protocolMismatch = providerNeedsGatewayProxy(targetFormat, nativeFormat);
  const modelMismatch = modelIds ? hasNonClaudeModelIds(modelIds) : false;
  if (!protocolMismatch && !modelMismatch) {
    return null;
  }
  if (protocolMismatch && modelMismatch) {
    return 'both';
  }
  return protocolMismatch ? 'protocol' : 'model';
};

/** i18n key for the "restore direct unavailable" tooltip/hint, chosen by the
 * underlying reason. Falls back to the protocol hint when reason is null. */
export const restoreDirectUnavailableHintKey = (
  reason: GatewayProxyReason,
): string => {
  if (reason === 'both') {
    return 'gateway.proxy.restoreDirectUnavailableHintBoth';
  }
  if (reason === 'model') {
    return 'gateway.proxy.restoreDirectUnavailableHintModel';
  }
  return 'gateway.proxy.restoreDirectUnavailableHintProtocol';
};
