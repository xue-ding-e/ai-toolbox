import type { ClaudeSettingsConfig } from '@/types/claudecode';

export const CLAUDE_ONE_M_MARKER = '[1M]';

export type ClaudeModelRole = 'sonnet' | 'opus' | 'fable' | 'haiku' | 'subagent' | 'custom';

export interface ClaudeModelRoleConfig {
  role: ClaudeModelRole;
  model: string;
  displayName: string;
  supportsOneM: boolean;
  /**
   * Whether Claude Code has a display-name variable for this binding.
   *
   * Subagent is the one slot without one (`CLAUDE_CODE_SUBAGENT_MODEL` has no
   * `_NAME` companion), so its row shows a read-only placeholder instead of an
   * input — an editable box would write a variable Claude Code never reads.
   */
  hasDisplayName: boolean;
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
  // Subagent and custom have no legacy top-level spelling: both arrived after
  // the env-only model fields did.
  const subagentModel = readEnvString(settingsConfig, 'CLAUDE_CODE_SUBAGENT_MODEL');
  const customModel = readEnvString(settingsConfig, 'ANTHROPIC_CUSTOM_MODEL_OPTION');

  return {
    fallbackModel: readConfigModel(settingsConfig, 'model', 'ANTHROPIC_MODEL'),
    roles: {
      sonnet: {
        role: 'sonnet',
        model: sonnetModel,
        displayName: readEnvString(settingsConfig, 'ANTHROPIC_DEFAULT_SONNET_MODEL_NAME') ||
          stripClaudeOneMMarker(sonnetModel),
        supportsOneM: true,
        hasDisplayName: true,
      },
      opus: {
        role: 'opus',
        model: opusModel,
        displayName: readEnvString(settingsConfig, 'ANTHROPIC_DEFAULT_OPUS_MODEL_NAME') ||
          stripClaudeOneMMarker(opusModel),
        supportsOneM: true,
        hasDisplayName: true,
      },
      fable: {
        role: 'fable',
        model: fableModel,
        displayName: readEnvString(settingsConfig, 'ANTHROPIC_DEFAULT_FABLE_MODEL_NAME') ||
          stripClaudeOneMMarker(fableModel),
        supportsOneM: true,
        hasDisplayName: true,
      },
      haiku: {
        role: 'haiku',
        model: haikuModel,
        displayName: readEnvString(settingsConfig, 'ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME') ||
          stripClaudeOneMMarker(haikuModel),
        supportsOneM: true,
        hasDisplayName: true,
      },
      subagent: {
        role: 'subagent',
        model: subagentModel,
        displayName: '',
        supportsOneM: true,
        hasDisplayName: false,
      },
      custom: {
        role: 'custom',
        model: customModel,
        displayName: readEnvString(settingsConfig, 'ANTHROPIC_CUSTOM_MODEL_OPTION_NAME') ||
          stripClaudeOneMMarker(customModel),
        supportsOneM: true,
        hasDisplayName: true,
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
 * Every binding the list card's meta line and the detail sidebar enumerate, in
 * the order they state them.
 *
 * Wider than `CLAUDE_CARD_SUBTITLE_ROLES` on purpose: those two surfaces have
 * room to name all six bindings, so a model a user configured must not be
 * invisible there just because it is not a tier. Built as a list here rather
 * than spelled out at each call site so a role added to `ClaudeModelRole`
 * cannot be silently left off one of them.
 */
export const CLAUDE_DETAIL_BINDING_ROLES = [
  'haiku',
  'sonnet',
  'opus',
  'fable',
  'subagent',
  'custom',
] as const satisfies readonly ClaudeModelRole[];

/**
 * The i18n key for each detail binding's label, as a total map.
 *
 * Spelled out rather than derived from the role name for the same reason as
 * `CLAUDE_CARD_ROLE_LABELS`: a rename fails to compile instead of rendering a
 * literal key on the panel. Note this is the *role* name, not the binding's
 * display name — Subagent has no display-name variable, so its display name is
 * always empty and would leave the row unlabelled.
 */
export const CLAUDE_DETAIL_BINDING_LABELS: Record<ClaudeModelRole, string> = {
  sonnet: 'claudecode.model.roleSonnet',
  opus: 'claudecode.model.roleOpus',
  fable: 'claudecode.model.roleFable',
  haiku: 'claudecode.model.roleHaiku',
  subagent: 'claudecode.model.roleSubagent',
  custom: 'claudecode.model.roleCustom',
};

/**
 * The raw model id configured for each binding, in `CLAUDE_DETAIL_BINDING_ROLES`
 * order, skipping the bindings that have no model.
 *
 * Reads `.model` rather than `.displayName`: these lists state what will be
 * sent upstream, so the configured id is the fact worth naming. The `[1M]`
 * marker stays on it — it is part of the stored value and these surfaces are
 * where a user checks whether it is set.
 */
export function resolveClaudeDetailBindings(
  modelConfig: ClaudeProviderModelConfig,
): Array<{ role: ClaudeModelRole; model: string }> {
  return CLAUDE_DETAIL_BINDING_ROLES
    .map((role) => ({ role, model: modelConfig.roles[role].model.trim() }))
    .filter((entry) => Boolean(entry.model));
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
    modelConfig.roles.subagent.model,
    modelConfig.roles.custom.model,
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
