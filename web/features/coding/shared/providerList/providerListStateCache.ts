import {
  getProviderListState,
  type ProviderListState,
} from '@/services/providerListApi';

/**
 * The provider list UI state, shared by every coding tab through **one**
 * `get_provider_list_state` round trip.
 *
 * Pages are KeepAlive-mounted and hydrate independently, so without a module
 * cache each visited tab would re-read the same record. Writers patch the cache
 * in place as well, so a later-mounting page sees the value it just changed
 * instead of racing a refetch.
 *
 * Deliberately module scope rather than a store: this is a read-through cache
 * for a backend-owned record, not application state. The backend remains the
 * source of truth, and every mutation is persisted through its own command.
 */
const EMPTY_STATE: ProviderListState = {
  sort_modes: {},
  last_used: {},
  view_modes: {},
};

let cachedState: ProviderListState = EMPTY_STATE;
let statePromise: Promise<ProviderListState> | null = null;

/**
 * Normalize a response: a backend older than a field would omit it, and
 * consumers index into these maps directly.
 */
const normalize = (state: ProviderListState): ProviderListState => ({
  sort_modes: state.sort_modes ?? {},
  last_used: state.last_used ?? {},
  view_modes: state.view_modes ?? {},
});

export const loadProviderListState = (): Promise<ProviderListState> => {
  if (!statePromise) {
    statePromise = getProviderListState()
      .then((state) => {
        cachedState = normalize(state);
      })
      // A failed read settles to defaults rather than rejecting: callers render
      // the default order/layout, which is always a valid view.
      .catch(() => {
        cachedState = EMPTY_STATE;
      })
      // Re-reads resolve from the CURRENT cache, not the load-time snapshot:
      // writers patch `cachedState` in place, and a KeepAlive LRU eviction can
      // remount a page long after this load settled — handing such a remount
      // the frozen snapshot would silently revert every write since then
      // (view mode, sort order) while the backend still holds the new value.
      .then(() => cachedState);
  }
  return statePromise;
};

/** The last loaded state, without waiting for the (single-flight) read. */
export const readCachedProviderListState = (): ProviderListState => cachedState;

/**
 * Apply a locally-known change so other tabs and later mounts see it without
 * refetching. Callers still persist through the matching backend command.
 */
export const patchCachedProviderListState = (
  patch: Partial<ProviderListState>,
): void => {
  cachedState = { ...cachedState, ...patch };
};
