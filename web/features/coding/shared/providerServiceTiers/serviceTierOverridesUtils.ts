/**
 * Provider-level per-model service (speed) tier override meta read/merge
 * helpers (issue #414).
 *
 * Mirrors the `providerModelRewrites` pattern: this feature subset owns its
 * `getServiceTierOverridesFromMeta` / `mergeServiceTierOverridesIntoMeta` pair
 * and only touches its own key, so it can be chained in a form's submit `meta:`
 * builder without clobbering other subsets. `mergeGatewayProfileReferenceIntoMeta`
 * does not delete `modelServiceTiers`, so switching a gateway profile preserves
 * the user's tier overrides.
 *
 * Persisted shape: `meta.modelServiceTiers: ModelServiceTierEntry[]`. The Rust
 * gateway parses the same array (`ModelServiceTierRule`) and, when the upstream
 * model matches, writes `service_tier` into an OpenAI Chat / Responses request
 * that did not already ask for a premium tier. See
 * `tauri/.../proxy_gateway/types.rs` and `.../runtime/upstream.rs`.
 */
export interface ModelServiceTierEntry {
  /** Upstream model to match (exact, trimmed, case-insensitive). */
  model: string;
  /**
   * Wire tier forwarded when the request asks for none. Codex's own ids:
   * `priority` (Fast) and `ultrafast`.
   */
  serviceTier: string;
}

/**
 * Tiers the picker offers. Proper nouns (the ids Codex itself sends), not
 * translatable copy — mirrors `CODEX_SERVICE_TIER_ENTRIES` in
 * `tauri/src/coding/codex/commands.rs`.
 */
export const SERVICE_TIER_OVERRIDE_OPTIONS: ReadonlyArray<{ value: string; label: string }> = [
  { value: 'priority', label: 'Fast' },
  { value: 'ultrafast', label: 'Ultrafast' },
];

export interface ServiceTierOverridesState {
  enabled: boolean;
  overrides: ModelServiceTierEntry[];
}

export interface GatewayProviderServiceTiersMeta {
  modelServiceTiers?: ModelServiceTierEntry[];
  /** Legacy snake_case variant, kept for parity with the Rust adapter. */
  model_service_tiers?: ModelServiceTierEntry[];
}

/** A fresh blank override row, defaulting to Fast (the common case). */
export function emptyServiceTierOverrideEntry(): ModelServiceTierEntry {
  return { model: '', serviceTier: 'priority' };
}

/**
 * Read the per-model tier overrides from provider meta into an editable state.
 * The state is `enabled` when any persisted row exists; an empty meta yields a
 * single blank row so the editor is immediately usable when toggled on.
 */
export function getServiceTierOverridesFromMeta(
  meta?: GatewayProviderServiceTiersMeta | null,
): ServiceTierOverridesState {
  const rawOverrides = Array.isArray(meta?.modelServiceTiers)
    ? meta.modelServiceTiers
    : Array.isArray(meta?.model_service_tiers)
      ? meta.model_service_tiers
      : [];
  const overrides = rawOverrides.length > 0 ? rawOverrides.map(normalizeEntry) : [];
  return {
    enabled: overrides.length > 0,
    overrides: overrides.length > 0 ? overrides : [emptyServiceTierOverrideEntry()],
  };
}

/** Merge the tier override state back into provider meta (field-level). */
export function mergeServiceTierOverridesIntoMeta<T extends GatewayProviderServiceTiersMeta>(
  meta: T | undefined,
  state: ServiceTierOverridesState,
): T | undefined {
  const nextMeta = { ...(meta || {}) } as T;
  const record = nextMeta as unknown as Record<string, unknown>;
  delete record.modelServiceTiers;
  delete record.model_service_tiers;

  if (state.enabled) {
    const meaningful = state.overrides
      .map((entry) => ({ model: entry.model.trim(), serviceTier: entry.serviceTier }))
      .filter((entry) => entry.model !== '' && entry.serviceTier !== '');
    if (meaningful.length > 0) {
      record.modelServiceTiers = meaningful;
    }
  }

  return hasMeaningfulMeta(nextMeta) ? nextMeta : undefined;
}

function normalizeEntry(entry: ModelServiceTierEntry): ModelServiceTierEntry {
  return {
    model: entry.model ?? '',
    serviceTier: entry.serviceTier ?? 'priority',
  };
}

function hasMeaningfulMeta(meta: GatewayProviderServiceTiersMeta): boolean {
  return Object.values(meta).some((value) => {
    if (value === undefined || value === null || value === '') return false;
    if (Array.isArray(value)) return value.length > 0;
    return true;
  });
}