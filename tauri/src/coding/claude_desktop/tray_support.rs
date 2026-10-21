//! Claude Desktop Tray Support Module.
//!
//! Provides the standardized API for tray menu integration. Applying from the
//! tray goes through `provider_switch`, the same entry point every other CLI
//! uses: it engages the gateway when the selected provider needs it and applies
//! directly otherwise. Calling the direct apply internal here would let a
//! tray click write a gateway profile without the gateway being engaged.

use tauri::{AppHandle, Manager, Runtime};

use crate::coding::proxy_gateway::{
    cli_proxy, paths::ProxyGatewayPaths, provider_protocol, provider_switch, types::GatewayCliKey,
    ProxyGatewayState,
};

use super::constants::COMMON_CONFIG_ID;

/// Item for provider selection in the tray menu.
#[derive(Debug, Clone)]
pub struct TrayProviderItem {
    pub id: String,
    pub display_name: String,
    pub is_selected: bool,
    pub is_disabled: bool,
    pub sort_index: i64,
}

/// Data for a provider submenu.
#[derive(Debug, Clone)]
pub struct TrayProviderData {
    pub title: String,
    pub current_display: String,
    pub items: Vec<TrayProviderItem>,
}

fn find_provider_display_name(items: &[TrayProviderItem]) -> String {
    items
        .iter()
        .find(|item| item.is_selected)
        .map(|item| item.display_name.clone())
        .unwrap_or_default()
}

fn gateway_running<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.state::<ProxyGatewayState>()
        .manager
        .lock()
        .map(|manager| manager.status().running)
        .unwrap_or(false)
}

/// Get tray provider data for Claude Desktop.
pub async fn get_claude_desktop_tray_data<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<TrayProviderData, String> {
    let providers = super::commands::list_claude_desktop_providers(app.state()).await?;
    // Which tray clicks would fail, mirroring the Claude Code tray's rule:
    //
    // - a provider whose model routes need the gateway cannot engage it while
    //   the gateway is stopped;
    // - while the gateway is taking this CLI over, the already-applied row is
    //   a no-op, an official row cannot be engaged at all, and nothing can be
    //   switched if the gateway stopped meanwhile.
    let gateway_running = gateway_running(app);
    let gateway_paths = ProxyGatewayPaths::new(crate::app_paths::resolved_data_dir());
    let gateway_takeover_active =
        cli_proxy::provider_switch_locked_by_manifest(&gateway_paths, GatewayCliKey::ClaudeDesktop);

    let mut items: Vec<TrayProviderItem> = providers
        .into_iter()
        .filter(|provider| provider.id != COMMON_CONFIG_ID)
        .map(|provider| {
            let needs_gateway = provider_protocol::provider_needs_gateway_proxy(
                GatewayCliKey::ClaudeDesktop,
                &provider.category,
                provider.meta.as_ref(),
                &provider.settings_config,
            );
            let disabled_while_taken_over = gateway_takeover_active
                && (!gateway_running || provider.is_applied || provider.category == "official");
            TrayProviderItem {
                id: provider.id,
                display_name: provider.name,
                is_selected: provider.is_applied,
                is_disabled: provider.is_disabled
                    || (needs_gateway && !gateway_running)
                    || disabled_while_taken_over,
                sort_index: provider.sort_index.unwrap_or(0) as i64,
            }
        })
        .collect();

    items.sort_by_key(|item| item.sort_index);
    let current_display = find_provider_display_name(&items);
    Ok(TrayProviderData {
        title: "──── Claude Desktop ────".to_string(),
        current_display,
        items,
    })
}

/// Apply provider selection from the tray menu.
///
/// `apply_or_switch_provider` decides between a direct apply and gateway
/// takeover from the provider's own protocol/model mapping — the same decision
/// the page's 应用 / 应用并代理 buttons make, so the tray cannot leave a
/// mapping provider applied without a gateway.
pub async fn apply_claude_desktop_provider<R: Runtime>(
    app: &AppHandle<R>,
    provider_id: &str,
) -> Result<(), String> {
    provider_switch::apply_or_switch_provider(app, GatewayCliKey::ClaudeDesktop, provider_id, true)
        .await
        .map(|_| ())
}

/// Check if Claude Desktop should be shown in the tray menu.
/// Returns true — always visible as a core feature.
pub async fn is_enabled_for_tray<R: Runtime>(_app: &AppHandle<R>) -> bool {
    true
}
