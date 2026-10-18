//! Shared YAML read/write helpers for MCP config sync.
//!
//! These helpers perform serde_yaml round-trip (not comment-preserving section
//! splicing). They heal duplicate top-level keys before parsing because real
//! configs (e.g. Hermes `config.yaml`) can carry duplicates left by older
//! section-append tooling, which serde_yaml rejects outright.
//!
//! The comment-preserving section splice (`replace_yaml_section` /
//! `write_yaml_sections_with_backup`) stays in `hermes::commands` because it is
//! Hermes-specific and only used for provider/model/other-settings edits.

use std::fs;
use std::path::Path;

use serde_json::{Map, Value};

/// Check whether a line is a top-level YAML mapping key (column 0, not a
/// comment / sequence item, contains `:` followed by whitespace or EOL).
fn is_top_level_key_line(line: &str) -> bool {
    if line.is_empty() {
        return false;
    }
    let first_char = line.as_bytes()[0];
    if first_char == b' ' || first_char == b'\t' || first_char == b'#' || first_char == b'-' {
        return false;
    }
    if let Some(colon_pos) = line.find(':') {
        let after_colon = &line[colon_pos + 1..];
        after_colon.is_empty() || after_colon.starts_with([' ', '\t', '\r', '\n'])
    } else {
        false
    }
}

/// Remove duplicate top-level YAML sections, keeping the LAST occurrence of
/// each key. Older section-append tooling could leave several copies of a
/// top-level key behind, which serde_yaml rejects outright. Keep-last matches
/// PyYAML's last-wins semantics. No-op when there are no duplicates.
pub(crate) fn deduplicate_top_level_keys(raw: &str) -> String {
    use std::collections::HashMap;

    // Pass 1: locate every top-level key line as (key, byte offset).
    let mut sections: Vec<(&str, usize)> = Vec::new();
    let mut offset = 0;
    for line in raw.split('\n') {
        if is_top_level_key_line(line) {
            if let Some(colon_pos) = line.find(':') {
                sections.push((&line[..colon_pos], offset));
            }
        }
        offset += line.len() + 1;
    }

    let mut remaining: HashMap<&str, usize> = HashMap::new();
    for (key, _) in &sections {
        *remaining.entry(key).or_insert(0) += 1;
    }
    if remaining.values().all(|&count| count <= 1) {
        return raw.to_string();
    }

    // Pass 2: re-emit, dropping every section that has a later occurrence of
    // the same key. A section spans its key line to the next top-level key
    // (or EOF). Content before the first section (comments, `---`) is kept.
    let mut result = String::with_capacity(raw.len());
    let head_end = sections
        .first()
        .map(|&(_, start)| start)
        .unwrap_or(raw.len());
    result.push_str(&raw[..head_end]);

    for (i, &(key, start)) in sections.iter().enumerate() {
        let end = sections
            .get(i + 1)
            .map(|&(_, next_start)| next_start)
            .unwrap_or(raw.len());
        let count = remaining.get_mut(key).expect("key collected in pass 1");
        *count -= 1;
        if *count > 0 {
            log::warn!(
                "YAML config: dropped duplicate top-level section '{key}' (keeping the last occurrence)"
            );
            continue;
        }
        result.push_str(&raw[start..end]);
    }

    result
}

/// Read a YAML file into a JSON `Value`. Returns an empty object when the file
/// does not exist or is empty. Heals duplicate top-level keys before parsing.
pub fn read_yaml_object_or_empty(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(Value::Object(Map::new()));
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
    if content.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let healed = deduplicate_top_level_keys(&content);
    let yaml: serde_yaml::Value = serde_yaml::from_str(&healed)
        .map_err(|error| format!("Failed to parse {}: {error}", path.display()))?;
    let parsed: Value = serde_json::to_value(yaml)
        .map_err(|error| format!("Failed to convert {}: {error}", path.display()))?;
    if parsed.is_object() {
        Ok(parsed)
    } else {
        Err(format!("{} must contain a YAML mapping", path.display()))
    }
}

/// Write bytes atomically via a same-directory temp file + rename, so a crash
/// mid-write never leaves a truncated config file.
pub fn atomic_write_bytes(path: &Path, content: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create {}: {error}", parent.display()))?;
    }
    let temp_path = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4().simple()));
    fs::write(&temp_path, content)
        .map_err(|error| format!("Failed to write temp file {}: {error}", temp_path.display()))?;
    fs::rename(&temp_path, path)
        .map_err(|error| format!("Failed to rename temp file to {}: {error}", path.display()))?;
    Ok(())
}

/// Convert a JSON value to a `serde_yaml` value (used to serialize sections).
pub fn json_value_to_yaml(value: &Value) -> Result<serde_yaml::Value, String> {
    let json_str = serde_json::to_string(value)
        .map_err(|error| format!("Failed to serialize JSON: {error}"))?;
    serde_yaml::from_str(&json_str)
        .map_err(|error| format!("Failed to convert JSON to YAML: {error}"))
}

/// Marker js-yaml's `!!js` type constructs for an expression scalar (dsh's
/// loader reads `value.__jsExpr`).
const JS_EXPR_KEY: &str = "__jsExpr";

/// Rewrite one YAML tag spelling into another, outside quoted scalars.
///
/// serde_yaml only preserves single-bang tags (`!js`) as `Value::Tagged`;
/// the standard `!!js` form is resolved away and the expression degrades to a
/// plain string. dsh's js-yaml requires `!!js`. Converting on read and back on
/// write keeps expression semantics intact. Quoted text is left alone so a
/// literal string containing the same characters is not rewritten.
pub fn rewrite_unquoted_tag(text: &str, from: &str, to: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();
    let mut quote: Option<char> = None;
    while let Some((index, character)) = chars.next() {
        if let Some(active) = quote {
            result.push(character);
            if character == '\\' && active == '"' {
                if let Some((_, escaped)) = chars.next() {
                    result.push(escaped);
                }
            } else if character == active {
                quote = None;
            }
            continue;
        }
        if character == '"' || character == '\'' {
            quote = Some(character);
            result.push(character);
            continue;
        }
        let boundary_before = index == 0
            || text[..index]
                .chars()
                .next_back()
                .is_some_and(|ch| ch.is_whitespace() || matches!(ch, '[' | '{' | ',' | '-' | ':'));
        let tag_here = text[index..].starts_with(from)
            && text[index + from.len()..]
                .chars()
                .next()
                .is_some_and(|ch| ch.is_whitespace());
        if tag_here && boundary_before {
            result.push_str(to);
            for _ in 1..from.len() {
                chars.next();
            }
            continue;
        }
        result.push(character);
    }
    result
}

/// Parse YAML into JSON while keeping `!!js` expression scalars.
///
/// serde_yaml resolves `!!js` to the standard `tag:yaml.org,2002:js` tag and
/// keeps it as a `Tagged` node, but `serde_json::to_value` flattens that node
/// to a plain string — rewriting the file would then turn an expression into
/// literal text. dsh's js-yaml schema instead constructs `{ "__jsExpr": src }`,
/// so this conversion produces that same marker and [`json_to_yaml_lossless`]
/// turns it back into a tag.
pub fn yaml_to_json_lossless(value: serde_yaml::Value) -> Value {
    match value {
        serde_yaml::Value::Tagged(tagged) => {
            let tag = tagged.tag.to_string();
            let inner = yaml_to_json_lossless(tagged.value);
            if tag == "!!js" || tag == "!js" {
                let source = inner.as_str().unwrap_or_default().to_string();
                return serde_json::json!({ JS_EXPR_KEY: source });
            }
            inner
        }
        serde_yaml::Value::Sequence(items) => {
            Value::Array(items.into_iter().map(yaml_to_json_lossless).collect())
        }
        serde_yaml::Value::Mapping(mapping) => {
            let mut object = Map::new();
            for (key, value) in mapping {
                let key = match key {
                    serde_yaml::Value::String(key) => key,
                    other => serde_yaml::to_string(&other)
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                };
                object.insert(key, yaml_to_json_lossless(value));
            }
            Value::Object(object)
        }
        other => serde_json::to_value(other).unwrap_or(Value::Null),
    }
}

/// Inverse of [`yaml_to_json_lossless`]: a one-key `{ "__jsExpr": ... }` object
/// becomes a `!!js` tagged scalar again, so dsh evaluates it as an expression.
pub fn json_to_yaml_lossless(value: Value) -> serde_yaml::Value {
    match value {
        Value::Object(object) => {
            if object.len() == 1 {
                if let Some(source) = object.get(JS_EXPR_KEY).and_then(Value::as_str) {
                    return serde_yaml::Value::Tagged(Box::new(serde_yaml::value::TaggedValue {
                        tag: serde_yaml::value::Tag::new("!!js"),
                        value: serde_yaml::Value::String(source.to_string()),
                    }));
                }
            }
            let mut mapping = serde_yaml::Mapping::new();
            for (key, value) in object {
                mapping.insert(serde_yaml::Value::String(key), json_to_yaml_lossless(value));
            }
            serde_yaml::Value::Mapping(mapping)
        }
        Value::Array(items) => serde_yaml::Value::Sequence(
            items.into_iter().map(json_to_yaml_lossless).collect(),
        ),
        other => serde_yaml::to_value(other).unwrap_or(serde_yaml::Value::Null),
    }
}

/// Serialize a top-level section `key:` + value into a YAML fragment like:
///
/// ```yaml
/// mcp_servers:
///   fs:
///     command: npx
/// ```
pub fn serialize_yaml_section(key: &str, value: &Value) -> Result<String, String> {
    let mut mapping = serde_yaml::Mapping::new();
    mapping.insert(
        serde_yaml::Value::String(key.to_string()),
        json_value_to_yaml(value)?,
    );
    serde_yaml::to_string(&serde_yaml::Value::Mapping(mapping))
        .map_err(|error| format!("Failed to serialize YAML section '{key}': {error}"))
}

/// Find the byte range `(start_inclusive, end_exclusive)` of a top-level YAML
/// section (a mapping key at column 0). Returns None when the section is absent.
pub fn find_yaml_section_range(raw: &str, section_key: &str) -> Option<(usize, usize)> {
    let target = format!("{section_key}:");
    let mut section_start = None;
    let mut offset = 0;
    for line in raw.split('\n') {
        if section_start.is_none() && is_top_level_key_line(line) && line.starts_with(&target) {
            // Verify exact match: after "key:" must be whitespace or EOL (\r for
            // CRLF files split on \n).
            let after_target = &line[target.len()..];
            if after_target.is_empty() || after_target.starts_with([' ', '\t', '\r']) {
                section_start = Some(offset);
            }
        } else if section_start.is_some() && is_top_level_key_line(line) {
            // Found the next top-level key — this is the end of our section.
            return Some((section_start.unwrap(), offset));
        }
        offset += line.len() + 1; // +1 for the \n
    }
    section_start.map(|start| (start, raw.len()))
}

/// Remove every top-level section with `section_key` from `raw`. Splices out
/// the duplicate copies an older append bug could leave behind.
pub fn remove_all_sections(raw: &str, section_key: &str) -> String {
    let mut result = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some((start, end)) = find_yaml_section_range(rest, section_key) {
        result.push_str(&rest[..start]);
        rest = &rest[end..];
    }
    result.push_str(rest);
    result
}

/// Replace a top-level YAML section in `raw`, or append it when absent.
///
/// Only the target section is touched (byte-for-byte), so comments and
/// unrelated sections elsewhere in the file survive. When the section exists,
/// any stale duplicate copies of the same key after it are dropped.
pub fn replace_yaml_section(raw: &str, section_key: &str, value: &Value) -> Result<String, String> {
    let serialized = serialize_yaml_section(section_key, value)?;

    if let Some((start, end)) = find_yaml_section_range(raw, section_key) {
        let mut result = String::with_capacity(raw.len());
        result.push_str(&raw[..start]);
        result.push_str(&serialized);
        // Drop duplicate copies of this key from the remainder.
        let remainder = remove_all_sections(&raw[end..], section_key);
        if !serialized.ends_with('\n') && !remainder.is_empty() && !remainder.starts_with('\n') {
            result.push('\n');
        }
        result.push_str(&remainder);
        Ok(result)
    } else {
        // Section not found — append at end.
        let mut result = raw.to_string();
        if !result.is_empty() && !result.ends_with('\n') {
            result.push('\n');
        }
        result.push_str(&serialized);
        if !result.ends_with('\n') {
            result.push('\n');
        }
        Ok(result)
    }
}

/// Read a YAML file as raw text, replace a single top-level section, and
/// atomically write it back. Preserves comments and unrelated sections
/// (byte-level section splice, not serde round-trip).
pub fn write_yaml_section(path: &Path, section_key: &str, value: &Value) -> Result<(), String> {
    let raw = if path.exists() {
        fs::read_to_string(path)
            .map_err(|error| format!("Failed to read {}: {error}", path.display()))?
    } else {
        String::new()
    };
    // Heal duplicate top-level keys before splicing (same reason as
    // read_yaml_object_or_empty: serde_yaml rejects duplicates and the
    // section finder keeps-last semantics).
    let healed = deduplicate_top_level_keys(&raw);
    let new_raw = replace_yaml_section(&healed, section_key, value)?;
    if new_raw == healed {
        return Ok(());
    }
    atomic_write_bytes(path, new_raw.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_missing_file_returns_empty_object() {
        let path = Path::new("/nonexistent/yaml_sync_test_missing.yaml");
        let result = read_yaml_object_or_empty(path).unwrap();
        assert!(result.is_object());
        assert!(result.as_object().unwrap().is_empty());
    }

    #[test]
    fn dedup_keeps_last_toplevel_occurrence() {
        let raw = "\
model:
  default: old
model:
  default: new
";
        let healed = deduplicate_top_level_keys(raw);
        assert_eq!(healed.matches("model:").count(), 1);
        assert!(healed.contains("default: new"));
        assert!(!healed.contains("default: old"));
    }

    #[test]
    fn dedup_no_duplicates_is_noop() {
        let raw = "model:\n  default: x\n";
        assert_eq!(deduplicate_top_level_keys(raw), raw);
    }
}
