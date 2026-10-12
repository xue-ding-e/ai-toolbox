import type { ClaudeSettingsConfig } from '@/types/claudecode';

export const CLAUDE_ONE_M_MARKER = '[1M]';

export type ClaudeModelRole = 'sonnet' | 'opus' | 'fable' | 'haiku';

export interface ClaudeModelRoleConfig {
  role: ClaudeModelRole;
  model: string;
  displayName: string;
  supportsOneM: boolean;
}

export interface ClaudeProviderModelConfig {
  fallbackModel: string;
  roles: Record<ClaudeModelRole, ClaudeModelRoleConfig>;
  legacyReasoningModel: string;
}

export function hasClaudeOneMMarker(model: string): boolean {
  return model.trimEnd().toLowerCase().endsWith('[1m]');
}

export function stripClaudeOneMMarker(model: string): string {
  const trimmedModel = model.trimEnd();
  if (!trimmedModel.toLowerCase().endsWith('[1m]')) {
    return model;
  }
  return trimmedModel.slice(0, -CLAUDE_ONE_M_MARKER.length).trimEnd();
}

export function setClaudeOneMMarker(model: string, enabled: boolean): string {
  const baseModel = stripClaudeOneMMarker(model).trim();
  if (!baseModel) {
    return '';
  }
  return enabled ? `${baseModel}${CLAUDE_ONE_M_MARKER}` : baseModel;
}

export function parseClaudeSettingsConfig(rawConfig: string | undefined): ClaudeSettingsConfig {
  if (!rawConfig?.trim()) {
    return {};
  }

  try {
    const parsed = JSON.parse(rawConfig) as unknown;
    return typeof parsed === 'object' && parsed !== null && !Array.isArray(parsed)
      ? parsed as ClaudeSettingsConfig
      : {};
  } catch (error) {
    console.error('Failed to parse Claude settings config:', error);
    return {};
  }
}

export function getClaudeProviderModelConfig(
  settingsConfig: ClaudeSettingsConfig,
): ClaudeProviderModelConfig {
  const sonnetModel = readConfigModel(settingsConfig, 'sonnetModel', 'ANTHROPIC_DEFAULT_SONNET_MODEL');
  const opusModel = readConfigModel(settingsConfig, 'opusModel', 'ANTHROPIC_DEFAULT_OPUS_MODEL');
  const fableModel = readConfigModel(settingsConfig, 'fableModel', 'ANTHROPIC_DEFAULT_FABLE_MODEL');
  const haikuModel = readConfigModel(settingsConfig, 'haikuModel', 'ANTHROPIC_DEFAULT_HAIKU_MODEL');

  return {
    fallbackModel: readConfigModel(settingsConfig, 'model', 'ANTHROPIC_MODEL'),
    roles: {
      sonnet: {
        role: 'sonnet',
        model: sonnetModel,
        displayName: readEnvString(settingsConfig, 'ANTHROPIC_DEFAULT_SONNET_MODEL_NAME') ||
          stripClaudeOneMMarker(sonnetModel),
        supportsOneM: true,
      },
      opus: {
        role: 'opus',
        model: opusModel,
        displayName: readEnvString(settingsConfig, 'ANTHROPIC_DEFAULT_OPUS_MODEL_NAME') ||
          stripClaudeOneMMarker(opusModel),
        supportsOneM: true,
      },
      fable: {
        role: 'fable',
        model: fableModel,
        displayName: readEnvString(settingsConfig, 'ANTHROPIC_DEFAULT_FABLE_MODEL_NAME') ||
          stripClaudeOneMMarker(fableModel),
        supportsOneM: true,
      },
      haiku: {
        role: 'haiku',
        model: haikuModel,
        displayName: readEnvString(settingsConfig, 'ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME') ||
          stripClaudeOneMMarker(haikuModel),
        supportsOneM: true,
      },
    },
    legacyReasoningModel: readConfigModel(settingsConfig, 'reasoningModel', 'ANTHROPIC_REASONING_MODEL'),
  };
}

/**
 * The roles a compact provider card states on its second line, in the order it
 * states them.
 *
 * High tier first, because that is how these providers are compared. Only the
 * four role bindings — the provider-wide `ANTHROPIC_MODEL` catch-all and the
 * legacy reasoning slot are deliberately absent: the card is a glance, and the
 * entries that are not a tier would be the ones crowding out the tiers.
 */
export const CLAUDE_CARD_SUBTITLE_ROLES = ['opus', 'sonnet', 'haiku', 'fable'] as const;

export type ClaudeCardSubtitleRole = typeof CLAUDE_CARD_SUBTITLE_ROLES[number];

/**
 * The i18n key for each role's label, as a total map.
 *
 * Spelled out rather than built from the role name (`role${capitalize(role)}`)
 * so a rename here fails to compile instead of rendering a literal key string
 * on the card.
 */
export const CLAUDE_CARD_ROLE_LABELS: Record<ClaudeCardSubtitleRole, string> = {
  opus: 'claudecode.model.roleOpus',
  sonnet: 'claudecode.model.roleSonnet',
  haiku: 'claudecode.model.roleHaiku',
  fable: 'claudecode.model.roleFable',
};

/** One `Role: model` pair on the compact card's second line. */
export interface ClaudeCardSubtitle {
  role: ClaudeCardSubtitleRole;
  /** Display name, or the model id with its `[1M]` marker stripped. */
  model: string;
}

/**
 * Every role model a compact provider card states, in tier order.
 *
 * The list card shows these bindings plus the default and reasoning slots on
 * its meta line; the card grid shows only the tiers, on one truncated line.
 * Returning the list (rather than picking one) is what makes the line stable:
 * there is no "which role leads" preference to keep in sync with the store, and
 * no fallback chain to explain — either a role has a model or it is not on the
 * card.
 *
 * An empty array is a real answer, not a failure: a provider with no role
 * bindings has nothing to say here, and the card renders no second line.
 */
export function resolveClaudeCardSubtitles(
  modelConfig: ClaudeProviderModelConfig,
): ClaudeCardSubtitle[] {
  return CLAUDE_CARD_SUBTITLE_ROLES
    .map((role) => ({ role, model: modelConfig.roles[role].displayName.trim() }))
    .filter((entry) => Boolean(entry.model));
}

export function getClaudeConfiguredModelIds(
  settingsConfig: ClaudeSettingsConfig,
  options: { stripOneMMarker?: boolean; includeLegacyReasoning?: boolean } = {},
): string[] {
  const modelConfig = getClaudeProviderModelConfig(settingsConfig);
  const modelIds = [
    modelConfig.fallbackModel,
    modelConfig.roles.sonnet.model,
    modelConfig.roles.opus.model,
    modelConfig.roles.fable.model,
    modelConfig.roles.haiku.model,
    ...(options.includeLegacyReasoning === false ? [] : [modelConfig.legacyReasoningModel]),
  ];

  return Array.from(new Set(
    modelIds
      .map((modelId) => options.stripOneMMarker ? stripClaudeOneMMarker(modelId) : modelId)
      .map((modelId) => modelId.trim())
      .filter(Boolean),
  ));
}

function readConfigModel(
  settingsConfig: ClaudeSettingsConfig,
  legacyField: 'model' | 'haikuModel' | 'sonnetModel' | 'opusModel' | 'fableModel' | 'reasoningModel',
  envField: keyof NonNullable<ClaudeSettingsConfig['env']>,
): string {
  return readEnvString(settingsConfig, envField) || readTopLevelString(settingsConfig, legacyField);
}

function readTopLevelString(
  settingsConfig: ClaudeSettingsConfig,
  field: 'model' | 'haikuModel' | 'sonnetModel' | 'opusModel' | 'fableModel' | 'reasoningModel',
): string {
  const value = settingsConfig[field];
  return typeof value === 'string' ? value.trim() : '';
}

function readEnvString(
  settingsConfig: ClaudeSettingsConfig,
  field: keyof NonNullable<ClaudeSettingsConfig['env']>,
): string {
  const value = settingsConfig.env?.[field];
  return typeof value === 'string' ? value.trim() : '';
}
