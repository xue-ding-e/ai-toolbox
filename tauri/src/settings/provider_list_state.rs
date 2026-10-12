use chrono::Local;
use serde_json::Value;

use super::types::{AppSettings, ProviderListState};
use crate::db::helpers::{db_patch_fields, db_put};
use crate::db::schema::DbTable;
use crate::db::SqliteDbState;

use super::adapter;
use super::store::{load_settings_from_sqlite_state, SETTINGS_ID};

/// Read the provider list UI state (per-module sort modes, last-used map and
/// list layout).
pub fn get_provider_list_state(sqlite_state: &SqliteDbState) -> Result<ProviderListState, String> {
    let settings = load_settings_from_sqlite_state(sqlite_state)?;
    Ok(ProviderListState {
        sort_modes: settings.provider_sort_modes,
        last_used: settings.provider_last_used,
        view_modes: settings.provider_view_modes,
    })
}

/// Persist a single module's provider sort mode. Only patches the nested
/// `provider_sort_modes` key so rapid menu toggles never clobber other
/// settings fields (same pattern as session_detail_filters).
pub fn save_provider_sort_mode_in_sqlite_state(
    sqlite_state: &SqliteDbState,
    module: &str,
    mode: &str,
) -> Result<(), String> {
    let settings = load_settings_from_sqlite_state(sqlite_state)?;
    let mut sort_modes = settings.provider_sort_modes;
    sort_modes.insert(module.to_string(), mode.to_string());

    let value = serde_json::to_value(sort_modes)
        .map_err(|error| format!("serialize provider sort modes: {error}"))?;
    patch_or_create_settings_field(sqlite_state, "provider_sort_modes", value)
}

/// Persist a single module's provider list layout (`list` / `card`). Same
/// nested-patch semantics as the sort mode above.
pub fn save_provider_view_mode_in_sqlite_state(
    sqlite_state: &SqliteDbState,
    module: &str,
    mode: &str,
) -> Result<(), String> {
    let settings = load_settings_from_sqlite_state(sqlite_state)?;
    let mut view_modes = settings.provider_view_modes;
    view_modes.insert(module.to_string(), mode.to_string());

    let value = serde_json::to_value(view_modes)
        .map_err(|error| format!("serialize provider view modes: {error}"))?;
    patch_or_create_settings_field(sqlite_state, "provider_view_modes", value)
}

/// Record that a provider was just applied/selected. Called from each coding
/// module's apply flow (covers both window and tray switching) and exposed as
/// a Tauri command for module paths where only the frontend knows the
/// provider id (e.g. "set as default model" actions in file-based tabs).
pub fn record_provider_last_used_in_sqlite_state(
    sqlite_state: &SqliteDbState,
    module: &str,
    provider_id: &str,
) -> Result<(), String> {
    let settings = load_settings_from_sqlite_state(sqlite_state)?;
    let mut last_used = settings.provider_last_used;
    last_used.insert(format!("{module}:{provider_id}"), Local::now().to_rfc3339());

    let value = serde_json::to_value(last_used)
        .map_err(|error| format!("serialize provider last-used map: {error}"))?;
    patch_or_create_settings_field(sqlite_state, "provider_last_used", value)
}

fn patch_or_create_settings_field(
    sqlite_state: &SqliteDbState,
    field: &str,
    value: Value,
) -> Result<(), String> {
    sqlite_state.with_conn(|conn| {
        let updated = db_patch_fields(
            conn,
            DbTable::Settings,
            SETTINGS_ID,
            &[(field, value.clone())],
        )?;

        if updated.is_none() {
            let mut payload = adapter::to_db_value(&AppSettings::default());
            if let Some(object) = payload.as_object_mut() {
                object.insert(field.to_string(), value.clone());
            }
            db_put(conn, DbTable::Settings, SETTINGS_ID, &payload)?;
        }

        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDbState;

    #[test]
    fn provider_list_state_round_trip_creates_missing_record() {
        let sqlite_state = SqliteDbState::in_memory_for_test().expect("sqlite");

        // No settings record yet: read returns empty state.
        let empty = get_provider_list_state(&sqlite_state).expect("read empty");
        assert!(empty.sort_modes.is_empty());
        assert!(empty.last_used.is_empty());
        assert!(empty.view_modes.is_empty());

        save_provider_sort_mode_in_sqlite_state(&sqlite_state, "kimi", "recent")
            .expect("save sort mode");
        record_provider_last_used_in_sqlite_state(&sqlite_state, "kimi", "prov_a")
            .expect("record last used");

        let state = get_provider_list_state(&sqlite_state).expect("read saved");
        assert_eq!(
            state.sort_modes.get("kimi").map(String::as_str),
            Some("recent")
        );
        assert!(state.last_used.contains_key("kimi:prov_a"));

        // Other settings survive the nested patch.
        let settings = load_settings_from_sqlite_state(&sqlite_state).expect("load settings");
        assert_eq!(settings.theme, "system");
    }

    #[test]
    fn provider_sort_mode_patch_merges_modules_and_updates_in_place() {
        let sqlite_state = SqliteDbState::in_memory_for_test().expect("sqlite");

        save_provider_sort_mode_in_sqlite_state(&sqlite_state, "kimi", "recent").expect("kimi");
        save_provider_sort_mode_in_sqlite_state(&sqlite_state, "codex", "name").expect("codex");
        save_provider_sort_mode_in_sqlite_state(&sqlite_state, "kimi", "created").expect("kimi 2");

        let state = get_provider_list_state(&sqlite_state).expect("read state");
        assert_eq!(state.sort_modes.len(), 2);
        assert_eq!(
            state.sort_modes.get("kimi").map(String::as_str),
            Some("created")
        );
        assert_eq!(
            state.sort_modes.get("codex").map(String::as_str),
            Some("name")
        );
    }

    #[test]
    fn provider_view_mode_round_trips_without_clobbering_sort_mode() {
        let sqlite_state = SqliteDbState::in_memory_for_test().expect("sqlite");

        save_provider_sort_mode_in_sqlite_state(&sqlite_state, "claudecode", "recent")
            .expect("sort mode");
        save_provider_view_mode_in_sqlite_state(&sqlite_state, "claudecode", "card")
            .expect("view mode");

        let state = get_provider_list_state(&sqlite_state).expect("read state");
        assert_eq!(
            state.view_modes.get("claudecode").map(String::as_str),
            Some("card")
        );
        // The sibling nested key survives: each writer patches only its own key.
        assert_eq!(
            state.sort_modes.get("claudecode").map(String::as_str),
            Some("recent")
        );

        save_provider_view_mode_in_sqlite_state(&sqlite_state, "codex", "list").expect("codex");
        let state = get_provider_list_state(&sqlite_state).expect("read state");
        assert_eq!(state.view_modes.len(), 2);
        assert_eq!(
            state.view_modes.get("codex").map(String::as_str),
            Some("list")
        );
    }

    #[test]
    fn record_provider_last_used_keys_by_module_and_refreshes_timestamp() {
        let sqlite_state = SqliteDbState::in_memory_for_test().expect("sqlite");

        record_provider_last_used_in_sqlite_state(&sqlite_state, "codex", "prov_a")
            .expect("record a");
        record_provider_last_used_in_sqlite_state(&sqlite_state, "kimi", "prov_a")
            .expect("record kimi");
        let first = get_provider_list_state(&sqlite_state)
            .expect("read state")
            .last_used
            .get("codex:prov_a")
            .cloned()
            .expect("codex entry");

        record_provider_last_used_in_sqlite_state(&sqlite_state, "codex", "prov_a")
            .expect("record a again");
        let state = get_provider_list_state(&sqlite_state).expect("read state");
        assert_eq!(state.last_used.len(), 2);
        let refreshed = state.last_used.get("codex:prov_a").cloned().expect("entry");
        assert!(refreshed >= first);
    }
}
