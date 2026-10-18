//! OpenCode MCP Config Path Resolution
//!
//! Provides functions to get the OpenCode MCP configuration file path.
//! This reuses the path resolution logic from the OpenCode module.

use std::path::PathBuf;

/// Get OpenCode MCP config path synchronously (without database access)
///
/// Priority:
/// 1. Environment variable OPENCODE_CONFIG
/// 2. Shell configuration files
/// 3. Default path ($OPENCODE_CONFIG_DIR, then $XDG_CONFIG_HOME/opencode,
///    then ~/.config/opencode; opencode.jsonc, opencode.json, config.json)
///
/// Note: This is a simplified version that doesn't check the database
/// for custom config path, since MCP sync typically runs without async context.
pub fn get_opencode_mcp_config_path_sync() -> Option<PathBuf> {
    // 1. Check system environment variable
    if let Ok(env_path) = std::env::var("OPENCODE_CONFIG") {
        if !env_path.is_empty() {
            return Some(PathBuf::from(env_path));
        }
    }

    // 2. Check shell configuration files
    if let Some(shell_path) =
        crate::coding::open_code::shell_env::get_env_from_shell_config("OPENCODE_CONFIG")
    {
        if !shell_path.is_empty() {
            return Some(PathBuf::from(shell_path));
        }
    }

    // 3. Return default path
    get_default_opencode_config_path()
}

/// Get the default OpenCode config path
/// Checks for .jsonc first, then .json, then defaults to .jsonc for new files
fn get_default_opencode_config_path() -> Option<PathBuf> {
    // Same directory resolution as the config reader (`OPENCODE_CONFIG_DIR`,
    // `XDG_CONFIG_HOME`, then `~/.config/opencode`).
    let config_dir = crate::coding::open_code::commands::default_config_dir().ok()?;

    // V1 also accepted the legacy `config.json` name, in the same order.
    let candidates = ["opencode.jsonc", "opencode.json", "config.json"];
    for candidate in candidates {
        let path = config_dir.join(candidate);
        if path.exists() {
            return Some(path);
        }
    }
    // Return default path for new file
    Some(config_dir.join(candidates[0]))
}
