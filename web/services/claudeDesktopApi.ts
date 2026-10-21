/**
 * Claude Desktop API Service
 *
 * Wraps the Tauri backend commands for Claude Desktop (desktop GUI) provider
 * and config management. Invoke names must match the backend contract exactly.
 */

import { invoke } from '@tauri-apps/api/core';
import type { AllApiHubProviderItem, AllApiHubProvidersResult } from '@/types/allApiHub';
import type {
  ClaudeDesktopCommonConfig,
  ClaudeDesktopCommonConfigInput,
  ClaudeDesktopPathInfo,
  ClaudeDesktopProvider,
  ClaudeDesktopProviderInput,
  ClaudeDesktopStatus,
} from '@/types/claudedesktop';

/** Get the resolved Claude Desktop config paths for the current platform. */
export const getClaudeDesktopPaths = async (): Promise<ClaudeDesktopPathInfo> => {
  return await invoke<ClaudeDesktopPathInfo>('get_claude_desktop_paths');
};

/** Get Claude Desktop on-disk status (supported / configured / applied). */
export const getClaudeDesktopStatus = async (): Promise<ClaudeDesktopStatus> => {
  return await invoke<ClaudeDesktopStatus>('get_claude_desktop_status');
};

/** Read the current on-disk Claude Desktop 3P files for the preview modal. */
export const getClaudeDesktopPreview = async (): Promise<unknown> => {
  return await invoke<unknown>('get_claude_desktop_preview');
};

/** List all Claude Desktop providers, ordered by sort_index. */
export const listClaudeDesktopProviders = async (): Promise<ClaudeDesktopProvider[]> => {
  return await invoke<ClaudeDesktopProvider[]>('list_claude_desktop_providers');
};

/** Create a new Claude Desktop provider. */
export const createClaudeDesktopProvider = async (
  provider: ClaudeDesktopProviderInput,
): Promise<ClaudeDesktopProvider> => {
  return await invoke<ClaudeDesktopProvider>('create_claude_desktop_provider', { provider });
};

/** Update an existing Claude Desktop provider. */
export const updateClaudeDesktopProvider = async (
  provider: ClaudeDesktopProvider,
): Promise<ClaudeDesktopProvider> => {
  return await invoke<ClaudeDesktopProvider>('update_claude_desktop_provider', { provider });
};

/** Delete a Claude Desktop provider. */
export const deleteClaudeDesktopProvider = async (id: string): Promise<void> => {
  await invoke('delete_claude_desktop_provider', { id });
};

/** Toggle a Claude Desktop provider's disabled flag. */
export const toggleClaudeDesktopProviderDisabled = async (
  providerId: string,
  isDisabled: boolean,
): Promise<void> => {
  await invoke('toggle_claude_desktop_provider_disabled', { providerId, isDisabled });
};

/** Reorder Claude Desktop providers (full ordered id list). */
export const reorderClaudeDesktopProviders = async (ids: string[]): Promise<void> => {
  await invoke('reorder_claude_desktop_providers', { ids });
};

/** Mark a provider as applied in the database (no disk write). */
export const selectClaudeDesktopProvider = async (id: string): Promise<void> => {
  await invoke('select_claude_desktop_provider', { id });
};

/** Apply a provider's configuration to the Claude Desktop files. */
export const applyClaudeDesktopProvider = async (providerId: string): Promise<void> => {
  await invoke('apply_claude_desktop_provider', { providerId });
};

/** Get the Claude Desktop common (base) config. */
export const getClaudeDesktopCommonConfig = async (): Promise<ClaudeDesktopCommonConfig | null> => {
  return await invoke<ClaudeDesktopCommonConfig | null>('get_claude_desktop_common_config');
};

/** Save the common (base) config and re-apply the applied provider. */
export const saveClaudeDesktopCommonConfig = async (
  input: ClaudeDesktopCommonConfigInput,
): Promise<void> => {
  await invoke('save_claude_desktop_common_config', { input });
};

/** Import Claude Code providers into the Claude Desktop provider table. */
export const importClaudeDesktopProvidersFromClaude = async (): Promise<number> => {
  return await invoke<number>('import_claude_desktop_providers_from_claude');
};

export const listClaudeDesktopAllApiHubProviders = async (): Promise<AllApiHubProvidersResult> => {
  return await invoke<AllApiHubProvidersResult>('list_claude_desktop_all_api_hub_providers');
};

export const resolveClaudeDesktopAllApiHubProviders = async (
  providerIds: string[]
): Promise<AllApiHubProviderItem[]> => {
  return await invoke<AllApiHubProviderItem[]>('resolve_claude_desktop_all_api_hub_providers', {
    request: { providerIds },
  });
};
