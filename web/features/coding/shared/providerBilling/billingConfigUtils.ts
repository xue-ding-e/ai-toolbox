export type BillingPricingModelSource = 'inherit' | 'requested' | 'upstream';

export interface BillingConfigState {
  enabled: boolean;
  costMultiplier?: string;
  pricingModelSource: BillingPricingModelSource;
  /**
   * Extra multiplier this channel charges when it serves the Fast service tier
   * (`service_tier = priority|fast`). Only OpenAI-protocol channels (Codex) can
   * hit a tier, so only those forms render it. Empty means no surcharge.
   */
  fastCostMultiplier?: string;
  /** Same as {@link BillingConfigState.fastCostMultiplier} for Ultrafast. */
  ultrafastCostMultiplier?: string;
}

export interface GatewayProviderBillingMeta {
  providerType?: string;
  costMultiplier?: string;
  pricingModelSource?: string;
  fastCostMultiplier?: string;
  ultrafastCostMultiplier?: string;
}

export function normalizeBillingPricingModelSource(
  source?: string,
): BillingPricingModelSource {
  const normalizedSource = source?.trim().toLowerCase();
  if (normalizedSource === 'request' || normalizedSource === 'requested') {
    return 'requested';
  }
  if (normalizedSource === 'upstream' || normalizedSource === 'response') {
    return 'upstream';
  }
  return 'inherit';
}

export function getBillingConfigFromMeta(
  meta?: GatewayProviderBillingMeta,
): BillingConfigState {
  const costMultiplier = meta?.costMultiplier?.trim();
  const pricingModelSource = normalizeBillingPricingModelSource(meta?.pricingModelSource);
  const fastCostMultiplier = meta?.fastCostMultiplier?.trim();
  const ultrafastCostMultiplier = meta?.ultrafastCostMultiplier?.trim();

  return {
    enabled: Boolean(
      costMultiplier
      || pricingModelSource !== 'inherit'
      || fastCostMultiplier
      || ultrafastCostMultiplier
    ),
    costMultiplier: costMultiplier || undefined,
    pricingModelSource,
    fastCostMultiplier: fastCostMultiplier || undefined,
    ultrafastCostMultiplier: ultrafastCostMultiplier || undefined,
  };
}

export function mergeBillingConfigIntoMeta<T extends GatewayProviderBillingMeta>(
  meta: T | undefined,
  billingConfig: BillingConfigState,
): T | undefined {
  const nextMeta = { ...(meta || {}) } as T;
  delete nextMeta.costMultiplier;
  delete nextMeta.pricingModelSource;
  delete nextMeta.fastCostMultiplier;
  delete nextMeta.ultrafastCostMultiplier;

  if (billingConfig.enabled) {
    const costMultiplier = billingConfig.costMultiplier?.trim();
    if (costMultiplier) {
      nextMeta.costMultiplier = costMultiplier;
    }

    if (billingConfig.pricingModelSource !== 'inherit') {
      nextMeta.pricingModelSource = billingConfig.pricingModelSource;
    }

    const fastCostMultiplier = billingConfig.fastCostMultiplier?.trim();
    if (fastCostMultiplier) {
      nextMeta.fastCostMultiplier = fastCostMultiplier;
    }

    const ultrafastCostMultiplier = billingConfig.ultrafastCostMultiplier?.trim();
    if (ultrafastCostMultiplier) {
      nextMeta.ultrafastCostMultiplier = ultrafastCostMultiplier;
    }
  }

  return hasMeaningfulMeta(nextMeta) ? nextMeta : undefined;
}

function hasMeaningfulMeta(meta: GatewayProviderBillingMeta): boolean {
  return Object.values(meta).some((value) => value !== undefined && value !== null && value !== '');
}
