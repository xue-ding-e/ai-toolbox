/**
 * OpenCode model references are `provider/model` with an optional
 * `#variant` suffix (`model.variant` in V2, or the `provider/model#variant`
 * string form). The provider is the first segment; the model may itself
 * contain slashes.
 */
export interface OpenCodeModelReference {
  providerId: string;
  modelId: string;
  variant?: string;
}

export const parseOpenCodeModelReference = (
  reference: string | undefined,
): OpenCodeModelReference | undefined => {
  const value = reference?.trim();
  if (!value) {
    return undefined;
  }
  const slash = value.indexOf('/');
  if (slash <= 0 || slash === value.length - 1) {
    return undefined;
  }
  const providerId = value.slice(0, slash);
  const remainder = value.slice(slash + 1);
  const hash = remainder.indexOf('#');
  const modelId = hash === -1 ? remainder : remainder.slice(0, hash);
  const variant = hash === -1 ? undefined : remainder.slice(hash + 1) || undefined;
  if (!modelId) {
    return undefined;
  }
  return { providerId, modelId, variant };
};

/** `provider/model#variant` → `provider/model`; other values pass through. */
export const stripOpenCodeModelVariant = (reference: string): string => {
  const hash = reference.indexOf('#');
  return hash === -1 ? reference : reference.slice(0, hash);
};
