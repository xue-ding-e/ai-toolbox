import type { PresetModel } from '@/constants/presetModels';
import type {
  ZcodeModelInputFormat,
  ZcodeModelOptionSpecs,
  ZcodeModelProperties,
  ZcodeModelRuleKind,
  ZcodeModelRow,
} from '@/types/zcode';

/**
 * Modality switches, in the order the model form shows them.
 *
 * `storageKey` is the field inside `properties.inputFormat`; ZCode declares one
 * boolean per modality rather than a list, so the form's multi-select is
 * projected back into separate booleans on save.
 */
export const ZCODE_MODALITY_FIELDS = [
  { value: 'text', storageKey: 'supportsText', labelKey: 'zcode.model.text' },
  { value: 'image', storageKey: 'supportsImage', labelKey: 'zcode.model.image' },
  { value: 'video', storageKey: 'supportsVideo', labelKey: 'zcode.model.video' },
  { value: 'audio', storageKey: 'supportsAudio', labelKey: 'zcode.model.audio' },
  { value: 'pdf', storageKey: 'supportsPdf', labelKey: 'zcode.model.pdf' },
] as const;

/**
 * ZCode's manual rule schema declares only these three modality flags; `text`
 * and `audio` are smart-only. Offering them in manual mode would let the user
 * pick a value the projection layer then drops.
 */
const MANUAL_MODALITY_VALUES = new Set<string>(['image', 'video', 'pdf']);

/** Modalities a rule of this kind may carry, in display order. */
export const zcodeModalityValuesFor = (ruleKind: ZcodeModelRuleKind): string[] =>
  ZCODE_MODALITY_FIELDS.filter(
    (field) => ruleKind !== 'manual' || MANUAL_MODALITY_VALUES.has(field.value),
  ).map((field) => field.value);

/**
 * Model properties this app never edits, carried over verbatim on save.
 *
 * ZCode treats these as system fields. Its own model dialog renders no control
 * for them and passes the stored value through untouched
 * (`ProviderModelMetadata.ts`: "系统字段不由编辑草稿产生"), and its manual-mode
 * schema does not even declare them — `manualModelConfigSchema` picks only the
 * leaves the product deliberately opens up.
 *
 * Two consequences for this app:
 *
 * - **Do not render inputs for them.** Doing so made the value editable in
 *   smart mode and uneditable in manual mode: one field with two behaviours.
 * - **Do not drop them on save.** A row written by ZCode (or by an older build
 *   of this app) carries them, and re-saving must not silently delete them.
 */
export const ZCODE_SYSTEM_PROPERTY_KEYS = [
  'supportsToolCall',
  'requiresMfjsToolSchema',
  'outputFormat',
] as const;

/** Copies the system properties present on `existing`, leaving absent ones absent. */
export const pickZcodeSystemProperties = (
  existing: ZcodeModelProperties | undefined,
): Pick<ZcodeModelProperties, (typeof ZCODE_SYSTEM_PROPERTY_KEYS)[number]> => {
  const carried: ZcodeModelProperties = {};
  if (existing?.supportsToolCall !== undefined) {
    carried.supportsToolCall = existing.supportsToolCall;
  }
  if (existing?.requiresMfjsToolSchema !== undefined) {
    carried.requiresMfjsToolSchema = existing.requiresMfjsToolSchema;
  }
  if (existing?.outputFormat !== undefined) {
    carried.outputFormat = existing.outputFormat;
  }
  return carried;
};

/**
 * Drops undefined keys so the projection layer can tell "leave this to ZCode's
 * catalog" from "explicitly set".
 */
export const compactObject = <T extends object>(value: T): T | undefined => {
  const entries = Object.entries(value).filter(([, item]) => item !== undefined);
  return entries.length > 0 ? (Object.fromEntries(entries) as T) : undefined;
};

/**
 * Projects a modality selection into ZCode's per-modality booleans.
 *
 * Only the selected modalities are written; the rest stay absent so ZCode's
 * catalog keeps deciding them. An empty selection writes no `inputFormat`.
 */
export const buildInputFormat = (
  selectedModalities: Iterable<string>,
): ZcodeModelInputFormat | undefined => {
  const selected = new Set(selectedModalities);
  return compactObject<ZcodeModelInputFormat>(
    Object.fromEntries(
      ZCODE_MODALITY_FIELDS.map((field) => [
        field.storageKey,
        selected.has(field.value) ? true : undefined,
      ]),
    ) as ZcodeModelInputFormat,
  );
};

/** Reads the stored per-modality booleans back into the form's selection list. */
export const readInputModalities = (inputFormat: ZcodeModelInputFormat): string[] =>
  ZCODE_MODALITY_FIELDS.filter((field) => inputFormat[field.storageKey] === true).map(
    (field) => field.value,
  );

/** Preset input modalities this rule kind can carry; the rest are dropped. */
export const presetInputModalities = (
  preset: PresetModel,
  ruleKind: ZcodeModelRuleKind,
): string[] => {
  const accepted = new Set(zcodeModalityValuesFor(ruleKind));
  return (preset.modalities?.input ?? []).filter((value) => accepted.has(value));
};

/**
 * Preset catalogs to prefer first, most specific first.
 *
 * ZCode's three API formats line up with the AI SDK provider families the
 * preset data is grouped by, so a DeepSeek model appears under the Anthropic
 * entry when the provider speaks `anthropic-messages`.
 */
export const ZCODE_PRIMARY_PRESET_NPM_TYPES: Record<string, string[]> = {
  'anthropic-messages': ['@ai-sdk/anthropic'],
  'openai-chat-completions': ['@ai-sdk/openai', '@ai-sdk/openai-compatible'],
  'openai-responses': ['@ai-sdk/openai'],
};

/** The npm group to look in first for a provider speaking `apiType`. */
export const preferredPresetNpmTypes = (apiType: string | undefined): string[] =>
  ZCODE_PRIMARY_PRESET_NPM_TYPES[apiType ?? ''] ?? [];

/**
 * Builds a stored model row from a preset.
 *
 * Only fields ZCode actually stores are written; the preset's cost, variants
 * and options have no ZCode counterpart and are dropped rather than forced into
 * a shape the CLI would reject. A model the preset catalog does not know still
 * yields a usable row — id only — so callers need no separate branch.
 */
export const buildZcodeModelRowFromPreset = (
  modelId: string,
  ruleKind: ZcodeModelRuleKind,
  preset?: PresetModel,
): ZcodeModelRow => {
  return {
    modelId,
    displayName: preset?.name?.trim() || undefined,
    ruleKind,
    enabled: true,
    // `supportsToolCall` is deliberately not written here: it is a system
    // property (see `ZCODE_SYSTEM_PROPERTY_KEYS`), and ZCode's own dialog sets
    // it from the model catalog rather than from what a preset claims.
    properties: compactObject<ZcodeModelProperties>({
      contextWindow: preset?.contextLimit,
      inputFormat: preset ? buildInputFormat(presetInputModalities(preset, ruleKind)) : undefined,
    }),
    optionSpecs: compactObject<ZcodeModelOptionSpecs>({
      maxOutputTokens: compactObject({ max: preset?.outputLimit }),
    }),
    isDefault: false,
  };
};