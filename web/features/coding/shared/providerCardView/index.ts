/**
 * The compact provider-card presentation: a grid of small cards, one per
 * provider, selected by the user's layout preference.
 *
 * This is a *presentation* of the same provider data the three
 * `providerCardVariants` styles render — not a fourth style. The styles are
 * chosen by the shape of a CLI's provider model; this is chosen by how dense
 * the user wants the list. See `ProviderCompactCard` and this module's
 * `AGENTS.md`.
 */
export { default as ProviderCompactCard } from './ProviderCompactCard';
export type { ProviderCompactCardLaunchAction } from './ProviderCompactCard';
export { default as ProviderCardGrid } from './ProviderCardGrid';
