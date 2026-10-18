//! Cordis patch adapter for dsh (DeepSeek Harness) MCP sync.
//!
//! dsh configures MCP via `cordis.patch.yml` — a YAML array of patch
//! operations (`insert`/`override`/`delete`). Each MCP server is an `insert`
//! row whose `name` is the fixed package `@deepseek-ai/dsh-mcp-client` and
//! whose `config.serverName` acts as the key (like `mcpServers` key).
//!
//! This adapter:
//! - **sync**: finds an existing `insert` row with matching `serverName`, or
//!   appends a new top-level `- insert:` op. Preserves all other plugin rows.
//! - **remove**: deletes the matching inner entry; removes the op if empty.
//! - **import**: collects only `@deepseek-ai/dsh-mcp-client` entries.
//!
//! dsh is in developer preview with expected breaking changes; this adapter is
//! isolated here so format updates are localized.

use std::path::Path;

use serde_json::{json, Value};

use super::command_normalize;
use super::types::{now_ms, McpServer};
use super::yaml_sync::atomic_write_bytes;

/// The fixed Cordis plugin package name for dsh's MCP client.
const DSH_MCP_PACKAGE: &str = "@deepseek-ai/dsh-mcp-client";

/// Longest `serverName` upstream's MCP client config accepts.
///
/// dsh validates the name against `^[A-Za-z0-9_-]{1,32}$` and a violating row
/// fails to load, so it must never reach the patch file. The mirrored frontend
/// predicate lives in `web/features/coding/mcp/utils/mcpServerName.ts`.
const DSH_SERVER_NAME_MAX_LEN: usize = 32;

/// Whether a server name is usable as a dsh `serverName`.
///
/// The name is the row's identity key (`config.serverName`, and the derived
/// plugin `id`), so it is not sanitizable: reject it instead of rewriting it.
fn validate_dsh_server_name(name: &str) -> Result<(), String> {
    let length_ok = !name.is_empty() && name.chars().count() <= DSH_SERVER_NAME_MAX_LEN;
    let charset_ok = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if length_ok && charset_ok {
        return Ok(());
    }
    Err(format!(
        "dsh MCP server name {name:?} cannot be used: dsh's MCP client requires 1-32 characters \
         from A-Z a-z 0-9 _ -"
    ))
}

// ============================================================================
// Helpers
// ============================================================================

/// Read the cordis.patch.yml as a YAML array. Returns an empty array when the
/// file is missing or empty. Unlike `read_yaml_object_or_empty`, this expects a
/// top-level YAML **array** (cordis patch ops), not a mapping.
fn read_cordis_array(path: &Path) -> Result<Vec<Value>, String> {
    if !path.exists() {
        return Ok(vec![]);
    }
    let content = std::fs::read_to_string(path)
        .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
    if content.trim().is_empty() {
        return Ok(vec![]);
    }
    // serde_yaml only keeps `!js` as a tagged node; the `!!js` spelling dsh
    // reads is resolved away. Convert on the way in (and back on write) so an
    // expression is never rewritten as a literal string.
    let content = super::yaml_sync::rewrite_unquoted_tag(&content, "!!js", "!js");
    let yaml: serde_yaml::Value = serde_yaml::from_str(&content)
        .map_err(|error| format!("Failed to parse {}: {error}", path.display()))?;
    let parsed: Value = super::yaml_sync::yaml_to_json_lossless(yaml);
    if parsed.is_array() {
        Ok(parsed.as_array().cloned().unwrap_or_default())
    } else {
        // A non-array cordis.patch.yml is either corrupt or hand-edited into a
        // mapping. Returning an empty vec would cause sync to overwrite it with
        // a fresh array, destroying existing plugin rows. Refuse instead.
        Err(format!(
            "{} must contain a YAML array of patch operations, but the root is not an array",
            path.display()
        ))
    }
}

/// Write the cordis array back as YAML, restoring dsh's `!!js` spelling.
fn write_cordis_array(path: &Path, array: &[Value]) -> Result<(), String> {
    let yaml = super::yaml_sync::json_to_yaml_lossless(Value::Array(array.to_vec()));
    let yaml_str = serde_yaml::to_string(&yaml)
        .map_err(|e| format!("Failed to serialize cordis.patch.yml: {e}"))?;
    let yaml_str = super::yaml_sync::rewrite_unquoted_tag(&yaml_str, "!js", "!!js");
    atomic_write_bytes(path, yaml_str.as_bytes())
}

/// Build the `config` object for a dsh-mcp-client insert row.
fn build_cordis_config(server: &McpServer) -> Result<Value, String> {
    let server_type = server.server_type.as_str();
    let obj = server
        .server_config
        .as_object()
        .ok_or_else(|| "MCP server_config must be a JSON object".to_string())?;

    let mut config = serde_json::Map::new();
    config.insert("serverName".to_string(), json!(server.name));

    match server_type {
        "stdio" => {
            config.insert("transport".to_string(), json!("stdio"));
            if let Some(command) = obj.get("command") {
                config.insert("command".to_string(), command.clone());
            }
            if let Some(args) = obj.get("args") {
                if args.is_array() && !args.as_array().map(|a| a.is_empty()).unwrap_or(true) {
                    config.insert("args".to_string(), args.clone());
                }
            }
            if let Some(env) = obj.get("env") {
                if env.is_object() && !env.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                    config.insert("env".to_string(), env.clone());
                }
            }
        }
        "sse" | "http" => {
            config.insert("transport".to_string(), json!("streamable-http"));
            if let Some(url) = obj.get("url") {
                config.insert("url".to_string(), url.clone());
            }
            if let Some(headers) = obj.get("headers") {
                if headers.is_object() && !headers.as_object().map(|o| o.is_empty()).unwrap_or(true)
                {
                    config.insert("headers".to_string(), headers.clone());
                }
            }
        }
        _ => {
            return Err(format!("Unknown MCP type: {server_type}"));
        }
    }

    Ok(Value::Object(config))
}

/// Check whether a plugin entry is a dsh-mcp-client with the given serverName.
fn is_dsh_mcp_entry(entry: &Value, server_name: &str) -> bool {
    let name_matches = entry.get("name").and_then(|v| v.as_str()) == Some(DSH_MCP_PACKAGE);
    let server_name_matches = entry
        .get("config")
        .and_then(|c| c.get("serverName"))
        .and_then(|s| s.as_str())
        == Some(server_name);
    name_matches && server_name_matches
}

/// Check whether a plugin entry is a dsh-mcp-client (any serverName).
fn is_any_dsh_mcp_entry(entry: &Value) -> bool {
    entry.get("name").and_then(|v| v.as_str()) == Some(DSH_MCP_PACKAGE)
}

// ============================================================================
// Public API: Sync / Remove / Import
// ============================================================================

/// Sync a single MCP server to cordis.patch.yml.
///
/// Finds an existing insert row with matching `serverName` and updates it, or
/// appends a new top-level `- insert:` op. Preserves all other plugin rows.
pub(crate) fn sync_server_to_cordis(config_path: &Path, server: &McpServer) -> Result<(), String> {
    // The only guarded entry point, deliberately: `remove_server_from_cordis`
    // must stay able to delete a row whose name predates this check, and
    // `import_servers_from_cordis` must stay readable so such a row can be
    // inspected. Create/update do not reject either — a name that is fine for
    // Claude/Codex/Gemini must remain usable, and the failure belongs to this
    // tool's sync result.
    validate_dsh_server_name(&server.name)?;

    let mut array = read_cordis_array(config_path)?;

    let new_config = build_cordis_config(server)?;
    let new_entry = json!({
        "id": format!("mcp-{}", server.name),
        "name": DSH_MCP_PACKAGE,
        "config": new_config,
    });

    let mut found = false;
    for op in array.iter_mut() {
        // Each op is like { "insert": [ {plugin entries} ] }
        if let Some(insert_list) = op.get_mut("insert").and_then(|v| v.as_array_mut()) {
            for entry in insert_list.iter_mut() {
                if is_dsh_mcp_entry(entry, &server.name) {
                    // Update config in place; preserve id if present.
                    // Update ALL matching entries (duplicates should not exist,
                    // but if they do, keep them consistent — matches remove behavior).
                    if let Some(existing_id) = entry.get("id").cloned() {
                        let mut updated_entry = new_entry.clone();
                        if let Some(updated_obj) = updated_entry.as_object_mut() {
                            updated_obj.insert("id".to_string(), existing_id);
                        }
                        *entry = updated_entry;
                    } else {
                        *entry = new_entry.clone();
                    }
                    found = true;
                }
            }
        }
    }

    if !found {
        // Append a new top-level insert op
        array.push(json!({ "insert": [new_entry] }));
    }

    write_cordis_array(config_path, &array)
}

/// Remove a single MCP server from cordis.patch.yml.
///
/// Deletes the matching inner entry from any insert op; removes the op itself
/// if it becomes empty. No-op if nothing matched.
pub(crate) fn remove_server_from_cordis(
    config_path: &Path,
    server_name: &str,
) -> Result<(), String> {
    let mut array = read_cordis_array(config_path)?;

    let mut changed = false;
    let mut indices_to_remove = vec![];

    for (op_idx, op) in array.iter_mut().enumerate() {
        if let Some(insert_list) = op.get_mut("insert").and_then(|v| v.as_array_mut()) {
            let before_len = insert_list.len();
            insert_list.retain(|entry| !is_dsh_mcp_entry(entry, server_name));
            if insert_list.len() != before_len {
                changed = true;
            }
            // If the insert op is now empty, mark for removal
            if insert_list.is_empty() {
                indices_to_remove.push(op_idx);
            }
        }
    }

    if !changed {
        return Ok(());
    }

    // Remove empty insert ops (iterate in reverse to keep indices valid)
    for idx in indices_to_remove.into_iter().rev() {
        array.remove(idx);
    }

    write_cordis_array(config_path, &array)
}

/// Import MCP servers from cordis.patch.yml.
///
/// Collects only `@deepseek-ai/dsh-mcp-client` entries, ignoring all other
/// plugins (tools/skills/models/etc.).
pub(crate) fn import_servers_from_cordis(config_path: &Path) -> Result<Vec<McpServer>, String> {
    let array = read_cordis_array(config_path)?;
    let now = now_ms();
    let mut servers = Vec::new();

    for op in &array {
        let Some(insert_list) = op.get("insert").and_then(|v| v.as_array()) else {
            continue;
        };
        for entry in insert_list {
            if !is_any_dsh_mcp_entry(entry) {
                continue;
            }
            let Some(config) = entry.get("config") else {
                continue;
            };
            let Some(server_name) = config.get("serverName").and_then(|v| v.as_str()) else {
                continue;
            };
            let transport = config.get("transport").and_then(|v| v.as_str());

            if let Some(server) = build_mcp_server_from_cordis(server_name, transport, config, now)
            {
                servers.push(server);
            }
        }
    }

    Ok(servers)
}

/// Build a unified `McpServer` from a cordis config object.
fn build_mcp_server_from_cordis(
    name: &str,
    transport: Option<&str>,
    config: &Value,
    now: i64,
) -> Option<McpServer> {
    match transport {
        Some("stdio") | None => {
            // stdio: command/args/env
            if config.get("command").is_none() {
                log::warn!("dsh MCP server '{name}' has transport stdio but no command");
                return None;
            }
            let mut unified = serde_json::Map::new();
            unified.insert("type".to_string(), json!("stdio"));
            if let Some(command) = config.get("command") {
                unified.insert("command".to_string(), command.clone());
            }
            if let Some(args) = config.get("args") {
                if args.is_array() && !args.as_array().map(|a| a.is_empty()).unwrap_or(true) {
                    unified.insert("args".to_string(), args.clone());
                }
            }
            if let Some(env) = config.get("env") {
                if env.is_object() && !env.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                    unified.insert("env".to_string(), env.clone());
                }
            }
            let server_config = command_normalize::unwrap_cmd_c(&Value::Object(unified));
            Some(McpServer {
                id: String::new(),
                name: name.to_string(),
                server_type: "stdio".to_string(),
                server_config,
                enabled_tools: vec![],
                sync_details: None,
                description: None,
                user_group: None,
                user_note: None,
                tags: vec![],
                timeout: None,
                sort_index: 0,
                management_enabled: true,
                disabled_previous_tools: Vec::new(),
                created_at: now,
                updated_at: now,
            })
        }
        Some("streamable-http") => {
            // HTTP: url/headers
            if config.get("url").is_none() {
                log::warn!("dsh MCP server '{name}' has transport streamable-http but no url");
                return None;
            }
            let mut unified = serde_json::Map::new();
            unified.insert("type".to_string(), json!("sse"));
            if let Some(url) = config.get("url") {
                unified.insert("url".to_string(), url.clone());
            }
            if let Some(headers) = config.get("headers") {
                if headers.is_object() && !headers.as_object().map(|o| o.is_empty()).unwrap_or(true)
                {
                    unified.insert("headers".to_string(), headers.clone());
                }
            }
            Some(McpServer {
                id: String::new(),
                name: name.to_string(),
                server_type: "sse".to_string(),
                server_config: Value::Object(unified),
                enabled_tools: vec![],
                sync_details: None,
                description: None,
                user_group: None,
                user_note: None,
                tags: vec![],
                timeout: None,
                sort_index: 0,
                management_enabled: true,
                disabled_previous_tools: Vec::new(),
                created_at: now,
                updated_at: now,
            })
        }
        Some(other) => {
            log::warn!("dsh MCP server '{name}' has unknown transport: {other}");
            None
        }
    }
}

// ============================================================================
// Generic plugin disabled-state helpers (used by dsh agent-instructions check)
// ============================================================================

/// Check if a plugin row exists in cordis.patch.yml with a `disabled` field.
///
/// Returns `Ok(None)` when the plugin is not in the patch (no override — the
/// effective state comes from the bundle layers).
/// Returns `Ok(Some(true))` when `disabled: true`, `Ok(Some(false))` when
/// `disabled: false`.
pub(crate) fn get_plugin_disabled_state(
    config_path: &Path,
    plugin_id: &str,
) -> Result<Option<bool>, String> {
    let array = read_cordis_array(config_path)?;
    for op in &array {
        // Check both top-level entries (single-row ops) and insert-list entries.
        if entry_matches_id(op, plugin_id) {
            return Ok(op.get("disabled").and_then(|v| v.as_bool()));
        }
        if let Some(insert_list) = op.get("insert").and_then(|v| v.as_array()) {
            for entry in insert_list {
                if entry_matches_id(entry, plugin_id) {
                    return Ok(entry.get("disabled").and_then(|v| v.as_bool()));
                }
            }
        }
    }
    Ok(None)
}

/// Set `disabled: <value>` for a plugin row in cordis.patch.yml.
///
/// If the plugin row exists (as a top-level entry or inside an insert list),
/// updates its `disabled` field. If not, appends a new top-level entry.
/// Preserves all other plugin rows and ops.
pub(crate) fn set_plugin_disabled(
    config_path: &Path,
    plugin_id: &str,
    disabled: bool,
) -> Result<(), String> {
    let mut array = read_cordis_array(config_path)?;
    let mut found = false;

    // Try updating an existing top-level entry first.
    for op in array.iter_mut() {
        if entry_matches_id(op, plugin_id) {
            set_entry_disabled(op, disabled);
            found = true;
            break;
        }
        if let Some(insert_list) = op.get_mut("insert").and_then(|v| v.as_array_mut()) {
            let mut inner_found = false;
            for entry in insert_list.iter_mut() {
                if entry_matches_id(entry, plugin_id) {
                    set_entry_disabled(entry, disabled);
                    inner_found = true;
                    break;
                }
            }
            if inner_found {
                found = true;
                break;
            }
        }
    }

    if !found {
        array.push(json!({ "id": plugin_id, "disabled": disabled }));
    }

    write_cordis_array(config_path, &array)
}

/// Set a `config.<field>` value for a plugin row in cordis.patch.yml.
///
/// If the plugin row exists (as a top-level entry or inside an insert list),
/// merges the field into its `config` object (creating it when absent) and
/// preserves all other fields. If not, appends a new top-level entry
/// `{ id, config: { field: value } }`. Preserves all other plugin rows and ops.
pub(crate) fn set_plugin_config_field(
    config_path: &Path,
    plugin_id: &str,
    field: &str,
    value: Value,
) -> Result<(), String> {
    let mut array = read_cordis_array(config_path)?;
    let mut found = false;

    // Try updating an existing top-level entry first.
    for op in array.iter_mut() {
        if entry_matches_id(op, plugin_id) {
            set_entry_config_field(op, field, value.clone());
            found = true;
            break;
        }
        if let Some(insert_list) = op.get_mut("insert").and_then(|v| v.as_array_mut()) {
            let mut inner_found = false;
            for entry in insert_list.iter_mut() {
                if entry_matches_id(entry, plugin_id) {
                    set_entry_config_field(entry, field, value.clone());
                    inner_found = true;
                    break;
                }
            }
            if inner_found {
                found = true;
                break;
            }
        }
    }

    if !found {
        let mut config = serde_json::Map::new();
        config.insert(field.to_string(), value);
        array.push(json!({ "id": plugin_id, "config": config }));
    }

    write_cordis_array(config_path, &array)
}

/// Check whether a patch entry targets the given plugin id.
fn entry_matches_id(entry: &Value, plugin_id: &str) -> bool {
    entry.get("id").and_then(|v| v.as_str()) == Some(plugin_id)
}

/// Set the `disabled` field on a patch entry (as a JSON object mutation).
fn set_entry_disabled(entry: &mut Value, disabled: bool) {
    if let Some(obj) = entry.as_object_mut() {
        obj.insert("disabled".to_string(), json!(disabled));
    }
}

/// Set `config.<field>` on a patch entry, preserving other config fields.
fn set_entry_config_field(entry: &mut Value, field: &str, value: Value) {
    if let Some(obj) = entry.as_object_mut() {
        let config = obj.entry("config".to_string()).or_insert_with(|| json!({}));
        if let Some(config_obj) = config.as_object_mut() {
            config_obj.insert(field.to_string(), value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_stdio_server(name: &str, command: &str, args: &[&str]) -> McpServer {
        McpServer {
            id: String::new(),
            name: name.to_string(),
            server_type: "stdio".to_string(),
            server_config: json!({
                "type": "stdio",
                "command": command,
                "args": args,
            }),
            enabled_tools: vec![],
            sync_details: None,
            description: None,
            user_group: None,
            user_note: None,
            tags: vec![],
            timeout: None,
            sort_index: 0,
            management_enabled: true,
            disabled_previous_tools: vec![],
            created_at: 0,
            updated_at: 0,
        }
    }

    fn make_http_server(name: &str, url: &str) -> McpServer {
        McpServer {
            id: String::new(),
            name: name.to_string(),
            server_type: "sse".to_string(),
            server_config: json!({
                "type": "sse",
                "url": url,
            }),
            enabled_tools: vec![],
            sync_details: None,
            description: None,
            user_group: None,
            user_note: None,
            tags: vec![],
            timeout: None,
            sort_index: 0,
            management_enabled: true,
            disabled_previous_tools: vec![],
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn test_sync_adds_new_insert_op() {
        let dir =
            std::env::temp_dir().join(format!("cordis_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cordis.patch.yml");

        let server = make_stdio_server("memory", "npx", &["-y", "@mcp/memory"]);
        sync_server_to_cordis(&path, &server).unwrap();

        let array = read_cordis_array(&path).unwrap();
        assert_eq!(array.len(), 1);
        let entry = &array[0]["insert"][0];
        assert_eq!(entry["name"], DSH_MCP_PACKAGE);
        assert_eq!(entry["config"]["serverName"], "memory");
        assert_eq!(entry["config"]["transport"], "stdio");
        assert_eq!(entry["config"]["command"], "npx");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_sync_updates_existing_preserves_other_rows() {
        let dir =
            std::env::temp_dir().join(format!("cordis_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cordis.patch.yml");

        // Pre-populate with an existing MCP server + an unrelated plugin
        let initial = json!([
            { "insert": [
                { "id": "mcp-memory", "name": DSH_MCP_PACKAGE, "config": { "serverName": "memory", "transport": "stdio", "command": "node", "args": ["old"] } },
                { "id": "other-plugin", "name": "@dsh/some-other", "config": {} }
            ]}
        ]);
        let yaml_str = serde_yaml::to_string(&initial).unwrap();
        atomic_write_bytes(&path, yaml_str.as_bytes()).unwrap();

        // Sync update for "memory"
        let server = make_stdio_server("memory", "npx", &["-y", "@mcp/memory"]);
        sync_server_to_cordis(&path, &server).unwrap();

        let array = read_cordis_array(&path).unwrap();
        assert_eq!(array.len(), 1); // still one insert op

        let insert_list = array[0]["insert"].as_array().unwrap();
        assert_eq!(insert_list.len(), 2); // both entries preserved

        // memory entry updated
        let memory_entry = &insert_list[0];
        assert_eq!(memory_entry["name"], DSH_MCP_PACKAGE);
        assert_eq!(memory_entry["config"]["command"], "npx");
        assert_eq!(memory_entry["config"]["args"][0], "-y");
        // id preserved
        assert_eq!(memory_entry["id"], "mcp-memory");

        // other plugin untouched
        let other_entry = &insert_list[1];
        assert_eq!(other_entry["name"], "@dsh/some-other");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_remove_only_deletes_matching_server() {
        let dir =
            std::env::temp_dir().join(format!("cordis_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cordis.patch.yml");

        // Two MCP servers + one other plugin
        let initial = json!([
            { "insert": [
                { "id": "mcp-a", "name": DSH_MCP_PACKAGE, "config": { "serverName": "a", "transport": "stdio", "command": "node" } },
                { "id": "mcp-b", "name": DSH_MCP_PACKAGE, "config": { "serverName": "b", "transport": "stdio", "command": "node" } },
                { "id": "other", "name": "@dsh/other", "config": {} }
            ]}
        ]);
        let yaml_str = serde_yaml::to_string(&initial).unwrap();
        atomic_write_bytes(&path, yaml_str.as_bytes()).unwrap();

        remove_server_from_cordis(&path, "a").unwrap();

        let array = read_cordis_array(&path).unwrap();
        let insert_list = array[0]["insert"].as_array().unwrap();
        assert_eq!(insert_list.len(), 2); // b + other remain

        // a is gone, b + other survive
        let names: Vec<&str> = insert_list
            .iter()
            .map(|e| {
                e.get("config")
                    .and_then(|c| c.get("serverName"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
            })
            .collect();
        assert!(names.contains(&"b"));
        assert!(!names.contains(&"a"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_remove_removes_empty_insert_op() {
        let dir =
            std::env::temp_dir().join(format!("cordis_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cordis.patch.yml");

        let initial = json!([
            { "insert": [
                { "id": "mcp-a", "name": DSH_MCP_PACKAGE, "config": { "serverName": "a", "transport": "stdio", "command": "node" } }
            ]}
        ]);
        let yaml_str = serde_yaml::to_string(&initial).unwrap();
        atomic_write_bytes(&path, yaml_str.as_bytes()).unwrap();

        remove_server_from_cordis(&path, "a").unwrap();

        let array = read_cordis_array(&path).unwrap();
        assert!(array.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_remove_missing_is_noop() {
        let dir =
            std::env::temp_dir().join(format!("cordis_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cordis.patch.yml");

        let initial = json!([
            { "insert": [
                { "id": "mcp-a", "name": DSH_MCP_PACKAGE, "config": { "serverName": "a", "transport": "stdio", "command": "node" } }
            ]}
        ]);
        let yaml_str = serde_yaml::to_string(&initial).unwrap();
        atomic_write_bytes(&path, yaml_str.as_bytes()).unwrap();

        remove_server_from_cordis(&path, "nonexistent").unwrap();

        let array = read_cordis_array(&path).unwrap();
        assert_eq!(array.len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_import_extracts_only_dsh_mcp_entries() {
        let dir =
            std::env::temp_dir().join(format!("cordis_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cordis.patch.yml");

        // Mix of dsh-mcp-client entries and other plugins
        let initial = json!([
            { "insert": [
                { "id": "mcp-memory", "name": DSH_MCP_PACKAGE, "config": { "serverName": "memory", "transport": "stdio", "command": "npx", "args": ["-y", "@mcp/memory"] } },
                { "id": "other", "name": "@dsh/other", "config": {} }
            ]},
            { "insert": [
                { "id": "mcp-remote", "name": DSH_MCP_PACKAGE, "config": { "serverName": "remote", "transport": "streamable-http", "url": "https://example.com/mcp" } }
            ]}
        ]);
        let yaml_str = serde_yaml::to_string(&initial).unwrap();
        atomic_write_bytes(&path, yaml_str.as_bytes()).unwrap();

        let servers = import_servers_from_cordis(&path).unwrap();
        assert_eq!(servers.len(), 2);

        let names: Vec<&str> = servers.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"memory"));
        assert!(names.contains(&"remote"));

        let memory = servers.iter().find(|s| s.name == "memory").unwrap();
        assert_eq!(memory.server_type, "stdio");
        assert_eq!(memory.server_config["command"], "npx");

        let remote = servers.iter().find(|s| s.name == "remote").unwrap();
        assert_eq!(remote.server_type, "sse");
        assert_eq!(remote.server_config["url"], "https://example.com/mcp");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_sync_http_server() {
        let dir =
            std::env::temp_dir().join(format!("cordis_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cordis.patch.yml");

        let server = make_http_server("remote", "https://example.com/mcp");
        sync_server_to_cordis(&path, &server).unwrap();

        let array = read_cordis_array(&path).unwrap();
        let entry = &array[0]["insert"][0];
        assert_eq!(entry["config"]["transport"], "streamable-http");
        assert_eq!(entry["config"]["url"], "https://example.com/mcp");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_sync_then_import_roundtrip() {
        let dir =
            std::env::temp_dir().join(format!("cordis_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cordis.patch.yml");

        let server = make_stdio_server("fs", "npx", &["-y", "@mcp/fs"]);
        sync_server_to_cordis(&path, &server).unwrap();

        let imported = import_servers_from_cordis(&path).unwrap();
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].name, "fs");
        assert_eq!(imported[0].server_type, "stdio");
        assert_eq!(imported[0].server_config["command"], "npx");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A name dsh's MCP client cannot load must be refused before it is
    /// written, and the target file must stay untouched.
    #[test]
    fn test_sync_rejects_names_dsh_cannot_load() {
        let dir =
            std::env::temp_dir().join(format!("cordis_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cordis.patch.yml");

        let too_long = "a".repeat(DSH_SERVER_NAME_MAX_LEN + 1);
        for bad_name in ["", "my server", "a.b", "名字", too_long.as_str()] {
            let server = make_stdio_server(bad_name, "npx", &["-y", "@mcp/memory"]);
            assert!(
                sync_server_to_cordis(&path, &server).is_err(),
                "expected {bad_name:?} to be refused"
            );
        }
        assert!(!path.exists(), "a refused name must not create the file");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The accepted boundary is 1..=32 characters from `[A-Za-z0-9_-]`.
    #[test]
    fn test_sync_accepts_the_dsh_name_boundaries() {
        let dir =
            std::env::temp_dir().join(format!("cordis_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cordis.patch.yml");

        let exactly_max = "a".repeat(DSH_SERVER_NAME_MAX_LEN);
        for good_name in ["a", "my-server_1", exactly_max.as_str()] {
            let server = make_stdio_server(good_name, "npx", &["-y", "@mcp/memory"]);
            sync_server_to_cordis(&path, &server)
                .unwrap_or_else(|error| panic!("expected {good_name:?} to be accepted: {error}"));
        }

        let imported = import_servers_from_cordis(&path).unwrap();
        assert_eq!(imported.len(), 3);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A row written before this guard existed must stay removable, otherwise
    /// the invalid entry could never be cleaned out of the user's file.
    #[test]
    fn test_remove_can_delete_a_legacy_invalid_name() {
        let dir =
            std::env::temp_dir().join(format!("cordis_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cordis.patch.yml");

        let legacy = json!([
            { "insert": [
                { "id": "mcp-my server", "name": DSH_MCP_PACKAGE, "config": { "serverName": "my server", "transport": "stdio", "command": "npx" } }
            ]}
        ]);
        let yaml_str = serde_yaml::to_string(&legacy).unwrap();
        atomic_write_bytes(&path, yaml_str.as_bytes()).unwrap();

        remove_server_from_cordis(&path, "my server").unwrap();

        let array = read_cordis_array(&path).unwrap();
        assert!(array.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_get_plugin_disabled_state_missing_file() {
        let path = std::path::Path::new("/nonexistent/cordis_test.yml");
        assert_eq!(
            get_plugin_disabled_state(path, "agent-instructions").unwrap(),
            None
        );
    }

    #[test]
    fn test_get_plugin_disabled_state_not_present() {
        let dir =
            std::env::temp_dir().join(format!("cordis_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cordis.patch.yml");
        atomic_write_bytes(&path, b"[]").unwrap();

        assert_eq!(
            get_plugin_disabled_state(&path, "agent-instructions").unwrap(),
            None
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_get_plugin_disabled_state_explicit_true() {
        let dir =
            std::env::temp_dir().join(format!("cordis_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cordis.patch.yml");
        let yaml = "- id: agent-instructions\n  disabled: true\n";
        atomic_write_bytes(&path, yaml.as_bytes()).unwrap();

        assert_eq!(
            get_plugin_disabled_state(&path, "agent-instructions").unwrap(),
            Some(true)
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_get_plugin_disabled_state_explicit_false() {
        let dir =
            std::env::temp_dir().join(format!("cordis_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cordis.patch.yml");
        let yaml = "- id: agent-instructions\n  disabled: false\n";
        atomic_write_bytes(&path, yaml.as_bytes()).unwrap();

        assert_eq!(
            get_plugin_disabled_state(&path, "agent-instructions").unwrap(),
            Some(false)
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_set_plugin_disabled_appends_new_entry() {
        let dir =
            std::env::temp_dir().join(format!("cordis_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cordis.patch.yml");
        atomic_write_bytes(&path, b"[]").unwrap();

        set_plugin_disabled(&path, "agent-instructions", false).unwrap();

        let state = get_plugin_disabled_state(&path, "agent-instructions").unwrap();
        assert_eq!(state, Some(false));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_set_plugin_disabled_updates_existing() {
        let dir =
            std::env::temp_dir().join(format!("cordis_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cordis.patch.yml");
        let yaml = "- id: agent-instructions\n  disabled: true\n- id: other\n  disabled: true\n";
        atomic_write_bytes(&path, yaml.as_bytes()).unwrap();

        set_plugin_disabled(&path, "agent-instructions", false).unwrap();

        assert_eq!(
            get_plugin_disabled_state(&path, "agent-instructions").unwrap(),
            Some(false)
        );
        // other entry preserved
        assert_eq!(
            get_plugin_disabled_state(&path, "other").unwrap(),
            Some(true)
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_set_plugin_config_field_appends_new_entry() {
        let dir =
            std::env::temp_dir().join(format!("cordis_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cordis.patch.yml");
        atomic_write_bytes(&path, b"[]").unwrap();

        set_plugin_config_field(&path, "agent-instructions", "maxBytes", json!(262144)).unwrap();

        let array = read_cordis_array(&path).unwrap();
        assert_eq!(array.len(), 1);
        let entry = &array[0];
        assert_eq!(entry["id"], "agent-instructions");
        assert_eq!(entry["config"]["maxBytes"], 262144);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_set_plugin_config_field_updates_existing_preserves_other_fields() {
        let dir =
            std::env::temp_dir().join(format!("cordis_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cordis.patch.yml");
        let yaml = "- id: agent-instructions\n  disabled: false\n  config:\n    maxBytes: 65536\n- id: other\n  disabled: true\n";
        atomic_write_bytes(&path, yaml.as_bytes()).unwrap();

        set_plugin_config_field(&path, "agent-instructions", "maxBytes", json!(262144)).unwrap();

        let array = read_cordis_array(&path).unwrap();
        assert_eq!(array.len(), 2);
        let entry = &array[0];
        assert_eq!(entry["id"], "agent-instructions");
        assert_eq!(entry["disabled"], false);
        assert_eq!(entry["config"]["maxBytes"], 262144);
        // other entry preserved
        assert_eq!(array[1]["id"], "other");
        assert_eq!(array[1]["disabled"], true);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_set_plugin_config_field_merges_into_existing_config() {
        let dir =
            std::env::temp_dir().join(format!("cordis_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cordis.patch.yml");
        let yaml = "- id: agent-instructions\n  config:\n    otherKey: keep-me\n";
        atomic_write_bytes(&path, yaml.as_bytes()).unwrap();

        set_plugin_config_field(&path, "agent-instructions", "maxBytes", json!(262144)).unwrap();

        let array = read_cordis_array(&path).unwrap();
        let entry = &array[0];
        assert_eq!(entry["config"]["maxBytes"], 262144);
        assert_eq!(entry["config"]["otherKey"], "keep-me");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_set_plugin_config_field_updates_insert_list_entry() {
        let dir =
            std::env::temp_dir().join(format!("cordis_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cordis.patch.yml");
        let yaml = "- insert:\n    - id: agent-instructions\n      disabled: true\n    - id: other\n      disabled: true\n";
        atomic_write_bytes(&path, yaml.as_bytes()).unwrap();

        set_plugin_config_field(&path, "agent-instructions", "maxBytes", json!(262144)).unwrap();

        let array = read_cordis_array(&path).unwrap();
        let insert_list = array[0]["insert"].as_array().unwrap();
        assert_eq!(insert_list.len(), 2);
        assert_eq!(insert_list[0]["config"]["maxBytes"], 262144);
        assert_eq!(insert_list[0]["disabled"], true);
        assert_eq!(insert_list[1]["id"], "other");

        std::fs::remove_dir_all(&dir).ok();
    }
}
