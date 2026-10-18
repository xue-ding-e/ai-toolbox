//! MCP Command Normalization Module
//!
//! Handles cmd /c wrapper for Windows compatibility.
//!
//! ## Background
//! On Windows, commands like npx/npm/yarn/pnpm/node/bun/deno are actually .cmd batch files
//! and need to be executed via `cmd /c`. However:
//! - Database storage should be normalized (no cmd /c)
//! - Windows local sync needs cmd /c wrapper
//! - Mac/Linux/WSL don't need cmd /c
//!
//! ## Functions
//! - `unwrap_cmd_c`: Remove cmd /c wrapper (for database storage, import, WSL)
//! - `wrap_cmd_c`: Add cmd /c wrapper (for Windows local sync)
//! - `process_*`: Process entire config file content (for cross-platform backup restore)

use serde_json::{json, Value};

/// Commands that need cmd /c wrapper on Windows
const WINDOWS_WRAP_COMMANDS: &[&str] = &["npx", "npm", "yarn", "pnpm", "node", "bun", "deno"];

/// Check if a command needs cmd /c wrapper
fn needs_wrap(command: &str) -> bool {
    let cmd_name = std::path::Path::new(command)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(command);

    WINDOWS_WRAP_COMMANDS
        .iter()
        .any(|&c| cmd_name.eq_ignore_ascii_case(c))
}

/// Check if command is already wrapped with cmd /c
fn is_cmd_wrapped(command: &str, args: &[Value]) -> bool {
    if !command.eq_ignore_ascii_case("cmd") && !command.eq_ignore_ascii_case("cmd.exe") {
        return false;
    }

    // Check if first arg is /c
    args.first()
        .and_then(|v| v.as_str())
        .map(|s| s.eq_ignore_ascii_case("/c"))
        .unwrap_or(false)
}

// ============================================================================
// Single Server Config Processing
// ============================================================================

/// Remove cmd /c wrapper from server config (only for stdio type)
///
/// Input:  {"type": "stdio", "command": "cmd", "args": ["/c", "npx", "-y", "foo"]}
/// Output: {"type": "stdio", "command": "npx", "args": ["-y", "foo"]}
///
/// http/sse types are returned unchanged.
pub fn unwrap_cmd_c(server_config: &Value) -> Value {
    let Some(obj) = server_config.as_object() else {
        return server_config.clone();
    };

    // Only process stdio type
    let server_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("stdio");
    if server_type != "stdio" {
        return server_config.clone();
    }

    let command = obj.get("command").and_then(|v| v.as_str()).unwrap_or("");
    let args = obj
        .get("args")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Check if wrapped with cmd /c
    if !is_cmd_wrapped(command, &args) {
        return server_config.clone();
    }

    // Unwrap: args[1] becomes command, args[2..] become new args
    if args.len() < 2 {
        return server_config.clone();
    }

    let new_command = args[1].as_str().unwrap_or("");
    let new_args: Vec<Value> = args[2..].to_vec();

    let mut result = obj.clone();
    result.insert("command".to_string(), json!(new_command));
    result.insert("args".to_string(), json!(new_args));

    Value::Object(result)
}

/// Add cmd /c wrapper to server config (only for stdio type, only on Windows)
///
/// Input:  {"type": "stdio", "command": "npx", "args": ["-y", "foo"]}
/// Output: {"type": "stdio", "command": "cmd", "args": ["/c", "npx", "-y", "foo"]}
///
/// On non-Windows, returns the input unchanged.
/// http/sse types are returned unchanged.
/// Commands not in WINDOWS_WRAP_COMMANDS are returned unchanged.
pub fn wrap_cmd_c_for_target(server_config: &Value, should_wrap: bool) -> Value {
    if !should_wrap {
        return server_config.clone();
    }

    let Some(obj) = server_config.as_object() else {
        return server_config.clone();
    };

    // Only process stdio type
    let server_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("stdio");
    if server_type != "stdio" {
        return server_config.clone();
    }

    let command = obj.get("command").and_then(|v| v.as_str()).unwrap_or("");
    let args = obj
        .get("args")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Already wrapped?
    if is_cmd_wrapped(command, &args) {
        return server_config.clone();
    }

    // Check if command needs wrapping
    if !needs_wrap(command) {
        return server_config.clone();
    }

    // Wrap: command becomes "cmd", args become ["/c", original_command, ...original_args]
    let mut new_args = vec![json!("/c"), json!(command)];
    new_args.extend(args);

    let mut result = obj.clone();
    result.insert("command".to_string(), json!("cmd"));
    result.insert("args".to_string(), Value::Array(new_args));

    Value::Object(result)
}

#[cfg(windows)]
pub fn wrap_cmd_c(server_config: &Value) -> Value {
    wrap_cmd_c_for_target(server_config, true)
}

#[cfg(not(windows))]
pub fn wrap_cmd_c(server_config: &Value) -> Value {
    // On non-Windows, no wrapping needed
    server_config.clone()
}

// ============================================================================
// OpenCode Array Format Processing
// ============================================================================

/// Unwrap cmd /c from OpenCode command array format
///
/// Input:  ["cmd", "/c", "npx", "-y", "foo"]
/// Output: ["npx", "-y", "foo"]
pub fn unwrap_cmd_c_opencode_array(command_array: &[Value]) -> Vec<Value> {
    if command_array.len() < 3 {
        return command_array.to_vec();
    }

    let first = command_array[0].as_str().unwrap_or("");
    let second = command_array[1].as_str().unwrap_or("");

    if (first.eq_ignore_ascii_case("cmd") || first.eq_ignore_ascii_case("cmd.exe"))
        && second.eq_ignore_ascii_case("/c")
    {
        command_array[2..].to_vec()
    } else {
        command_array.to_vec()
    }
}

/// Wrap cmd /c for OpenCode command array format (Windows only)
///
/// Input:  ["npx", "-y", "foo"]
/// Output: ["cmd", "/c", "npx", "-y", "foo"]
pub fn wrap_cmd_c_opencode_array_for_target(
    command_array: &[Value],
    should_wrap: bool,
) -> Vec<Value> {
    if !should_wrap {
        return command_array.to_vec();
    }

    if command_array.is_empty() {
        return command_array.to_vec();
    }

    let first = command_array[0].as_str().unwrap_or("");

    // Already wrapped?
    if first.eq_ignore_ascii_case("cmd") || first.eq_ignore_ascii_case("cmd.exe") {
        return command_array.to_vec();
    }

    // Check if needs wrapping
    if !needs_wrap(first) {
        return command_array.to_vec();
    }

    let mut result = vec![json!("cmd"), json!("/c")];
    result.extend(command_array.iter().cloned());
    result
}

#[cfg(windows)]
pub fn wrap_cmd_c_opencode_array(command_array: &[Value]) -> Vec<Value> {
    wrap_cmd_c_opencode_array_for_target(command_array, true)
}

#[cfg(not(windows))]
pub fn wrap_cmd_c_opencode_array(command_array: &[Value]) -> Vec<Value> {
    command_array.to_vec()
}

// ============================================================================
// Full Config File Processing (for backup restore and WSL sync)
// ============================================================================

/// Process Claude JSON config file content
///
/// - wrap=true: Add cmd /c (restore to Windows)
/// - wrap=false: Remove cmd /c (restore to Mac/Linux/WSL)
///
/// `path_transform` is applied to each stdio server's final command (after the
/// wrap/unwrap step). WSL sync passes a `windows_to_wsl_path` closure so a
/// Windows-bound `C:\...`/`~/`/`%APPDATA%` command becomes `/mnt/c/...`; SSH
/// and other callers pass identity to keep the command unchanged.
pub fn process_claude_json(
    content: &str,
    wrap: bool,
    path_transform: &impl Fn(&str) -> String,
) -> Result<String, String> {
    if content.trim().is_empty() {
        return Ok(content.to_string());
    }

    let mut root: Value =
        json5::from_str(content).map_err(|e| format!("Failed to parse Claude JSON: {}", e))?;

    // Process mcpServers field
    if let Some(mcp_servers) = root.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
        for (_name, server_config) in mcp_servers.iter_mut() {
            let processed = if wrap {
                wrap_cmd_c(server_config)
            } else {
                unwrap_cmd_c(server_config)
            };
            *server_config = processed;
            transform_stdio_command(server_config, path_transform);
        }
    }

    serde_json::to_string_pretty(&root).map_err(|e| format!("Failed to serialize JSON: {}", e))
}

/// Process ZCode JSON config file content.
///
/// ZCode keeps its servers under `mcp.servers` (like OpenCode 2.x) but uses the
/// Claude-style server shape — `type: "stdio"` with a string `command` plus
/// `args` — so the Claude unwrap logic applies, only at a nested path.
pub fn process_zcode_json(
    content: &str,
    wrap: bool,
    path_transform: &impl Fn(&str) -> String,
) -> Result<String, String> {
    if content.trim().is_empty() {
        return Ok(content.to_string());
    }

    let mut root: Value =
        json5::from_str(content).map_err(|e| format!("Failed to parse ZCode JSON: {}", e))?;

    if let Some(servers) = root
        .get_mut("mcp")
        .and_then(|v| v.as_object_mut())
        .and_then(|mcp| mcp.get_mut("servers"))
        .and_then(|v| v.as_object_mut())
    {
        for server_config in servers.values_mut() {
            *server_config = if wrap {
                wrap_cmd_c(server_config)
            } else {
                unwrap_cmd_c(server_config)
            };
            transform_stdio_command(server_config, path_transform);
        }
    }

    serde_json::to_string_pretty(&root).map_err(|e| format!("Failed to serialize JSON: {}", e))
}

/// Process OpenCode JSON/JSONC config file content
///
/// OpenCode format: type=local, command=array
pub fn process_opencode_json(
    content: &str,
    wrap: bool,
    path_transform: &impl Fn(&str) -> String,
) -> Result<String, String> {
    if content.trim().is_empty() {
        return Ok(content.to_string());
    }

    let mut root: Value =
        json5::from_str(content).map_err(|e| format!("Failed to parse OpenCode JSON: {}", e))?;

    // OpenCode 1.x keeps its servers directly under `mcp`; 2.x nests them under
    // `mcp.servers` (the shape the OpenCode page's migration switch declares).
    // Normalize whichever map holds them, so a WSL/SSH copy of either generation
    // still loses the Windows-only `cmd /c` wrapper.
    if let Some(mcp) = root.get_mut("mcp").and_then(|v| v.as_object_mut()) {
        let servers = match mcp.get_mut("servers") {
            Some(Value::Object(servers)) => servers,
            _ => mcp,
        };

        for (name, server_config) in servers.iter_mut() {
            // Skip non-object entries and special fields
            if name == "enabled" || name == "disabled" {
                continue;
            }

            let Some(obj) = server_config.as_object_mut() else {
                continue;
            };

            // Only process local type (equivalent to stdio)
            let server_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("local");
            if server_type != "local" {
                continue;
            }

            // OpenCode uses command as array
            if let Some(cmd_arr) = obj.get("command").and_then(|v| v.as_array()).cloned() {
                let processed = if wrap {
                    wrap_cmd_c_opencode_array(&cmd_arr)
                } else {
                    unwrap_cmd_c_opencode_array(&cmd_arr)
                };
                obj.insert("command".to_string(), Value::Array(processed));
            }

            transform_stdio_command(server_config, path_transform);
        }
    }

    serde_json::to_string_pretty(&root).map_err(|e| format!("Failed to serialize JSON: {}", e))
}

/// Process Codex TOML config file content
///
/// Codex format: [mcp_servers.name] with command and args fields
pub fn process_codex_toml(
    content: &str,
    wrap: bool,
    path_transform: &impl Fn(&str) -> String,
) -> Result<String, String> {
    if content.trim().is_empty() {
        return Ok(content.to_string());
    }

    let mut doc: toml_edit::DocumentMut = content
        .parse()
        .map_err(|e| format!("Failed to parse Codex TOML: {}", e))?;

    // Process mcp_servers table
    if let Some(mcp_servers) = doc.get_mut("mcp_servers").and_then(|v| v.as_table_mut()) {
        for (_name, server_item) in mcp_servers.iter_mut() {
            let Some(server) = server_item.as_table_mut() else {
                continue;
            };

            // Only process stdio type
            let server_type = server
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("stdio");
            if server_type != "stdio" {
                continue;
            }

            let command = server
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let args: Vec<String> = server
                .get("args")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();

            if wrap {
                // Wrap cmd /c
                if !command.eq_ignore_ascii_case("cmd")
                    && !command.eq_ignore_ascii_case("cmd.exe")
                    && needs_wrap(&command)
                {
                    server["command"] = toml_edit::value("cmd");
                    let mut new_args = toml_edit::Array::new();
                    new_args.push("/c");
                    new_args.push(&command);
                    for arg in &args {
                        new_args.push(arg.as_str());
                    }
                    server["args"] = toml_edit::value(new_args);
                }
            } else {
                // Unwrap cmd /c
                if (command.eq_ignore_ascii_case("cmd") || command.eq_ignore_ascii_case("cmd.exe"))
                    && args
                        .first()
                        .map(|s| s.eq_ignore_ascii_case("/c"))
                        .unwrap_or(false)
                    && args.len() >= 2
                {
                    server["command"] = toml_edit::value(&args[1]);
                    let mut new_args = toml_edit::Array::new();
                    for arg in &args[2..] {
                        new_args.push(arg.as_str());
                    }
                    server["args"] = toml_edit::value(new_args);
                }
            }

            // Transform the stdio command via the supplied closure (e.g. WSL
            // `windows_to_wsl_path`). Read the final command back after the
            // wrap/unwrap step so we always transform the real executable.
            if let Some(cmd) = server.get("command").and_then(|v| v.as_str()) {
                let transformed = path_transform(cmd);
                if transformed != cmd {
                    server["command"] = toml_edit::value(&transformed);
                }
            }
        }
    }

    Ok(doc.to_string())
}

// ============================================================================
// Hermes YAML / dsh Cordis patch processing (WSL/SSH targets are Linux)
// ============================================================================

/// The npm package dsh's cordis plugin rows use for MCP servers. Kept in sync
/// with `crate::coding::mcp::cordis_patch::DSH_MCP_PACKAGE`.
const DSH_MCP_PACKAGE: &str = "@deepseek-ai/dsh-mcp-client";

/// If `command` is a `cmd /c` wrapper (`cmd|cmd.exe` + first arg `/c`), return
/// the unwrapped command and remaining args. Otherwise `None`.
fn unwrap_wrapped_stdio(command: &str, args: &[Value]) -> Option<(String, Vec<Value>)> {
    if !is_cmd_wrapped(command, args) || args.len() < 2 {
        return None;
    }
    let new_command = args[1].as_str()?.to_string();
    let new_args = args[2..].to_vec();
    Some((new_command, new_args))
}

/// Apply `path_transform` to a stdio server's command in place.
///
/// Handles both the string-command form (Claude/Codex: `{"command": "..."}`) and
/// the array-command form (OpenCode: `{"command": ["npx", "-y", ...]}` — only
/// the first element, the executable, is transformed). Non-stdio/local servers
/// and commands that don't change are left untouched.
fn transform_stdio_command(server: &mut Value, path_transform: &impl Fn(&str) -> String) {
    let Some(obj) = server.as_object_mut() else {
        return;
    };
    let server_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("stdio");
    if server_type != "stdio" && server_type != "local" {
        return;
    }

    // String command form.
    if let Some(cmd) = obj.get("command").and_then(|v| v.as_str()) {
        let transformed = path_transform(cmd);
        if transformed != cmd {
            obj.insert("command".to_string(), Value::String(transformed));
        }
        return;
    }

    // Array command form (OpenCode): transform only the executable (first element).
    if let Some(arr) = obj.get_mut("command").and_then(|v| v.as_array_mut()) {
        if let Some(first) = arr.first_mut() {
            if let Some(cmd) = first.as_str() {
                let transformed = path_transform(cmd);
                if transformed != cmd {
                    *first = Value::String(transformed);
                }
            }
        }
    }
}

/// Strip `cmd /c` wrappers from a Hermes `config.yaml`'s `mcp_servers:`
/// section (WSL/SSH sync copies the Windows-authored file to a Linux target,
/// where `cmd` does not exist). Only the `mcp_servers:` section is rewritten;
/// comments and unrelated top-level sections are preserved byte-for-byte.
///
/// Returns the input unchanged when there is nothing to strip or the file has
/// no `mcp_servers:` section.
///
/// `path_transform` is applied to each stdio server's final command after the
/// `cmd /c` strip step (e.g. WSL `windows_to_wsl_path`). Pass identity to leave
/// commands untouched (SSH).
pub fn process_hermes_yaml_mcp_servers(
    content: &str,
    path_transform: &impl Fn(&str) -> String,
) -> Result<String, String> {
    if content.trim().is_empty() {
        return Ok(content.to_string());
    }

    // Old section-append tooling can leave duplicate top-level YAML sections,
    // which serde_yaml rejects. Heal before parsing so WSL/SSH post-processing
    // works on the same configs that the Hermes editor already accepts.
    let healed = super::yaml_sync::deduplicate_top_level_keys(content);
    let value: serde_yaml::Value = serde_yaml::from_str(&healed)
        .map_err(|e| format!("Failed to parse Hermes config.yaml: {}", e))?;
    let root: Value = serde_json::to_value(&value)
        .map_err(|e| format!("Failed to convert Hermes config.yaml: {}", e))?;

    let Some(mcp_servers) = root.get("mcp_servers").and_then(|v| v.as_object()) else {
        // No mcp_servers section — nothing to strip.
        return Ok(content.to_string());
    };

    let mut new_servers = serde_json::Map::new();
    let mut changed = false;
    for (name, spec) in mcp_servers {
        let Some(obj) = spec.as_object() else {
            new_servers.insert(name.clone(), spec.clone());
            continue;
        };
        let command = obj.get("command").and_then(|v| v.as_str()).unwrap_or("");
        let args = obj
            .get("args")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut updated = obj.clone();
        let mut local_changed = false;
        if let Some((new_command, new_args)) = unwrap_wrapped_stdio(command, &args) {
            updated.insert("command".to_string(), json!(new_command));
            updated.insert("args".to_string(), Value::Array(new_args));
            local_changed = true;
        }
        // Transform the (possibly unwrapped) command path.
        if let Some(cmd) = updated.get("command").and_then(|v| v.as_str()) {
            let transformed = path_transform(cmd);
            if transformed != cmd {
                updated.insert("command".to_string(), json!(transformed));
                local_changed = true;
            }
        }
        if local_changed {
            changed = true;
        }
        new_servers.insert(name.clone(), Value::Object(updated));
    }

    if !changed {
        return Ok(content.to_string());
    }

    super::yaml_sync::replace_yaml_section(&healed, "mcp_servers", &Value::Object(new_servers))
}

/// Strip `cmd /c` wrappers from a dsh `cordis.patch.yml` (a YAML array of
/// plugin ops; MCP servers are `insert` rows of `@deepseek-ai/dsh-mcp-client`).
///
/// Returns the input unchanged when there is nothing to strip.
///
/// `path_transform` is applied to each dsh MCP entry's final command after the
/// `cmd /c` strip step (e.g. WSL `windows_to_wsl_path`). Pass identity to leave
/// commands untouched (SSH).
pub fn process_cordis_patch_yaml(
    content: &str,
    path_transform: &impl Fn(&str) -> String,
) -> Result<String, String> {
    if content.trim().is_empty() {
        return Ok(content.to_string());
    }

    // Keep `!!js` expressions: serde_yaml resolves that standard tag away, so
    // parse the `!js` spelling instead and restore `!!js` when writing back.
    let normalized = super::yaml_sync::rewrite_unquoted_tag(content, "!!js", "!js");
    let value: serde_yaml::Value = serde_yaml::from_str(&normalized)
        .map_err(|e| format!("Failed to parse cordis.patch.yml: {}", e))?;
    let array = value
        .as_sequence()
        .ok_or_else(|| "cordis.patch.yml must be a YAML array".to_string())?;

    let mut changed = false;
    let mut new_array: Vec<Value> = Vec::with_capacity(array.len());
    for op in array {
        let json_op = super::yaml_sync::yaml_to_json_lossless(op.clone());
        let Some(insert_list) = json_op.get("insert").and_then(|v| v.as_array()) else {
            new_array.push(json_op);
            continue;
        };

        let mut new_insert: Vec<Value> = Vec::with_capacity(insert_list.len());
        for entry in insert_list {
            let is_dsh_mcp = entry.get("name").and_then(|v| v.as_str()) == Some(DSH_MCP_PACKAGE);
            if !is_dsh_mcp {
                new_insert.push(entry.clone());
                continue;
            }
            let Some(config) = entry.get("config").and_then(|c| c.as_object()) else {
                new_insert.push(entry.clone());
                continue;
            };
            let command = config.get("command").and_then(|v| v.as_str()).unwrap_or("");
            let args = config
                .get("args")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            let mut new_config = config.clone();
            let mut local_changed = false;
            if let Some((new_command, new_args)) = unwrap_wrapped_stdio(command, &args) {
                new_config.insert("command".to_string(), json!(new_command));
                new_config.insert("args".to_string(), Value::Array(new_args));
                local_changed = true;
            }
            if let Some(cmd) = new_config.get("command").and_then(|v| v.as_str()) {
                let transformed = path_transform(cmd);
                if transformed != cmd {
                    new_config.insert("command".to_string(), json!(transformed));
                    local_changed = true;
                }
            }
            if local_changed {
                changed = true;
                let mut new_entry = entry.clone();
                if let Some(obj) = new_entry.as_object_mut() {
                    obj.insert("config".to_string(), Value::Object(new_config));
                }
                new_insert.push(new_entry);
            } else {
                new_insert.push(entry.clone());
            }
        }

        let mut new_op = json_op;
        if let Some(obj) = new_op.as_object_mut() {
            obj.insert("insert".to_string(), Value::Array(new_insert));
        }
        new_array.push(new_op);
    }

    if !changed {
        return Ok(content.to_string());
    }

    let yaml = super::yaml_sync::json_to_yaml_lossless(Value::Array(new_array));
    let text = serde_yaml::to_string(&yaml)
        .map_err(|e| format!("Failed to serialize cordis.patch.yml: {e}"))?;
    Ok(super::yaml_sync::rewrite_unquoted_tag(&text, "!js", "!!js"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_hermes_yaml_mcp_servers_heals_duplicate_top_level_sections() {
        let raw = r#"model:
  default: test
mcp_servers:
  fs:
    command: cmd
    args: ["/c", "npx", "-y", "@mcp/test"]
mcp_servers:
  fs:
    command: cmd
    args: ["/c", "npx", "-y", "@mcp/test"]
"#;
        let processed = process_hermes_yaml_mcp_servers(raw, &|s: &str| s.to_string())
            .expect("should heal duplicates");
        // Only one mcp_servers section should remain in the output.
        assert_eq!(processed.matches("mcp_servers:").count(), 1);
        assert!(processed.contains("command: npx"));
        assert!(!processed.contains("command: cmd"));
    }

    /// Simulate the WSL path transform: `C:\...` or `C:/...` -> `/mnt/c/...`;
    /// bare command names and already-Linux paths pass through unchanged.
    /// Mirrors the shape of `windows_to_wsl_path` without depending on real env.
    fn wsl_transform(s: &str) -> String {
        let normalized = s.replace('\\', "/");
        if normalized.len() >= 2 && normalized.as_bytes()[1] == b':' {
            let drive = normalized.chars().next().unwrap().to_lowercase();
            let rest = &normalized[2..];
            return format!("/mnt/{}{}", drive, rest);
        }
        s.to_string()
    }

    fn identity(s: &str) -> String {
        s.to_string()
    }

    #[test]
    fn process_claude_json_wsl_transforms_full_path_command() {
        let raw = r#"{"mcpServers":{"fs":{"type":"stdio","command":"C:\\Users\\x\\.fastctx\\bin\\fastctx.exe","args":["-c","C:\\Users\\x\\conf.json"]}}}"#;
        let processed = process_claude_json(raw, false, &wsl_transform).unwrap();
        let v: Value = serde_json::from_str(&processed).unwrap();
        assert_eq!(
            v["mcpServers"]["fs"]["command"],
            "/mnt/c/Users/x/.fastctx/bin/fastctx.exe"
        );
        // args must NOT be transformed
        assert_eq!(v["mcpServers"]["fs"]["args"][0], "-c");
        assert_eq!(v["mcpServers"]["fs"]["args"][1], "C:\\Users\\x\\conf.json");
    }

    #[test]
    fn process_claude_json_wsl_transform_after_cmd_unwrap() {
        // cmd /c wrapping a full path: strip cmd first, then transform the real exe.
        let raw =
            r#"{"mcpServers":{"fs":{"type":"stdio","command":"cmd","args":["/c","C:\\x.exe"]}}}"#;
        let processed = process_claude_json(raw, false, &wsl_transform).unwrap();
        let v: Value = serde_json::from_str(&processed).unwrap();
        assert_eq!(v["mcpServers"]["fs"]["command"], "/mnt/c/x.exe");
        // "/c" and the exe path are consumed by unwrap; args is now empty array
        assert!(v["mcpServers"]["fs"]["args"].as_array().unwrap().is_empty());
    }

    /// ZCode nests Claude-shaped servers one level down, under `mcp.servers`.
    /// Without a dedicated processor the sync copied `cmd /c` verbatim into a
    /// Linux target, where `cmd` does not exist.
    #[test]
    fn process_zcode_json_unwraps_nested_servers() {
        let raw = r#"{"mcp":{"servers":{"fs":{"type":"stdio","command":"cmd","args":["/c","npx","-y","pkg"]}}}}"#;
        let processed = process_zcode_json(raw, false, &identity).unwrap();
        let v: Value = serde_json::from_str(&processed).unwrap();
        assert_eq!(v["mcp"]["servers"]["fs"]["command"], "npx");
        assert_eq!(v["mcp"]["servers"]["fs"]["args"], json!(["-y", "pkg"]));
    }

    /// The rest of `cli/config.json` (hooks, features) must survive the pass.
    #[test]
    fn process_zcode_json_preserves_unrelated_keys() {
        let raw = r#"{"hooks":{"x":1},"mcp":{"servers":{"fs":{"type":"stdio","command":"npx","args":["-y"]}}}}"#;
        let processed = process_zcode_json(raw, false, &identity).unwrap();
        let v: Value = serde_json::from_str(&processed).unwrap();
        assert_eq!(v["hooks"]["x"], json!(1));
        assert_eq!(v["mcp"]["servers"]["fs"]["command"], "npx");
    }

    #[test]
    fn process_claude_json_wsl_transform_leaves_bare_command() {
        let raw = r#"{"mcpServers":{"fs":{"type":"stdio","command":"npx","args":["-y","pkg"]}}}"#;
        let processed = process_claude_json(raw, false, &wsl_transform).unwrap();
        let v: Value = serde_json::from_str(&processed).unwrap();
        assert_eq!(v["mcpServers"]["fs"]["command"], "npx");
        assert_eq!(v["mcpServers"]["fs"]["args"], json!(["-y", "pkg"]));
    }

    #[test]
    fn process_claude_json_identity_preserves_command() {
        let raw = r#"{"mcpServers":{"fs":{"type":"stdio","command":"C:\\x.exe","args":["-y"]}}}"#;
        let processed = process_claude_json(raw, false, &identity).unwrap();
        let v: Value = serde_json::from_str(&processed).unwrap();
        assert_eq!(v["mcpServers"]["fs"]["command"], "C:\\x.exe");
    }

    #[test]
    fn process_claude_json_wsl_skips_http_server() {
        let raw = r#"{"mcpServers":{"r":{"type":"http","url":"https://e.com/mcp","command":"C:\\x.exe"}}}"#;
        let processed = process_claude_json(raw, false, &wsl_transform).unwrap();
        let v: Value = serde_json::from_str(&processed).unwrap();
        // http server command (if present) must not be touched
        assert_eq!(v["mcpServers"]["r"]["url"], "https://e.com/mcp");
        assert_eq!(
            v["mcpServers"]["r"]["command"], "C:\\x.exe",
            "http servers must not be path-transformed"
        );
    }

    #[test]
    fn process_opencode_json_wsl_transforms_array_command() {
        let raw = r#"{"mcp":{"fs":{"type":"local","command":["C:\\Users\\x\\bin\\opencode.exe","-y","pkg"]}}}"#;
        let processed = process_opencode_json(raw, false, &wsl_transform).unwrap();
        let v: Value = serde_json::from_str(&processed).unwrap();
        let arr = v["mcp"]["fs"]["command"].as_array().unwrap();
        assert_eq!(arr[0], "/mnt/c/Users/x/bin/opencode.exe");
        assert_eq!(arr[1], "-y");
        assert_eq!(arr[2], "pkg");
    }

    #[test]
    fn process_opencode_json_wsl_transforms_v2_nested_array_command() {
        let raw = r#"{"mcp":{"servers":{"fs":{"type":"local","command":["cmd","/c","C:\\Users\\x\\bin\\opencode.exe","-y","pkg"]}}}}"#;
        let processed = process_opencode_json(raw, false, &wsl_transform).unwrap();
        let v: Value = serde_json::from_str(&processed).unwrap();
        // 2.x keeps servers under `mcp.servers`; the wrapper must still be
        // stripped and the Windows path rewritten for the Linux-side target.
        assert_eq!(
            v["mcp"]["servers"]["fs"]["command"],
            json!(["/mnt/c/Users/x/bin/opencode.exe", "-y", "pkg"])
        );
    }

    #[test]
    fn process_codex_toml_wsl_transforms_command() {
        let raw = r#"[mcp_servers.fs]
type = "stdio"
command = "C:\\Users\\x\\.fastctx\\bin\\fastctx.exe"
args = ["-y", "pkg"]
"#;
        let processed = process_codex_toml(raw, false, &wsl_transform).unwrap();
        let doc: toml_edit::DocumentMut = processed.parse().unwrap();
        assert_eq!(
            doc["mcp_servers"]["fs"]["command"].as_str(),
            Some("/mnt/c/Users/x/.fastctx/bin/fastctx.exe")
        );
        assert_eq!(
            doc["mcp_servers"]["fs"]["args"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>(),
            vec!["-y", "pkg"]
        );
    }

    #[test]
    fn process_hermes_yaml_wsl_transforms_command() {
        let raw = r#"mcp_servers:
  fs:
    command: C:\Users\x\.fastctx\bin\fastctx.exe
    args: ["-y", "pkg"]
"#;
        let processed = process_hermes_yaml_mcp_servers(raw, &wsl_transform).unwrap();
        assert!(processed.contains("command: /mnt/c/Users/x/.fastctx/bin/fastctx.exe"));
        assert!(processed.contains("-y"));
    }

    #[test]
    fn process_hermes_yaml_wsl_transform_after_cmd_unwrap() {
        let raw = r#"mcp_servers:
  fs:
    command: cmd
    args: ["/c", "C:\\x.exe"]
"#;
        let processed = process_hermes_yaml_mcp_servers(raw, &wsl_transform).unwrap();
        assert!(processed.contains("command: /mnt/c/x.exe"));
    }

    #[test]
    fn process_hermes_yaml_identity_no_change_for_bare_command() {
        let raw = r#"mcp_servers:
  fs:
    command: npx
    args: ["-y", "pkg"]
"#;
        // bare command, nothing to strip, identity -> unchanged -> input returned
        let processed = process_hermes_yaml_mcp_servers(raw, &identity).unwrap();
        assert_eq!(processed, raw);
    }

    #[test]
    fn process_claude_json_no_change_returns_input_unchanged() {
        let raw = r#"{"mcpServers":{"fs":{"type":"stdio","command":"npx","args":["-y","pkg"]}}}"#;
        // bare command + identity: nothing changes -> input returned string-equivalent
        let processed = process_claude_json(raw, false, &identity).unwrap();
        let v: Value = serde_json::from_str(&processed).unwrap();
        assert_eq!(v["mcpServers"]["fs"]["command"], "npx");
    }

    #[test]
    fn process_codex_toml_preserves_format_when_no_change() {
        // Grok config.toml: bare npx command (no cmd /c), identity transform.
        // Verifies toml_edit does not rearrange the file when nothing changes,
        // so the strip/convert step won't silently rewrite Grok configs.
        let raw = r#"# Grok config
[mcp_servers.fs]
type = "stdio"
command = "npx"
args = ["-y", "pkg"]
"#;
        let processed = process_codex_toml(raw, false, &identity).unwrap();
        assert_eq!(
            processed, raw,
            "process_codex_toml must not rewrite the file when nothing changes"
        );
    }

    #[test]
    fn process_codex_toml_wsl_transforms_grok_full_path_command() {
        // Grok config uses the same TOML mcp_servers structure as Codex but
        // without cmd /c wrapping. A full-path command is transformed to /mnt.
        let raw = r#"[mcp_servers.fs]
type = "stdio"
command = "C:\\Users\\x\\.fastctx\\bin\\fastctx.exe"
args = ["-y", "pkg"]
"#;
        let processed = process_codex_toml(raw, false, &wsl_transform).unwrap();
        let doc: toml_edit::DocumentMut = processed.parse().unwrap();
        assert_eq!(
            doc["mcp_servers"]["fs"]["command"].as_str(),
            Some("/mnt/c/Users/x/.fastctx/bin/fastctx.exe")
        );
    }
}
