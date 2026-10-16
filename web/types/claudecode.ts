/**
 * Claude Code Configuration Types
 *
 * Type definitions for Claude Code configuration management.
 */

import type { CustomHeaderEntry } from '@/features/coding/shared/providerHeaders/customHeadersUtils';
import type { ModelRewriteEntry } from '@/features/coding/shared/providerModelRewrites/modelRewritesUtils';

export type ClaudeProviderCategory = 'official' | 'third_party' | 'custom';
export type ClaudeApiFormat = 'anthropic' | 'openai_chat' | 'openai_responses' | 'gemini_native';
export type ClaudeSettingsMergeStrategy =
  | 'provider_overrides_common'
  | 'common_overrides_provider'
  | 'merge_common_and_provider';

export const DEFAULT_CLAUDE_SETTINGS_MERGE_STRATEGY: ClaudeSettingsMergeStrategy =
  'provider_overrides_common';

export interface GatewayProviderProfileReference {
  tool?: 'claude' | 'codex' | 'grok' | 'gemini';
  profileId: string;
  endpointId: string;
}

export interface GatewayProviderMeta {
  gatewayProfile?: GatewayProviderProfileReference;
  providerType?: string;
  apiFormat?: ClaudeApiFormat | string;
  apiKeyField?: string;
  isFullUrl?: boolean;
  promptCacheKey?: string;
  reasoningField?: 'reasoning_content' | 'content' | 'reasoning' | 'none' | 'all' | string;
  defaultMaxTokens?: number;
  imageInputPolicy?: 'auto' | 'preserve' | 'strip' | 'text_only' | string;
  textOnlyModels?: string[];
  imageCapableModels?: string[];
  allowTextOnlyModelHeuristic?: boolean;
  costMultiplier?: string;
  pricingModelSource?: 'upstream' | 'requested' | string;
  /** Provider-level custom request-header overrides applied by the gateway on upstream requests. */
  customHeaders?: CustomHeaderEntry[];
  /** Provider-level exact model rewrite rules applied by the gateway (issue #321). */
  modelRewrites?: ModelRewriteEntry[];
}

/**
 * Claude Code Provider settings configuration
 * Maps to the settings.json env section
 */
export interface ClaudeSettingsConfig {
  env?: {
    ANTHROPIC_AUTH_TOKEN?: string;
    ANTHROPIC_API_KEY?: string; // 兼容旧版本，读取时检查，写入时不使用
    ANTHROPIC_BASE_URL?: string;
    ANTHROPIC_MODEL?: string;
    ANTHROPIC_DEFAULT_HAIKU_MODEL?: string;
    ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME?: string;
    ANTHROPIC_DEFAULT_SONNET_MODEL?: string;
    ANTHROPIC_DEFAULT_SONNET_MODEL_NAME?: string;
    ANTHROPIC_DEFAULT_OPUS_MODEL?: string;
    ANTHROPIC_DEFAULT_OPUS_MODEL_NAME?: string;
    ANTHROPIC_DEFAULT_FABLE_MODEL?: string;
    ANTHROPIC_DEFAULT_FABLE_MODEL_NAME?: string;
    CLAUDE_CODE_SUBAGENT_MODEL?: string;
    ANTHROPIC_CUSTOM_MODEL_OPTION?: string;
    ANTHROPIC_CUSTOM_MODEL_OPTION_NAME?: string;
    ANTHROPIC_REASONING_MODEL?: string;
  };
  // Legacy model configurations. New writes should use env.ANTHROPIC_* fields.
  model?: string;
  haikuModel?: string;
  sonnetModel?: string;
  opusModel?: string;
  fableModel?: string;
  reasoningModel?: string;
}

/**
 * Claude Code Provider stored in database
 */
export interface ClaudeCodeProvider {
  id: string;
  name: string;
  category: ClaudeProviderCategory;
  settingsConfig: string; // JSON string of ClaudeSettingsConfig
  extraSettingsConfig: string; // JSON string of additional settings.json fields for custom providers
  extraSettingsMergeStrategy: ClaudeSettingsMergeStrategy;
  // Source info if imported from settings
  sourceProviderId?: string;
  // Metadata
  websiteUrl?: string;
  notes?: string;
  icon?: string;
  iconColor?: string;
  sortIndex?: number;
  meta?: GatewayProviderMeta;
  isApplied?: boolean;
  isDisabled?: boolean;
  createdAt: string;
  updatedAt: string;
}

/**
 * Common configuration that applies to all providers
 * Stored as a single record in database
 */
export interface ClaudeCommonConfig {
  config: string; // JSON string like '{ "statusLine": {...}, "skipWebFetchPreflight": true }'
  rootDir?: string | null;
  updatedAt?: string;
}

export interface ConfigPathInfo {
  path: string;
  source: 'custom' | 'env' | 'shell' | 'default';
}

/**
 * Claude Code settings.json file structure
 * Note: Due to #[serde(flatten)] in Rust, other fields are flattened at the top level
 */
export interface ClaudeSettings {
  env?: {
    ANTHROPIC_AUTH_TOKEN?: string;
    ANTHROPIC_API_KEY?: string; // 兼容旧版本
    ANTHROPIC_BASE_URL?: string;
    ANTHROPIC_MODEL?: string;
    ANTHROPIC_DEFAULT_HAIKU_MODEL?: string;
    ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME?: string;
    ANTHROPIC_DEFAULT_SONNET_MODEL?: string;
    ANTHROPIC_DEFAULT_SONNET_MODEL_NAME?: string;
    ANTHROPIC_DEFAULT_OPUS_MODEL?: string;
    ANTHROPIC_DEFAULT_OPUS_MODEL_NAME?: string;
    ANTHROPIC_DEFAULT_FABLE_MODEL?: string;
    ANTHROPIC_DEFAULT_FABLE_MODEL_NAME?: string;
    CLAUDE_CODE_SUBAGENT_MODEL?: string;
    ANTHROPIC_CUSTOM_MODEL_OPTION?: string;
    ANTHROPIC_CUSTOM_MODEL_OPTION_NAME?: string;
    ANTHROPIC_REASONING_MODEL?: string;
  };
  // Common config fields (flattened at top level)
  [key: string]: unknown;
}

/**
 * Form values for creating/editing a provider
 */
export interface ClaudeProviderFormValues {
  name: string;
  category: ClaudeProviderCategory;
  providerEndpointKey?: string;
  providerProfileId?: string;
  providerEndpointId?: string;
  baseUrl?: string;
  apiKey?: string;
  model?: string;
  haikuModel?: string;
  haikuModelName?: string;
  sonnetModel?: string;
  sonnetModelName?: string;
  opusModel?: string;
  opusModelName?: string;
  fableModel?: string;
  fableModelName?: string;
  subagentModel?: string;
  customModel?: string;
  customModelName?: string;
  reasoningModel?: string; // Legacy only; new provider form no longer writes it.
  extraSettingsConfig?: string;
  extraSettingsMergeStrategy?: ClaudeSettingsMergeStrategy;
  meta?: GatewayProviderMeta;
  apiFormat?: ClaudeApiFormat;
  notes?: string;
  isDisabled?: boolean;
  // For import from settings
  sourceProviderId?: string;
}

/**
 * Provider input for saving local config
 */
export interface ClaudeProviderInput {
  name: string;
  category: ClaudeProviderCategory;
  settingsConfig: string;
  extraSettingsConfig?: string;
  extraSettingsMergeStrategy?: ClaudeSettingsMergeStrategy;
  sourceProviderId?: string;
  websiteUrl?: string;
  notes?: string;
  icon?: string;
  iconColor?: string;
  sortIndex?: number;
  meta?: GatewayProviderMeta;
}

/**
 * Local config save input
 */
export interface ClaudeLocalConfigInput {
  provider?: ClaudeProviderInput;
  commonConfig?: string;
  rootDir?: string | null;
  clearRootDir?: boolean;
}

export interface ClaudeCommonConfigInput {
  config: string;
  rootDir?: string | null;
  clearRootDir?: boolean;
}

/**
 * Import conflict action
 */
export type ImportConflictAction = 'overwrite' | 'duplicate' | 'cancel';

/**
 * Import conflict info
 */
export interface ImportConflictInfo {
  existingProvider: ClaudeCodeProvider;
  newProviderName: string;
  sourceProviderId: string;
}

/**
 * Claude Plugin integration status
 */
export interface ClaudePluginStatus {
  enabled: boolean;       // Whether primaryApiKey = "any" is set
  hasConfigFile: boolean; // Whether ~/.claude/config.json exists
}

export interface ClaudePluginRuntimeStatus {
  mode: 'local' | 'wslDirect';
  source: 'custom' | 'env' | 'shell' | 'default';
  rootDir: string;
  settingsPath: string;
  pluginsDir: string;
  distro?: string;
  linuxRootDir?: string;
}

export interface ClaudeKnownMarketplace {
  name: string;
  source: unknown;
  installLocation?: string;
  lastUpdated?: string;
  autoUpdateEnabled: boolean;
  owner?: {
    name?: string;
    email?: string;
  };
  description?: string;
  version?: string;
  pluginCount: number;
}

export interface ClaudeMarketplacePlugin {
  marketplaceName: string;
  name: string;
  description?: string;
  version?: string;
  homepage?: string;
  repository?: string;
  category?: string;
  tags: string[];
  source: unknown;
  pluginId: string;
}

export interface ClaudeInstalledPlugin {
  pluginId: string;
  name: string;
  marketplaceName: string;
  description?: string;
  version?: string;
  homepage?: string;
  repository?: string;
  installPath?: string;
  userScopeInstalled: boolean;
  userScopeEnabled: boolean;
  installScopes: string[];
  hasSkills: boolean;
  hasAgents: boolean;
  hasHooks: boolean;
  hasMcpServers: boolean;
  hasLspServers: boolean;
}

export interface ClaudeMarketplaceAddInput {
  source: string;
}

export interface ClaudeMarketplaceUpdateInput {
  marketplaceName?: string;
}

export interface ClaudeMarketplaceAutoUpdateInput {
  marketplaceName: string;
  autoUpdateEnabled: boolean;
}

export interface ClaudeMarketplaceRemoveInput {
  marketplaceName: string;
}

export interface ClaudePluginActionInput {
  pluginId: string;
}

export interface ClaudePluginBulkActionInput {
  enabled: boolean;
}

export interface ClaudePluginBulkActionResult {
  updatedCount: number;
}
