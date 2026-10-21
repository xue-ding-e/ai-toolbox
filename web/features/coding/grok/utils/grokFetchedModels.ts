import type { FetchedModel } from '../../../../components/common/FetchModelsModal/types.ts';
import type { PresetModel } from '../../../../constants/presetModels.ts';
import { findPresetModelById, PRESET_MODELS } from '../../../../constants/presetModels.ts';
import type { GrokApiFormat, GrokCatalogModel } from '../../../../types/grok.ts';
import {
  extractReasoningEffortsFromPresetVariants,
  getGrokPrimaryPresetNpmTypes,
  pickDefaultReasoningEffort,
  upsertGrokCatalogModel,
} from './grokProviderModels.ts';

/**
 * Resolve the preset for one fetched model.
 *
 * The groups are searched in the order the model editor's preset strip shows
 * them — xAI first, then the group matching the channel protocol — and only
 * then the global fallback (`findPresetModelById` in "any group" mode).
 *
 * The per-group pass reads `PRESET_MODELS` directly because
 * `findPresetModelById` always falls back to every group: calling it once per
 * group would make the protocol preference dead code. Ids are unique across
 * the bundled groups today, so both orders agree; this keeps the *stated*
 * preference true if a later remote preset update introduces a collision.
 */
export function findGrokPresetForModel(
  modelId: string,
  apiFormat?: GrokApiFormat | string | null,
): PresetModel | undefined {
  const normalizedModelId = modelId.trim().toLowerCase();
  if (!normalizedModelId) {
    return undefined;
  }
  for (const npmType of getGrokPrimaryPresetNpmTypes(apiFormat)) {
    const matched = (PRESET_MODELS[npmType] ?? [])
      .find((presetModel) => presetModel.id.trim().toLowerCase() === normalizedModelId);
    if (matched) {
      return matched;
    }
  }
  return findPresetModelById(modelId);
}

/**
 * Build a catalog row for a model picked from the provider's model list.
 *
 * Presets supply the capability data the API's `/models` response never
 * carries: display name, context window, reasoning menu and modalities. The
 * upstream id is copied verbatim (preset matching is case-insensitive; casing
 * belongs to the provider, not to the preset).
 *
 * `maxCompletionTokens` is deliberately not filled: the model editor does not
 * expose it, and the bundled xAI presets declare an output limit equal to the
 * context window — writing that into `[model.*]` would cap real responses.
 */
export function buildFetchedGrokCatalogModel(
  fetchedModel: FetchedModel,
  matchedPresetModel?: PresetModel | null,
  channelDefaults?: {
    baseUrl?: string;
    apiBackend?: string;
  },
): GrokCatalogModel {
  const modelId = fetchedModel.id.trim();
  const base: GrokCatalogModel = {
    key: modelId,
    model: modelId,
    displayName: fetchedModel.name?.trim() || modelId,
    ...(channelDefaults?.baseUrl ? { baseUrl: channelDefaults.baseUrl } : {}),
    ...(channelDefaults?.apiBackend ? { apiBackend: channelDefaults.apiBackend } : {}),
  };
  if (!matchedPresetModel) {
    return base;
  }

  const efforts = extractReasoningEffortsFromPresetVariants(matchedPresetModel.variants);
  const defaultEffort = pickDefaultReasoningEffort(efforts);
  const modalities = matchedPresetModel.modalities;
  const attachment = matchedPresetModel.attachment;

  return {
    ...base,
    displayName: matchedPresetModel.name?.trim() || base.displayName,
    ...(matchedPresetModel.contextLimit ? { contextWindow: matchedPresetModel.contextLimit } : {}),
    ...(modalities ? { modalities: { ...modalities } } : {}),
    ...(typeof attachment === 'boolean'
      ? { attachment, supportsImage: attachment }
      : {}),
    // A preset with no variants says nothing about the effort menu, so the row
    // omits it — exactly what the manual editor writes in that case. Only a
    // parsed menu is persisted.
    ...(efforts.length > 0
      ? {
        supportsReasoningEffort: true,
        reasoningEfforts: efforts,
        ...(defaultEffort ? { reasoningEffort: defaultEffort } : {}),
      }
      : {}),
  };
}

/**
 * Merge the models picked in FetchModelsModal into a channel's catalog.
 *
 * - Rows already in the catalog keep their context window and reasoning menu;
 *   the display name follows the provider's `/models` label when it sends one
 *   (the pre-existing behavior — the modal disables those rows anyway). The
 *   channel Base URL / API backend are channel-level values, so they are
 *   refreshed like any other channel save does
 *   (`buildGrokProviderSettingsWithModels` overwrites them regardless).
 * - New rows are enriched from the preset when one matches.
 */
export function mergeFetchedGrokCatalogModels(
  current: GrokCatalogModel[],
  fetchedModels: FetchedModel[],
  apiFormat: GrokApiFormat | string | null | undefined,
  channelDefaults: {
    baseUrl?: string;
    apiBackend?: string;
  },
): GrokCatalogModel[] {
  let models = current;
  for (const fetchedModel of fetchedModels) {
    const key = fetchedModel.id.trim();
    if (!key) {
      continue;
    }
    const existing = models.find((model) => (model.key?.trim() || model.model) === key);
    if (existing) {
      models = upsertGrokCatalogModel(models, {
        ...existing,
        key,
        model: key,
        displayName: fetchedModel.name?.trim() || existing.displayName || key,
        ...(channelDefaults.baseUrl ? { baseUrl: channelDefaults.baseUrl } : {}),
        ...(channelDefaults.apiBackend ? { apiBackend: channelDefaults.apiBackend } : {}),
      }, existing.key);
      continue;
    }
    models = upsertGrokCatalogModel(
      models,
      buildFetchedGrokCatalogModel(
        fetchedModel,
        findGrokPresetForModel(key, apiFormat),
        channelDefaults,
      ),
      key,
    );
  }
  return models;
}
