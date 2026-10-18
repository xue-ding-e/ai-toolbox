//! Qoder CLI and Qoder CN Agent transcripts.
//!
//! Both editions write Claude-Code-shaped JSONL under `~/.qoder/projects` and
//! `~/.qoder-cn/projects`; the CN edition is a separate tool key so the two
//! accounts never merge. Each assistant item is one model call, keyed by the
//! provider's `usage.request_id` — that id is stable across the parent file and
//! its `subagents/` children, so a shared request is counted once.
//!
//! **The four token buckets are zero on local-only routing.** Qoder's own agent
//! does not write token counts to disk; every observed row carries
//! `input_tokens: 0, output_tokens: 0, cache_read_input_tokens: 0,
//! cache_creation_input_tokens: 0` and only a non-zero `credits` value. Those
//! calls are still recorded (the request happened), but the token counters stay
//! zero rather than being invented. `cache_creation` may appear either as a flat
//! `cache_creation_input_tokens` field or as the nested
//! `cache_creation.{ephemeral_5m,ephemeral_1h}_input_tokens` pair; the flat
//! field wins when present, matching the reference collector. The transcript's
//! `input_tokens` includes both cache buckets, so the fresh input stored here
//! is `input - cache_read - cache_creation`.

use super::parsers::{
    native_record, number, read_jsonl_lenient, session_duration_ms, string, timestamp,
    timestamp_millis, ParsedSession,
};
use super::{GatewayUsageTool, SessionUsageGranularity, SessionUsageRecord, TokenUsage};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

fn cache_creation(usage: &Value) -> u64 {
    if let Some(value) = usage.get("cache_creation_input_tokens") {
        return value.as_u64().unwrap_or(0);
    }
    usage
        .get("cache_creation")
        .filter(|value| value.is_object())
        .map(|value| {
            number(value, &["ephemeral_5m_input_tokens"])
                + number(value, &["ephemeral_1h_input_tokens"])
        })
        .unwrap_or(0)
}

/// The provider's own Credit charge for one reply. Qoder's built-in routing
/// writes zero token buckets, so this is the only usage measure it records.
fn credits_of(message: &Value) -> Option<f64> {
    let usage = message.get("usage").filter(|value| value.is_object())?;
    ["credits", "original_credits"]
        .iter()
        .find_map(|key| usage.get(*key).and_then(Value::as_f64))
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn usage_of(message: &Value) -> Option<TokenUsage> {
    let usage = message.get("usage").filter(|value| value.is_object())?;
    let input = number(usage, &["input_tokens"]);
    let cache_read = number(usage, &["cache_read_input_tokens"]).min(input);
    let cache_creation = cache_creation(usage).min(input.saturating_sub(cache_read));
    Some(TokenUsage {
        // The transcript's `input_tokens` is the provider's prompt count and
        // already contains both cache buckets (the reference collector derives
        // `in = raw_in - cr - cw` for exactly this reason, and the gateway
        // parser's Claude path does the same). Storing the raw value would
        // count cached tokens a second time in fresh input.
        input_tokens: Some(
            input
                .saturating_sub(cache_read)
                .saturating_sub(cache_creation),
        ),
        output_tokens: Some(number(usage, &["output_tokens"])),
        cache_read_tokens: Some(cache_read),
        cache_creation_tokens: Some(cache_creation),
        ..Default::default()
    })
}

pub(super) fn parse(
    tool: GatewayUsageTool,
    path: &Path,
    fallback: i64,
) -> Result<ParsedSession, String> {
    let mut session_fallback = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut configured_model: Option<String> = None;
    let mut items = Vec::new();
    // The user turn that sent the current reply; Qoder's usage rows carry no
    // timing of their own, so this pair brackets the call end to end.
    let mut trigger_ms: Option<i64> = None;
    // Compaction rewrites the transcript in place, so a broken row in the middle
    // must not fail the whole read.
    let pending = read_jsonl_lenient(path, |line, value| {
        match value.get("type").and_then(Value::as_str) {
            // The session header names the model the CLI was configured with;
            // per-call rows usually repeat it, but not always.
            Some("runtime-config") => {
                if let Some(model) = value.get("model").and_then(Value::as_str) {
                    configured_model = Some(model.to_string());
                }
            }
            Some("user") | Some("assistant") => {
                if let Some(id) = value.get("sessionId").and_then(Value::as_str) {
                    session_fallback = id.to_string();
                }
                if value.get("type").and_then(Value::as_str) == Some("user") {
                    if let Some(at) = timestamp_millis(&value) {
                        trigger_ms = Some(at);
                    }
                }
                items.push((line, value));
            }
            _ => {}
        }
    })?;

    let mut records = BTreeMap::<String, SessionUsageRecord>::new();
    let mut session_project: Option<String> = None;
    for (line, item) in items {
        if item.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(message) = item.get("message").filter(|value| value.is_object()) else {
            continue;
        };
        // The credit-limit notice is not a model reply and carries no usage.
        if message.get("model").and_then(Value::as_str) == Some("<synthetic>") {
            continue;
        }
        let Some(usage) = usage_of(message) else {
            continue;
        };
        let session = string(&item, &["/sessionId"]).unwrap_or_else(|| session_fallback.clone());
        // `request_id` is the provider response id, shared by the parent
        // transcript and its subagent children; the message id is only unique
        // inside one file. The line number keeps a transcript without provider
        // ids importable, at the cost of a new identity when it is rewritten.
        let identity = string(&item, &["/message/usage/request_id", "/message/id"])
            .map(|id| format!("request:{id}"))
            .unwrap_or_else(|| format!("line:{line}"));
        let model = string(&item, &["/message/model"])
            .or_else(|| configured_model.clone())
            .unwrap_or_else(|| "unknown".to_string());
        let mut record = native_record(
            tool,
            &session,
            &identity,
            Some(model),
            usage,
            timestamp(&item).unwrap_or(fallback),
        );
        // Every snapshot of one reply is bracketed by the same user (or tool
        // result) turn, so the trigger stays until the next turn replaces it.
        record.duration_ms = match (trigger_ms, timestamp_millis(&item)) {
            (Some(start), Some(end)) => session_duration_ms(start, end),
            _ => None,
        };
        record.metadata.granularity = SessionUsageGranularity::Request;
        record.metadata.call_count = Some(1);
        record.credits = credits_of(message);
        // Every user/assistant row repeats the working directory; keep the
        // first one seen so a row that omits it still lands in its project.
        if let Some(project) = string(&item, &["/cwd", "/working_directory"]) {
            session_project = Some(project.clone());
            record.project_path = Some(project);
        } else if let Some(project) = session_project.clone() {
            record.project_path = Some(project);
        }
        // A streamed reply may be written over several rows. Keep the larger
        // counters; the project and identity do not change between them. With
        // every bucket zero this keeps the first row seen.
        let seen = records
            .get(&record.request_id)
            .map(|existing| existing.usage.total_tokens().unwrap_or(0));
        if seen.is_some_and(|seen| seen >= record.usage.total_tokens().unwrap_or(0)) {
            continue;
        }
        records.insert(record.request_id.clone(), record);
    }
    Ok(ParsedSession {
        records: records.into_values().collect(),
        pending,
        ..Default::default()
    })
}
