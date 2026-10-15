//! Remembers providers that refused a native Codex remote-compaction v2 turn.
//!
//! Codex marks a remote compaction v2 turn by appending a `compaction_trigger`
//! to `input[]` and expects exactly one `compaction` item back. A Responses
//! upstream that implements v2 answers one; a relay that only speaks the wire
//! answers an ordinary assistant message instead, which Codex rejects with
//! `got 0 from 1 output items`. The Gateway therefore forwards the trigger
//! first and summarizes the turn itself when the reply is not an item.
//!
//! Without a memory that costs a second upstream request on *every* compaction
//! turn for every relay — the failed attempt carries the whole conversation as
//! its input. This store records the providers that have already refused one so
//! the retry is paid once instead of always.
//!
//! The memory is deliberately volatile, keyed by provider configuration:
//!
//! * Nothing is persisted, so a Gateway restart forgets every refusal. A relay
//!   that starts implementing v2 (or an operator who fixes the chain) is
//!   re-probed on the next compaction rather than being remembered as broken
//!   forever.
//! * Entries are keyed by [`provider_config_identity`], so changing the base
//!   URL, the API key, the profile, or the provider id takes effect immediately
//!   without a restart.
//!
//! [`provider_config_identity`]: super::super::upstream::provider_config_identity

use std::collections::HashSet;
use std::sync::Mutex;

/// Provider configurations that were offered a bare `compaction_trigger` and
/// answered with something other than a `compaction` item.
///
/// The key space is provider configurations, so it is bounded by how many the
/// running process has seen; no eviction is needed.
#[derive(Debug, Default)]
pub(super) struct NativeCompactionRejectionStore {
    identities: Mutex<HashSet<[u8; 32]>>,
}

impl NativeCompactionRejectionStore {
    /// Records that this configuration cannot serve a native v2 turn.
    pub(super) fn remember(&self, provider_config_identity: [u8; 32]) {
        if let Ok(mut identities) = self.identities.lock() {
            identities.insert(provider_config_identity);
        }
    }

    /// Whether a native v2 attempt is known to be wasted on this configuration.
    pub(super) fn is_remembered(&self, provider_config_identity: [u8; 32]) -> bool {
        self.identities
            .lock()
            .ok()
            .is_some_and(|identities| identities.contains(&provider_config_identity))
    }
}

#[cfg(test)]
mod tests {
    use super::NativeCompactionRejectionStore;

    #[test]
    fn a_refusal_is_remembered_per_provider_configuration() {
        let store = NativeCompactionRejectionStore::default();
        let rejected = [1u8; 32];
        let untouched = [2u8; 32];

        assert!(!store.is_remembered(rejected));
        store.remember(rejected);
        assert!(store.is_remembered(rejected));
        // Another provider's configuration must keep being probed.
        assert!(!store.is_remembered(untouched));
    }

    #[test]
    fn remembering_the_same_configuration_twice_is_idempotent() {
        let store = NativeCompactionRejectionStore::default();
        let rejected = [7u8; 32];

        store.remember(rejected);
        store.remember(rejected);
        assert!(store.is_remembered(rejected));
    }
}
