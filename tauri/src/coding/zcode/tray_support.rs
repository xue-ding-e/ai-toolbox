//! Tray menu integration for ZCode.
//!
//! The tray only ever moves which provider `defaultModelSelection` names; it
//! never rewrites the registry rules themselves.

use tauri::{AppHandle, Manager, Runtime};

use super::commands;
use super::projection;
use crate::db::SqliteDbState;

#[derive(Debug, Clone)]
pub struct TrayProviderItem {
    pub id: String,
    pub display_name: String,
    pub is_selected: bool,
    pub is_disabled: bool,
}

#[derive(Debug, Clone)]
pub struct TrayProviderData {
    pub title: String,
    pub current_display: String,
    pub items: Vec<TrayProviderItem>,
}

#[derive(Debug, Clone)]
pub struct TrayPromptItem {
    pub id: String,
    pub display_name: String,
    pub is_selected: bool,
}

#[derive(Debug, Clone)]
pub struct TrayPromptData {
    pub title: String,
    pub current_display: String,
    pub items: Vec<TrayPromptItem>,
}

/// Tray sections are always offered for ZCode; visibility is decided by the
/// caller through `visible_tabs`.
pub async fn is_enabled_for_tray<R: Runtime>(_app: &AppHandle<R>) -> bool {
    true
}

/// Lists managed providers with the registry's current selection marked.
///
/// Names live only in the database, so rows drive the list; the registry
/// supplies the selection and the membership filter, because a provider that is
/// not projected into the registry cannot be selected.
pub async fn get_zcode_tray_data<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<TrayProviderData, String> {
    let state = app.state::<SqliteDbState>();
    let path = commands::zcode_provider_config_path(&state)?;
    let base = projection::read_provider_config_base(&path);
    let registered = projection::list_provider_ids(&base);
    let selected = projection::read_default_provider_id(&base);

    let items = commands::list_zcode_providers_for_db(&state)?
        .into_iter()
        .filter_map(|provider| {
            let settings = serde_json::from_str::<super::types::ZcodeSettingsConfig>(
                &provider.settings_config,
            )
            .ok()?;
            if !registered.iter().any(|id| id == &settings.provider_id) {
                return None;
            }
            Some(TrayProviderItem {
                is_selected: selected.as_deref() == Some(settings.provider_id.as_str()),
                id: provider.id,
                display_name: provider.name,
                is_disabled: provider.is_disabled,
            })
        })
        .collect::<Vec<_>>();

    let current_display = items
        .iter()
        .find(|item| item.is_selected)
        .map(|item| item.display_name.clone())
        .unwrap_or_default();
    Ok(TrayProviderData {
        title: "ZCode".to_string(),
        current_display,
        items,
    })
}

/// Applies a provider chosen from the tray.
///
/// The menu hands back the database row id, but ZCode keys everything by the
/// registry `providerId`, so the row is resolved before selecting.
pub async fn apply_zcode_provider<R: Runtime>(
    app: &AppHandle<R>,
    row_id: &str,
) -> Result<(), String> {
    let state = app.state::<SqliteDbState>();
    let db = state.db();
    let provider = commands::list_zcode_providers_for_db(db)?
        .into_iter()
        .find(|provider| provider.id == row_id)
        .ok_or_else(|| format!("ZCode provider '{row_id}' not found"))?;
    let settings: super::types::ZcodeSettingsConfig =
        serde_json::from_str(&provider.settings_config)
            .map_err(|error| format!("Invalid ZCode provider settings: {error}"))?;
    let model_id = settings
        .models
        .iter()
        .find(|model| model.is_default)
        .or(settings.models.first())
        .map(|model| model.model_id.clone())
        .ok_or_else(|| "This ZCode provider has no models to select".to_string())?;

    commands::select_zcode_provider_internal_without_events(db, &settings.provider_id, &model_id)
        .await
}

pub async fn get_zcode_prompt_tray_data<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<TrayPromptData, String> {
    let items = commands::list_zcode_prompt_configs(app.state())
        .await?
        .into_iter()
        // The local-file bridge record is a mirror of the on-disk prompt, not a
        // selectable tray entry — applying it would look up a DB row that does
        // not exist and answer "not found" (same filter as the other CLIs).
        .filter(|item| item.id != crate::coding::local_bridge::LOCAL_CONFIG_ID)
        .map(|item| TrayPromptItem {
            id: item.id,
            display_name: item.name,
            is_selected: item.is_applied,
        })
        .collect::<Vec<_>>();
    let current_display = items
        .iter()
        .find(|item| item.is_selected)
        .map(|item| item.display_name.clone())
        .unwrap_or_default();
    Ok(TrayPromptData {
        title: "Global Prompt".to_string(),
        current_display,
        items,
    })
}

pub async fn apply_zcode_prompt_config<R: Runtime>(
    app: &AppHandle<R>,
    config_id: &str,
) -> Result<(), String> {
    let state = app.state::<SqliteDbState>();
    commands::apply_zcode_prompt_config_internal_without_events(state.db(), config_id).await
}
