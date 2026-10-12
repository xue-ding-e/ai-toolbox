import React from 'react';
import { saveProviderViewMode } from '@/services/providerListApi';
import {
  loadProviderListState,
  patchCachedProviderListState,
  readCachedProviderListState,
} from './providerListStateCache';

/** How a provider list lays its providers out. */
export type ProviderViewMode = 'list' | 'card';

export const PROVIDER_VIEW_MODES: readonly ProviderViewMode[] = ['list', 'card'];

export const isProviderViewMode = (value: unknown): value is ProviderViewMode =>
  typeof value === 'string' && (PROVIDER_VIEW_MODES as readonly string[]).includes(value);

interface ProviderListViewPrefs {
  /** `'list'` until hydration completes, and for any unset/invalid stored value. */
  viewMode: ProviderViewMode;
  setViewMode: (mode: ProviderViewMode) => void;
}

/**
 * Per-module provider list layout preference, read through the same
 * single-flight cache as the sort mode so a page costs no extra backend round
 * trip.
 *
 * Like the sort mode it is a view preference: an unset or invalid value falls
 * back to the default instead of failing, and the write persists through its
 * own command so two tabs never clobber each other's keys.
 */
export function useProviderListViewPrefs(moduleKey: string): ProviderListViewPrefs {
  const [viewMode, setViewModeState] = React.useState<ProviderViewMode>('list');

  React.useEffect(() => {
    let cancelled = false;
    loadProviderListState().then((state) => {
      if (cancelled) {
        return;
      }
      const savedViewMode = state.view_modes[moduleKey];
      setViewModeState(isProviderViewMode(savedViewMode) ? savedViewMode : 'list');
    });
    return () => {
      cancelled = true;
    };
  }, [moduleKey]);

  const setViewMode = React.useCallback(
    (mode: ProviderViewMode) => {
      setViewModeState(mode);
      const cached = readCachedProviderListState();
      patchCachedProviderListState({
        view_modes: { ...cached.view_modes, [moduleKey]: mode },
      });
      void saveProviderViewMode(moduleKey, mode).catch(() => {});
    },
    [moduleKey],
  );

  return { viewMode, setViewMode };
}