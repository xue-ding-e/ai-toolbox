import type { ModelDisplayData } from '@/components/common/ProviderCard/types';
import type { OpenCodeProvider } from '@/types/opencode';
import { parseOpenCodeModelReference, type OpenCodeModelReference } from './modelReference';

/**
 * The card's header model.
 *
 * OpenCode lets a provider omit both the display name and the npm package, so
 * the card shows the id and the openai-compatible family as fallbacks — the
 * same defaults every write path in the page uses.
 */
export interface OpenCodeProviderCardModel {
  id: string;
  name: string;
  sdkName: string;
  baseUrl: string;
}

export const OPENCODE_DEFAULT_NPM = '@ai-sdk/openai-compatible';

export const buildOpenCodeProviderCardModel = (
  providerId: string,
  provider: OpenCodeProvider,
): OpenCodeProviderCardModel => ({
  id: providerId,
  name: provider.name || providerId,
  sdkName: provider.npm || OPENCODE_DEFAULT_NPM,
  baseUrl: provider.options?.baseURL || '',
});

/**
 * The provider's model catalog as shared card rows.
 *
 * `isPrimary` is resolved from the parsed default reference rather than from a
 * string prefix: a model id may itself contain slashes (`openai/gpt-5.5`) and
 * the stored reference may carry a `#variant` suffix, so both sides must go
 * through the same parser.
 */
export const buildOpenCodeModelRows = (
  providerId: string,
  provider: OpenCodeProvider,
  defaultModelReference: OpenCodeModelReference | undefined,
): ModelDisplayData[] => (
  Object.entries(provider.models ?? {}).map(([modelId, model]) => ({
    id: modelId,
    name: model.name || modelId,
    contextLimit: model.limit?.context,
    outputLimit: model.limit?.output,
    isPrimary: defaultModelReference?.providerId === providerId
      && defaultModelReference.modelId === modelId,
  }))
);

/** The provider that owns `config.model`, or `''` when no default is set. */
export const resolveOpenCodeDefaultProviderId = (modelReference: string | undefined): string => (
  parseOpenCodeModelReference(modelReference)?.providerId ?? ''
);
