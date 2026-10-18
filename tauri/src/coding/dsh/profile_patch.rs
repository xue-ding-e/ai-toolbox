//! dsh profile patch layer (`profiles/<profile>/cordis.patch.yml`).
//!
//! dsh >= 0.1.7 moved live configuration out of `$DSH_HOME/settings.yaml` and
//! into the active profile's `cordis.patch.yml`: a top-level YAML array of
//! loader patch entries, each addressing one plugin row by `id`. `llm-pi-ai`
//! (provider routes) and `agent-default-model` are two such rows. The legacy
//! `settings.yaml` is imported once (renamed to `settings.yaml.imported`), so
//! editing it no longer reaches the running composition (issue #431).
//!
//! Parsing note: dsh reads this file with js-yaml and the `!!js` tag, which
//! serializes as a `{ "__jsExpr": "<source>" }` node. serde_yaml only sees
//! `!!js` as the *standard* `tag:yaml.org,2002:js` tag; its value becomes a
//! `TaggedValue` and `serde_json::to_value` turns it into a plain string,
//! losing the tag on rewrite. To keep a round trip byte-meaning-preserving,
//! this module converts a `!!js` scalar into the same `__jsExpr` marker object
//! dsh itself uses, and writes it back as `!!js` on serialization.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::coding::mcp::yaml_sync::{
    atomic_write_bytes, json_to_yaml_lossless, rewrite_unquoted_tag, yaml_to_json_lossless,
};

/// Profile name the dsh desktop app always uses (`apps/desktop/src/paths.ts`).
pub const DSH_DESKTOP_PROFILE: &str = "desktop";
/// Profile name `dsh web` initializes (`PROFILE_TEMPLATES.web`).
pub const DSH_WEB_PROFILE: &str = "web";

/// `profiles/` directory under the dsh home.
pub fn profiles_dir(root_dir: &Path) -> PathBuf {
    root_dir.join("profiles")
}

/// `profiles/<profile>/cordis.patch.yml`.
pub fn profile_patch_path(root_dir: &Path, profile: &str) -> PathBuf {
    profiles_dir(root_dir)
        .join(profile)
        .join(super::constants::DSH_PATCH_FILE)
}

/// Profiles present under `profiles/` (a directory with a `package.json`, which
/// is what `initProfile` writes and `loadProfileDirectory` requires).
pub fn list_profiles(root_dir: &Path) -> Vec<String> {
    let dir = profiles_dir(root_dir);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut profiles: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter(|entry| entry.path().join("package.json").is_file())
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
        .collect();
    profiles.sort();
    profiles
}

/// Resolve the profile whose patch layer backs the live configuration.
///
/// The desktop app always boots `profiles/desktop`; `dsh web` boots
/// `profiles/web`. Either can exist alone. When neither does (a headless-only
/// or renamed profile), fall back to the most recently modified profile so the
/// editor still lands on the layer dsh is writing.
pub fn detect_profile(root_dir: &Path, configured: Option<&str>) -> Option<String> {
    let profiles = list_profiles(root_dir);
    if let Some(configured) = configured.map(str::trim).filter(|value| !value.is_empty()) {
        if profiles.iter().any(|profile| profile == configured) {
            return Some(configured.to_string());
        }
    }
    for preferred in [DSH_DESKTOP_PROFILE, DSH_WEB_PROFILE] {
        if profiles.iter().any(|profile| profile == preferred) {
            return Some(preferred.to_string());
        }
    }
    profiles
        .into_iter()
        .max_by_key(|profile| latest_modified(&profile_patch_path(root_dir, profile)))
}

fn latest_modified(path: &Path) -> Option<std::time::SystemTime> {
    fs::metadata(path).and_then(|metadata| metadata.modified()).ok()
}

/// Read the patch list as JSON values. A missing or empty file yields an empty
/// list; a non-array root is refused so a rewrite cannot destroy it.
pub fn read_patch_entries(path: &Path) -> Result<Vec<Value>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    // serde_yaml resolves the standard `!!js` tag away; its local `!js`
    // spelling survives as a tagged node. Convert on the way in and back on
    // the way out (see `rewrite_unquoted_tag`).
    let content = rewrite_unquoted_tag(&content, "!!js", "!js");
    let yaml: serde_yaml::Value = serde_yaml::from_str(&content)
        .map_err(|error| format!("Failed to parse {}: {error}", path.display()))?;
    let parsed = yaml_to_json_lossless(yaml);
    match parsed {
        Value::Array(entries) => Ok(entries),
        _ => Err(format!(
            "{} must contain a YAML array of patch entries, but the root is not an array",
            path.display()
        )),
    }
}

/// Write the patch list back, restoring `!!js` expressions and atomically
/// replacing the file.
pub fn write_patch_entries(path: &Path, entries: &[Value]) -> Result<(), String> {
    let yaml = json_to_yaml_lossless(Value::Array(entries.to_vec()));
    let content = serde_yaml::to_string(&yaml)
        .map_err(|error| format!("Failed to serialize {}: {error}", path.display()))?;
    // serde_yaml emits a tagged scalar as `!js`; dsh reads only `!!js`.
    let content = rewrite_unquoted_tag(&content, "!js", "!!js");
    atomic_write_bytes(path, content.as_bytes())
}

/// Entry object whose `id` equals `id` (a top-level row; `insert` rows have
/// their own identity and are not merged here).
pub fn find_entry_mut<'a>(entries: &'a mut [Value], id: &str) -> Option<&'a mut Map<String, Value>> {
    entries.iter_mut().find_map(|entry| {
        let object = entry.as_object_mut()?;
        (object.get("id").and_then(Value::as_str) == Some(id)).then_some(object)
    })
}

/// `config` object of the entry with `id`, if present.
pub fn entry_config<'a>(entries: &'a [Value], entry_id: &str) -> Option<&'a Map<String, Value>> {
    for entry in entries {
        let Some(object) = entry.as_object() else {
            continue;
        };
        if object.get("id").and_then(Value::as_str) != Some(entry_id) {
            continue;
        }
        return object
            .get("config")
            .and_then(|config| config.as_object());
    }
    None
}

/// Apply a complete `config` object to the entry `id`, creating the entry when
/// absent. `name` is the plugin package; dsh's config editor writes it too, and
/// an id-targeted patch may omit it, so it is only set when provided.
pub fn set_entry_config(
    entries: &mut Vec<Value>,
    id: &str,
    name: Option<&str>,
    config: Map<String, Value>,
) {
    if let Some(entry) = find_entry_mut(entries, id) {
        entry.insert("config".to_string(), Value::Object(config));
        if let Some(name) = name {
            entry
                .entry("name".to_string())
                .or_insert_with(|| Value::String(name.to_string()));
        }
        return;
    }
    let mut entry = Map::new();
    entry.insert("id".to_string(), Value::String(id.to_string()));
    if let Some(name) = name {
        entry.insert("name".to_string(), Value::String(name.to_string()));
    }
    entry.insert("config".to_string(), Value::Object(config));
    entries.push(Value::Object(entry));
}

/// Remove the entry `id` entirely (used when its config becomes empty).
pub fn remove_entry(entries: &mut Vec<Value>, id: &str) -> bool {
    let before = entries.len();
    entries.retain(|entry| entry.get("id").and_then(Value::as_str) != Some(id));
    entries.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn js_expressions_survive_a_round_trip() {
        let source = "- id: webserver\n  config:\n    port: !!js ctx.webStartup.port ?? 3080\n";
        let dir = std::env::temp_dir().join(format!("dsh-patch-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cordis.patch.yml");
        fs::write(&path, source).unwrap();

        let entries = read_patch_entries(&path).unwrap();
        assert_eq!(
            entries,
            vec![json!({
                "id": "webserver",
                "config": { "port": { "__jsExpr": "ctx.webStartup.port ?? 3080" } }
            })]
        );

        write_patch_entries(&path, &entries).unwrap();
        let rewritten = fs::read_to_string(&path).unwrap();
        assert!(rewritten.contains("!!js ctx.webStartup.port ?? 3080"));
        // Reading it back yields the same marker, so the expression is not lost.
        assert_eq!(read_patch_entries(&path).unwrap(), entries);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn other_entries_and_literal_strings_are_preserved() {
        let entries = vec![
            json!({ "insert": [{ "id": "mcp-x", "name": "@deepseek-ai/dsh-mcp-client" }] }),
            json!({ "id": "agent-instructions", "disabled": false, "config": { "maxBytes": 262144 } }),
        ];
        let yaml = json_to_yaml_lossless(Value::Array(entries.clone()));
        let text = serde_yaml::to_string(&yaml).unwrap();
        let parsed = yaml_to_json_lossless(serde_yaml::from_str(&text).unwrap());
        assert_eq!(parsed, Value::Array(entries));
    }
}
