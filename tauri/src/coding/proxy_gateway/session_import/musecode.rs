//! Muse Code session usage.
//!
//! Each run writes `session.jsonl` under a date-shaped directory chain. Only
//! `runtime.session` events whose kind is `run` and whose event kind is
//! `model_completed` carry usage:
//!
//! ```json
//! {"stream":{"id":"..."},"payload_type":"runtime.session",
//!  "payload":{"kind":"run","run_id":"...","event":{"kind":"model_completed",
//!    "usage":{"input_tokens":..., "cached_tokens":..., "output_tokens":...,
//!             "cache_write_tokens":..., "reasoning_tokens":...}}}}
//! ```
//!
//! `input_tokens` includes the cached part. Every completed record is keyed by
//! `source_run_record_id` when present; a record without one is still counted —
//! the CLI only omits it on older builds where duplicates do not occur.

use super::parsers::{
    bounded_session_duration_ms, native_record, number, read_jsonl, string, ParsedSession,
};
use super::{GatewayUsageTool, SessionUsageGranularity, SessionUsageRecord, TokenUsage};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

/// The session tree has two shapes: `sessions/<a>/<b>/<c>/<d>/session.jsonl`
/// and the shallower `sessions/<a>/session.jsonl`.
pub(super) fn is_session_file(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some("session.jsonl")
}

/// Muse time stamps are an epoch in seconds, milliseconds, or microseconds.
fn muse_timestamp(value: &Value) -> Option<i64> {
    let epoch = value.as_f64().filter(|epoch| epoch.is_finite())?;
    let seconds = if epoch > 100_000_000_000_000.0 {
        epoch / 1_000_000.0
    } else if epoch > 100_000_000_000.0 {
        epoch / 1000.0
    } else {
        epoch
    };
    Some(seconds as i64)
}

pub(super) fn parse(path: &Path, fallback: i64) -> Result<ParsedSession, String> {
    let mut session = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut run_models = HashMap::<String, String>::new();
    let mut seen_records = HashSet::<String>::new();
    let mut project_path: Option<String> = None;
    let mut records = BTreeMap::<String, SessionUsageRecord>::new();
    let pending = read_jsonl(path, |index, value| {
        if let Some(id) = string(&value, &["/stream/id"]) {
            session = id;
        }
        let Some(payload) = value.get("payload").filter(|value| value.is_object()) else {
            return;
        };
        match value.get("payload_type").and_then(Value::as_str) {
            // The session metadata record holds the workspace the CLI ran in,
            // once per transcript.
            Some("runtime.session.metadata") => {
                if project_path.is_none() {
                    project_path = payload
                        .get("record")
                        .and_then(|record| {
                            string(record, &["/workspace_root", "/cwd", "/working_directory"])
                        });
                }
            }
            Some("run.model.configured") => {
                let Some(record) = payload.get("record").filter(|value| value.is_object()) else {
                    return;
                };
                let Some(model) = record
                    .get("model_id")
                    .and_then(Value::as_str)
                    .filter(|model| !model.is_empty() && *model != "same-as-main")
                else {
                    return;
                };
                if let Some(run) = record.get("run_stream").and_then(|value| value.get("id")) {
                    if let Some(run) = run.as_str() {
                        run_models.insert(run.to_string(), model.to_string());
                    }
                }
            }
            Some("runtime.session")
                if payload.get("kind").and_then(Value::as_str) == Some("run") =>
            {
                let Some(event) = payload.get("event").filter(|value| value.is_object()) else {
                    return;
                };
                if event.get("kind").and_then(Value::as_str) != Some("model_completed") {
                    return;
                }
                let Some(usage) = event.get("usage").filter(|value| value.is_object()) else {
                    return;
                };
                let input_total = number(usage, &["input_tokens"]);
                let cached = number(usage, &["cached_tokens", "cache_read_tokens"]).min(input_total);
                let output = number(usage, &["output_tokens"]);
                let cache_write = number(usage, &["cache_write_tokens"]);
                let reasoning = number(usage, &["reasoning_tokens"]);
                let token_usage = TokenUsage {
                    input_tokens: Some(input_total.saturating_sub(cached)),
                    // Reasoning is part of the output count, as everywhere else.
                    output_tokens: Some(output),
                    cache_read_tokens: Some(cached),
                    cache_creation_tokens: Some(cache_write),
                    ..Default::default()
                };
                if input_total + output + cache_write + reasoning == 0 {
                    return;
                }
                let record_id = string(&value, &["/source_run_record_id"]);
                if let Some(record_id) = &record_id {
                    if !seen_records.insert(record_id.clone()) {
                        return;
                    }
                }
                let model = string(event, &["/model"])
                    .filter(|model| model != "same-as-main")
                    .or_else(|| {
                        payload
                            .get("run_id")
                            .and_then(Value::as_str)
                            .and_then(|run| run_models.get(run).cloned())
                    });
                // `recorded_at` is the record's own clock; fall back to the line
                // order index when a build omits it.
                let created_at = value
                    .get("recorded_at")
                    .and_then(muse_timestamp)
                    .unwrap_or(fallback);
                let identity = record_id.unwrap_or_else(|| format!("line:{index}"));
                let mut record = native_record(
                    GatewayUsageTool::Musecode,
                    &session,
                    &identity,
                    model,
                    token_usage,
                    created_at,
                );
                // The event reports its own elapsed time for the model call.
                record.duration_ms = event
                    .get("duration_ms")
                    .and_then(Value::as_u64)
                    .and_then(bounded_session_duration_ms);
                record.metadata.granularity = SessionUsageGranularity::Request;
                record.metadata.call_count = Some(1);
                record.project_path = project_path.clone();
                records.insert(record.request_id.clone(), record);
            }
            _ => {}
        }
    })?;
    Ok(ParsedSession {
        records: records.into_values().collect(),
        pending,
        ..Default::default()
    })
}

