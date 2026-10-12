import React from 'react';
import { recordProviderLastUsed, saveProviderSortMode } from '@/services/providerListApi';
import {
  loadProviderListState,
  patchCachedProviderListState,
  readCachedProviderListState,
} from './providerListStateCache';
import {
  isProviderSortMode,
  type ProviderSortMode,
} from './sortProviders';

interface ProviderListSortState {
  /** Current sort mode; 'custom' (drag order) until hydration completes. */
  sortMode: ProviderSortMode;
  setSortMode: (mode: ProviderSortMode) => void;
  /** Last-used timestamp lookup for this module's providers. */
  lastUsedAt: (providerId: string) => string | undefined;
  /**
   * Mark a provider as just used: refreshes the local cache immediately and
   * persists through the backend command. DB-backed tabs call this after a
   * successful apply (the backend apply flow already persisted its own
   * marker; this keeps the in-memory cache in sync without a refetch).
   */
  noteProviderUsed: (providerId: string) => void;
}

export function useProviderListSort(moduleKey: string): ProviderListSortState {
  const [sortMode, setSortModeState] = React.useState<ProviderSortMode>('custom');
  const [lastUsed, setLastUsed] = React.useState<Record<string, string>>({});

  React.useEffect(() => {
    let cancelled = false;
    loadProviderListState().then((state) => {
      if (cancelled) {
        return;
      }
      const saved = state.sort_modes[moduleKey];
      setSortModeState(isProviderSortMode(saved) ? saved : 'custom');
      setLastUsed({ ...state.last_used });
    });
    return () => {
      cancelled = true;
    };
  }, [moduleKey]);

  const setSortMode = React.useCallback(
    (mode: ProviderSortMode) => {
      setSortModeState(mode);
      const cached = readCachedProviderListState();
      patchCachedProviderListState({
        sort_modes: { ...cached.sort_modes, [moduleKey]: mode },
      });
      void saveProviderSortMode(moduleKey, mode).catch(() => {});
    },
    [moduleKey],
  );

  const lastUsedAt = React.useCallback(
    (providerId: string) => lastUsed[`${moduleKey}:${providerId}`],
    [moduleKey, lastUsed],
  );

  const noteProviderUsed = React.useCallback(
    (providerId: string) => {
      const key = `${moduleKey}:${providerId}`;
      const now = new Date().toISOString();
      const cached = readCachedProviderListState();
      patchCachedProviderListState({
        last_used: { ...cached.last_used, [key]: now },
      });
      setLastUsed((prev) => ({ ...prev, [key]: now }));
      void recordProviderLastUsed(moduleKey, providerId).catch(() => {});
    },
    [moduleKey],
  );

  return { sortMode, setSortMode, lastUsedAt, noteProviderUsed };
}
