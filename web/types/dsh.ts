// DeepSeek Harness (dsh) runtime configuration types.
//
// The dsh backend manages `~/.dsh/settings.yaml` and `~/.dsh/.credentials.yaml`.
// Provider definitions live under `llm-pi-ai.providers.<route>`, credentials are
// stored in the separate `.credentials.yaml` (REF -> secret), and the global
// prompt is `~/.dsh/AGENTS.md`. All serde-exposed view fields use camelCase.

export interface DshPathInfo {
  path: string;
  source: 'custom' | 'env' | 'shell' | 'default';
}

// Backend `get_dsh_settings_config` returns the stored config dir under
// camelCase `configDir`. `DshSettingsConfigInput` mirrors the shared
// RootDirectoryModal contract ({ config/rootDir/clearRootDir }); the page maps
// between the two (see DshPage getCommonConfig).
export interface DshSettingsConfig {
  configDir?: string;
  /** Pinned profile whose `cordis.patch.yml` backs the editor. */
  profile?: string;
  updatedAt?: string;
}

export interface DshSettingsConfigInput {
  config: string;
  rootDir?: string | null;
  clearRootDir?: boolean;
  /** Pin a profile; `''` clears the pin (back to auto-detection); omit to keep it. */
  profile?: string;
}

export type DshDeleteScope = 'credential' | 'provider' | 'both';
export type DshProviderWarning = 'missingProvider' | 'missingModel';

export interface DshModelSettings {
  provider?: string | null;
  model?: string | null;
  reasoningEffort?: string | null;
}

export interface DshModelSettingsInput {
  provider?: string | null;
  model?: string | null;
  reasoningEffort?: string | null;
}

export interface DshRuntimeProviderView {
  providerKey: string;
  displayName: string;
  /** Credential env-var ref name stored on the provider. */
  apiKeyEnv?: string;
  /** Whether a matching `REF` entry exists in `.credentials.yaml`. */
  credentialExists: boolean;
  /** Resolved credential value from `.credentials.yaml` (empty when absent). */
  apiKey?: string;
  /** Wire protocol for this route (`openai-completions` / `openai-responses` / `anthropic-messages`). */
  api?: string;
  /** Raw provider dict entry from `llm-pi-ai.providers.<route>` (e.g. `displayName`, `api`, `baseURL`, `apiKeyEnv`, `models`, `modelOverrides`). */
  provider?: Record<string, unknown>;
  modelIds: string[];
  /** Where the served models come from. `builtin` = no explicit `models`, so the route serves the adapter's default catalog. */
  modelSource?: 'explicit' | 'builtin';
  /** Adapter default model records (dsh model schema shape) when `modelSource` is `builtin`; these are not written into settings.yaml. */
  builtinModels?: Array<Record<string, unknown>>;
  isBuiltin: boolean;
  isDefault: boolean;
  warnings: DshProviderWarning[];
}

export interface DshCredentialView {
  refName: string;
  hasValue: boolean;
}

export interface DshBuiltinProvider {
  key: string;
  name: string;
}

export interface DshRuntimeConfig {
  rootPathInfo: DshPathInfo;
  /** Writable layer: `profiles/<profile>/cordis.patch.yml`, or legacy `settings.yaml`. */
  configPath: string;
  credentialsPath: string;
  promptPath: string;
  /** Profile whose patch layer backs the editor (`undefined` = legacy settings.yaml). */
  profile?: string;
  /** Profiles present under `profiles/`, for the profile picker. */
  profiles: string[];
  /** dsh imported the legacy `settings.yaml` into the profile patch layer. */
  migratedFromSettings: boolean;
  /** Legacy `settings.yaml` path, shown as migration provenance. */
  settingsYamlPath: string;
  /** Managed sections (`llm-pi-ai`, `agent-default-model`) of the active layer. */
  config: Record<string, unknown>;
  modelSettings: DshModelSettingsInput;
  /** Legacy "other settings"; empty on a profile layer (owned by dsh's own UI). */
  otherSettings: Record<string, unknown>;
  providers: DshRuntimeProviderView[];
  builtinProviders: DshBuiltinProvider[];
  credentials: DshCredentialView[];
  /** Raw `settings.yaml` file content for file-based preview. */
  configContent?: string | null;
  /** Raw `.credentials.yaml` file content for file-based preview. */
  credentialsContent?: string | null;
  /** Raw `AGENTS.md` file content for file-based preview. */
  promptContent?: string | null;
  /** Home-level `cordis.patch.yml` path (AI Toolbox-managed MCP plugin layer). */
  cordisPatchPath: string;
  /** Raw home-level `cordis.patch.yml` content for file-based preview. */
  cordisPatchContent?: string | null;
}

export interface DshModelsProviderInput {
  providerKey: string;
  provider: Record<string, unknown>;
}

/** Backend `save_dsh_credential` writes `REF: value` into `.credentials.yaml`. */
export interface DshCredentialInput {
  refName: string;
  value: string;
}