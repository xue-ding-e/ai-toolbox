import { invoke } from '@tauri-apps/api/core';

/**
 * Provider list UI state shared by all coding tabs: per-module sort mode
 * preference, the last-used timestamp map used by the "recently used" sort
 * mode, and the layout preference (list vs compact card grid). Field names
 * mirror the Rust `ProviderListState` serialization (snake_case, no rename).
 */
export interface ProviderListState {
  sort_modes: Record<string, string>;
  last_used: Record<string, string>;
  view_modes: Record<string, string>;
}

export const getProviderListState = async (): Promise<ProviderListState> => {
  return await invoke<ProviderListState>('get_provider_list_state');
};

export const saveProviderSortMode = async (module: string, mode: string): Promise<void> => {
  await invoke('save_provider_sort_mode', { module, mode });
};

export const saveProviderViewMode = async (module: string, mode: string): Promise<void> => {
  await invoke('save_provider_view_mode', { module, mode });
};

export const recordProviderLastUsed = async (module: string, providerId: string): Promise<void> => {
  await invoke('record_provider_last_used', { module, providerId });
};
