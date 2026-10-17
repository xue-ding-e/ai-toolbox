//! Qwen Code usage, merged from two sources.
//!
//! Current builds append one JSONL record per request under
//! `<runtime>/usage/token-usage-*.jsonl`; an older per-session rollup stays in
//! `<root>/usage_record.jsonl`. The runtime directory is `$QWEN_RUNTIME_DIR`,
//! else `advanced.runtimeOutputDir` from `settings.json`, else `~/.qwen`.
//!
//! Per-request fields: `inputTokens` (includes the cache), `cachedTokens`,
//! `outputTokens`, `thoughtsTokens`. The rollup's per-model numbers are
//! cumulative for the whole session, so the merge keeps every request record
//! and lets the rollup contribute only the remainder the request files do not
//! cover.

use super::parsers::{native_record, number, read_jsonl, string, timestamp, ParsedSession};
use super::{GatewayUsageTool, SessionUsageGranularity, SessionUsageRecord, TokenUsage};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The runtime directory that holds `usage/token-usage-*.jsonl`.
pub(super) fn root() -> PathBuf {
    if let Some(configured) = std::env::var_os("QWEN_RUNTIME_DIR") {
        return PathBuf::from(configured);
    }
    let home = dirs::home_dir().unwrap_or_default();
    if let Ok(text) = std::fs::read_to_string(home.join(".qwen/settings.json")) {
        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            if let Some(configured) = value
                .pointer("/advanced/runtimeOutputDir")
                .and_then(Value::as_str)
                .filter(|configured| configured.starts_with('/') || configured.starts_with('~'))
            {
                return configured.strip_prefix("~/").map_or_else(
                    || PathBuf::from(configured),
                    |rest| home.join(rest),
                );
            }
        }
    }
    home.join(".qwen")
}

/// Per-request usage files and the session rollup.
pub(super) fn is_usage_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    (name.starts_with("token-usage-") && name.ends_with(".jsonl")) || name == "usage_record.jsonl"
}

fn is_request_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("token-usage-"))
}

/// Fresh input, output (thoughts folded in), cached input.
fn token_parts(values: &Value) -> (u64, u64, u64) {
    let mut input_total = number(values, &["inputTokens"]);
    let mut cached = number(values, &["cachedTokens"]);
    if input_total == 0 && cached > 0 {
        input_total = cached;
    }
    cached = cached.min(input_total);
    let output = number(values, &["outputTokens"]);
    let thoughts = number(values, &["thoughtsTokens"]);
    (input_total - cached, output + thoughts, cached)
}

fn request_record(value: &Value, fallback: i64) -> Option<SessionUsageRecord> {
    if value.get("schemaVersion").and_then(Value::as_u64) != Some(1) {
        return None;
    }
    let id = string(value, &["/id"])?;
    let session = string(value, &["/sessionId"])?;
    let model = string(value, &["/model"]).unwrap_or_else(|| "unknown".into());
    let (input, output, cached) = token_parts(value);
    if input + output + cached == 0 {
        return None;
    }
    let mut record = native_record(
        GatewayUsageTool::Qwencode,
        &session,
        &id,
        Some(model),
        TokenUsage {
            input_tokens: Some(input),
            output_tokens: Some(output),
            cache_read_tokens: Some(cached),
            ..Default::default()
        },
        timestamp(value).unwrap_or(fallback),
    );
    record.metadata.granularity = SessionUsageGranularity::Request;
    record.metadata.call_count = Some(1);
    Some(record)
}

/// One session-level record per model in the rollup.
fn summary_records(value: &Value, fallback: i64) -> Vec<SessionUsageRecord> {
    if value.get("version").and_then(Value::as_u64) != Some(1) {
        return Vec::new();
    }
    let Some(session) = string(value, &["/sessionId"]) else {
        return Vec::new();
    };
    let Some(models) = value.get("models").and_then(Value::as_object) else {
        return Vec::new();
    };
    let created_at = timestamp(value)
        .or_else(|| {
            value
                .get("startTime")
                .and_then(|time| timestamp(&serde_json::json!({ "t": time })))
        })
        .unwrap_or(fallback);
    models
        .iter()
        .filter_map(|(model, values)| {
            let (input, output, cached) = token_parts(values);
            if input + output + cached == 0 {
                return None;
            }
            let mut record = native_record(
                GatewayUsageTool::Qwencode,
                &session,
                &format!("summary:{model}"),
                Some(model.clone()),
                TokenUsage {
                    input_tokens: Some(input),
                    output_tokens: Some(output),
                    cache_read_tokens: Some(cached),
                    ..Default::default()
                },
                created_at,
            );
            record.metadata.granularity = SessionUsageGranularity::Session;
            Some(record)
        })
        .collect()
}

fn read_file(path: &Path, fallback: i64) -> Result<Vec<SessionUsageRecord>, String> {
    let mut records = Vec::new();
    let _ = read_jsonl(path, |_, value| {
        records.extend(request_record(&value, fallback));
        records.extend(summary_records(&value, fallback));
    })?;
    Ok(records)
}

pub(super) fn parse(path: &Path, fallback: i64) -> Result<ParsedSession, String> {
    Ok(ParsedSession {
        records: read_file(path, fallback)?,
        ..Default::default()
    })
}

/// Imports the merged view as one source whose cursor is the runtime tree's
/// newest write. The merge is cheap enough to redo whenever any file changes.
///
/// The ordinary per-file walk also parses these files; both write the same
/// request IDs, and this merged view runs afterwards, so the rows converge on
/// the corrected values (the rollup remainder replaces the full session total).
/// Do not remove one path without teaching the other the remainder rule: the
/// per-file walk also feeds cost reconciliation, which needs `session_files`.
pub(super) fn sync_root(
    db: &super::SqliteDbState,
    root: &Path,
    states: &mut std::collections::HashMap<String, super::SourceState>,
    claims: &mut std::collections::HashMap<String, String>,
    now: i64,
) -> Result<super::GatewaySessionUsageImportResult, String> {
    let stamp = newest_write(root);
    let source_id = "qwencode:runtime".to_string();
    let old_state = states.get(&source_id).cloned().unwrap_or_default();
    let parser_revision = super::parsers::revision(GatewayUsageTool::Qwencode);
    if stamp.is_none()
        || (old_state.parser_revision == parser_revision
            && old_state.modified_nanos == stamp.unwrap_or(0))
    {
        return Ok(Default::default());
    }
    let stamp = stamp.unwrap_or(0);
    let mut records = read_root(root, (stamp / 1_000_000_000) as i64)?;
    records.retain(|record| record.created_at <= now - super::SESSION_SETTLE_SECONDS);
    let next_state = super::SourceState {
        cost_reconciliation_fingerprint: old_state.cost_reconciliation_fingerprint.clone(),
        cost_reconciliation_sources: old_state.cost_reconciliation_sources.clone(),
        cumulative_usage: old_state.cumulative_usage.clone(),
        parser_revision,
        modified_nanos: stamp,
        size: 0,
        pending: false,
        records: old_state.records.clone(),
        codex_snapshots: Vec::new(),
    };
    let mut result = super::GatewaySessionUsageImportResult {
        scanned_files: 1,
        ..Default::default()
    };
    let (state, changes) =
        super::persist_records(db, &source_id, next_state, records, claims, now)?;
    states.insert(source_id, state);
    result.merge(changes);
    Ok(result)
}

/// Newest mtime/size under the runtime tree, as one comparable stamp.
fn newest_write(root: &Path) -> Option<u64> {
    fn walk(dir: &Path, newest: &mut Option<(u64, u64)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                walk(&path, newest);
            } else if is_usage_file(&path) {
                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|time| time.as_nanos().min(u128::from(u64::MAX)) as u64)
                    .unwrap_or(0);
                let candidate = (modified, metadata.len());
                if newest.is_none_or(|current| candidate > current) {
                    *newest = Some(candidate);
                }
            }
        }
    }
    let mut newest = None;
    walk(root, &mut newest);
    newest.map(|(modified, size)| modified ^ size.rotate_left(32))
}

/// Reads the whole Qwen runtime directory: every request file plus the rollup
/// where it only fills what the request files do not cover.
pub(super) fn read_root(root: &Path, fallback: i64) -> Result<Vec<SessionUsageRecord>, String> {
    let mut requests = BTreeMap::<String, SessionUsageRecord>::new();
    if let Ok(entries) = std::fs::read_dir(root.join("usage")) {
        let mut paths = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| is_request_file(path))
            .collect::<Vec<_>>();
        paths.sort();
        for path in paths {
            for record in read_file(&path, fallback)? {
                // A later line with the same id replaces the earlier one.
                requests.insert(record.request_id.clone(), record);
            }
        }
    }
    let requests = requests.into_values().collect::<Vec<_>>();
    let rollup = root.join("usage_record.jsonl");
    if !rollup.is_file() {
        return Ok(requests);
    }
    // Per (session, model) totals already present in the request files.
    let mut covered = BTreeMap::<(String, String), [u64; 3]>::new();
    for record in &requests {
        let entry = covered
            .entry((record.session_id.clone(), record.model.clone()))
            .or_default();
        entry[0] += record.usage.input_tokens.unwrap_or(0);
        entry[1] += record.usage.output_tokens.unwrap_or(0);
        entry[2] += record.usage.cache_read_tokens.unwrap_or(0);
    }
    let mut summaries = Vec::new();
    let _ = read_jsonl(&rollup, |_, value| {
        summaries.extend(summary_records(&value, fallback));
    })?;
    let mut merged = requests;
    for mut record in summaries {
        let key = (record.session_id.clone(), record.model.clone());
        if let Some(used) = covered.get(&key) {
            let remaining = |field: Option<u64>, used: u64| field.unwrap_or(0).saturating_sub(used);
            let input = remaining(record.usage.input_tokens, used[0]);
            let output = remaining(record.usage.output_tokens, used[1]);
            let cache_read = remaining(record.usage.cache_read_tokens, used[2]);
            // The remainder's cache detail cannot exceed its prompt.
            let prompt = input + cache_read;
            let cache_read = cache_read.min(prompt);
            record.usage.input_tokens = Some(prompt - cache_read);
            record.usage.output_tokens = Some(output);
            record.usage.cache_read_tokens = Some(cache_read);
        }
        if record.usage.total_tokens().is_some() {
            merged.push(record);
        }
    }
    Ok(merged)
}
