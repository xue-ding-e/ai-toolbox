//! Devin CLI usage, read from its SQLite session store.
//!
//! `message_nodes.chat_message` holds the whole message as JSON. Assistant
//! messages carry their own token counts under `metadata.metrics`:
//!
//! ```json
//! {"role":"assistant","metadata":{"generation_model":"...","created_at":"...",
//!   "metrics":{"input_tokens":11486,"output_tokens":254,
//!              "cache_read_tokens":6450,"cache_creation_tokens":null}}}
//! ```
//!
//! `input_tokens` already excludes the cache, unlike Codex/ZCode, so the four
//! buckets map straight through. The node table is a forest: one reply can be
//! written as several sibling nodes with identical metrics, so the de-dupe key
//! is (session, original `metadata.created_at`, model, four buckets).

use super::parsers::{native_record, timestamp};
use super::{GatewayUsageTool, SessionUsageGranularity, SessionUsageRecord, SourceState, SqliteDbState, TokenUsage};
use rusqlite::Connection;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

pub(super) fn sync_database(
    db: &SqliteDbState,
    path: &Path,
    states: &mut HashMap<String, SourceState>,
    claims: &mut HashMap<String, String>,
    now: i64,
) -> Result<super::GatewaySessionUsageImportResult, String> {
    if !path.is_file() {
        return Ok(Default::default());
    }
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    let stamp = super::modified_nanos(&metadata);
    let source_id = "devin:database".to_string();
    let old_state = states.get(&source_id).cloned().unwrap_or_default();
    let parser_revision = super::parsers::revision(GatewayUsageTool::Devin);
    if old_state.parser_revision == parser_revision
        && old_state.modified_nanos == stamp
        && old_state.size == metadata.len()
    {
        return Ok(Default::default());
    }
    let mut records = Vec::new();
    for record in read_records(path)? {
        if record.created_at <= now - super::SESSION_SETTLE_SECONDS {
            records.push(record);
        }
    }
    let next_state = SourceState {
        cost_reconciliation_fingerprint: old_state.cost_reconciliation_fingerprint.clone(),
        cost_reconciliation_sources: old_state.cost_reconciliation_sources.clone(),
        cumulative_usage: old_state.cumulative_usage.clone(),
        parser_revision,
        modified_nanos: stamp,
        size: metadata.len(),
        pending: false,
        records: old_state.records.clone(),
        codex_snapshots: Vec::new(),
    };
    let mut result = super::GatewaySessionUsageImportResult {
        scanned_files: 1,
        ..Default::default()
    };
    let (state, changes) = super::persist_records(db, &source_id, next_state, records, claims, now)?;
    states.insert(source_id, state);
    result.merge(changes);
    Ok(result)
}

pub(super) fn cost_records(path: &Path) -> Result<Vec<SessionUsageRecord>, String> {
    read_records(path)
}

fn read_records(path: &Path) -> Result<Vec<SessionUsageRecord>, String> {
    let source = super::hermes::open_read_only(path)?;
    let tables = tables(&source)?;
    if !tables.contains("message_nodes") {
        return Ok(Vec::new());
    }
    let session_models = session_models(&source, &tables)?;
    let columns = super::hermes::columns(&source, "message_nodes")?;
    if !["session_id", "chat_message", "created_at"]
        .iter()
        .all(|column| columns.contains(*column))
    {
        return Ok(Vec::new());
    }
    let node = if columns.contains("node_id") {
        "node_id"
    } else {
        "rowid"
    };
    let sql = format!(
        "SELECT session_id, {node}, chat_message, created_at FROM message_nodes
         ORDER BY created_at ASC, {node} ASC"
    );
    let mut query = source.prepare(&sql).map_err(|error| error.to_string())?;
    let rows = query
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut records = Vec::new();
    let mut seen = HashSet::new();
    for row in rows.flatten() {
        // `node_id` is only needed to keep the de-dupe key stable for rows
        // without a microsecond timestamp; its text form is enough.
        let (session_id, node_id, message, row_created_at) = row;
        let Ok(root) = serde_json::from_slice::<Value>(&message) else {
            continue;
        };
        let Some(metadata) = root.get("metadata").filter(|value| value.is_object()) else {
            continue;
        };
        let Some(metrics) = metadata.get("metrics").filter(|value| value.is_object()) else {
            continue;
        };
        let input = metrics.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
        let output = metrics.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
        let cache_read = metrics
            .get("cache_read_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let cache_write = metrics
            .get("cache_creation_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if input + output + cache_read + cache_write == 0 {
            continue;
        }
        // The message's own timestamp is microsecond-precise (several replies
        // can share a row millisecond); the column is only the fallback.
        let stamp_source = metadata
            .get("created_at")
            .filter(|value| timestamp(value).is_some())
            .cloned();
        let Some(created_at) = stamp_source
            .as_ref()
            .and_then(timestamp)
            .or_else(|| {
                // Row timestamps are ISO strings in current builds; a bare
                // epoch is accepted through the same helper.
                serde_json::from_str::<Value>(&row_created_at)
                    .ok()
                    .as_ref()
                    .and_then(timestamp)
            })
        else {
            continue;
        };
        let model = metadata
            .get("generation_model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| session_models.get(&session_id).cloned());
        let identity = match &stamp_source {
            Some(stamp) => format!("{}:{stamp}", stamp),
            None => format!("node:{node_id}"),
        };
        let key = (
            identity.clone(),
            model.clone(),
            input,
            output,
            cache_read,
            cache_write,
        );
        if !seen.insert(key) {
            continue;
        }
        let mut record = native_record(
            GatewayUsageTool::Devin,
            &session_id,
            &identity,
            model,
            TokenUsage {
                input_tokens: Some(input),
                output_tokens: Some(output),
                cache_read_tokens: Some(cache_read),
                cache_creation_tokens: Some(cache_write),
                ..Default::default()
            },
            created_at,
        );
        record.metadata.granularity = SessionUsageGranularity::Request;
        record.metadata.call_count = Some(1);
        records.push(record);
    }
    Ok(records)
}

fn tables(source: &Connection) -> Result<HashSet<String>, String> {
    let mut query = source
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .map_err(|error| error.to_string())?;
    let rows = query
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

fn session_models(
    source: &Connection,
    tables: &HashSet<String>,
) -> Result<BTreeMap<String, String>, String> {
    let mut models = BTreeMap::new();
    if !tables.contains("sessions") {
        return Ok(models);
    }
    let columns = super::hermes::columns(source, "sessions")?;
    if !columns.contains("id") {
        return Ok(models);
    }
    let model = if columns.contains("model") {
        "model"
    } else {
        "NULL"
    };
    let mut query = source
        .prepare(&format!("SELECT id, {model} FROM sessions"))
        .map_err(|error| error.to_string())?;
    let rows = query
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    for (id, model) in rows {
        if let Some(model) = model.map(|value| value.trim().to_string()).filter(|value| !value.is_empty()) {
            models.insert(id, model);
        }
    }
    Ok(models)
}
