//! MiniMax Code runtime usage.
//!
//! `~/.minimax/v2/sqlite/runtime-state.sqlite` keeps one
//! `local_runtime_token_usage` row per turn with the four buckets side by side.
//!
//! Reasoning has no dedicated importer bucket. It is folded into the output
//! counter only when the row's own `raw.totalTokens` proves it is *not*
//! already included:
//! `totalTokens == input + output + cache_read + cache_write + reasoning`.
//! When the numbers disagree the bucket is dropped rather than guessed, which
//! keeps the total from being counted twice.
//!
//! Model attribution uses the row's `model` column; the CLI's runtime log is
//! only needed by builds that leave the column empty, where the session's
//! other rows supply the most frequent model instead.

use super::parsers::native_record;
use super::{GatewayUsageTool, SessionUsageGranularity, SessionUsageRecord, SourceState, SqliteDbState, TokenUsage};
use rusqlite::Connection;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

fn tables(source: &Connection) -> Result<std::collections::HashSet<String>, String> {
    let mut query = source
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .map_err(|error| error.to_string())?;
    let rows = query
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<std::collections::HashSet<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

fn column(columns: &std::collections::HashSet<String>, name: &str) -> String {
    if columns.contains(name) {
        name.to_string()
    } else {
        "NULL".to_string()
    }
}

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
    let source_id = "minimax:database".to_string();
    let old_state = states.get(&source_id).cloned().unwrap_or_default();
    let parser_revision = super::parsers::revision(GatewayUsageTool::Minimax);
    if old_state.parser_revision == parser_revision
        && old_state.modified_nanos == stamp
        && old_state.size == metadata.len()
    {
        return Ok(Default::default());
    }
    let mut records = read_records(path)?;
    records.retain(|record| record.created_at <= now - super::SESSION_SETTLE_SECONDS);
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
    if !tables.contains("local_runtime_token_usage") {
        return Ok(Vec::new());
    }
    let columns = super::hermes::columns(&source, "local_runtime_token_usage")?;
    if !["id", "session_id", "ts", "input_tokens", "output_tokens"]
        .iter()
        .all(|name| columns.contains(*name))
    {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT id, session_id, {}, {}, ts, input_tokens, output_tokens, {}, {}, {}, {}
         FROM local_runtime_token_usage ORDER BY id ASC",
        column(&columns, "turn_id"),
        column(&columns, "model"),
        column(&columns, "reasoning_tokens"),
        column(&columns, "cache_read_tokens"),
        column(&columns, "cache_write_tokens"),
        column(&columns, "raw"),
    );
    #[derive(Default)]
    struct Row {
        id: i64,
        session: String,
        turn: Option<String>,
        model: Option<String>,
        created_at: i64,
        input: u64,
        output: u64,
        reasoning: u64,
        cache_read: u64,
        cache_write: u64,
        raw: Option<String>,
    }
    let mut query = source.prepare(&sql).map_err(|error| error.to_string())?;
    let rows = query
        .query_map([], |row| {
            Ok(Row {
                id: row.get(0)?,
                session: row.get(1)?,
                turn: row.get::<_, Option<String>>(2)?,
                model: row
                    .get::<_, Option<String>>(3)?
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
                created_at: row
                    .get::<_, Option<i64>>(4)?
                    .unwrap_or(0),
                input: row.get::<_, Option<i64>>(5)?.unwrap_or(0).max(0) as u64,
                output: row.get::<_, Option<i64>>(6)?.unwrap_or(0).max(0) as u64,
                reasoning: row.get::<_, Option<i64>>(7)?.unwrap_or(0).max(0) as u64,
                cache_read: row.get::<_, Option<i64>>(8)?.unwrap_or(0).max(0) as u64,
                cache_write: row.get::<_, Option<i64>>(9)?.unwrap_or(0).max(0) as u64,
                raw: row.get::<_, Option<String>>(10)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut session_models = HashMap::<String, BTreeMap<String, usize>>::new();
    for row in &rows {
        if let Some(model) = &row.model {
            *session_models
                .entry(row.session.clone())
                .or_default()
                .entry(model.clone())
                .or_default() += 1;
        }
    }
    let mut records = Vec::new();
    for row in rows {
        let model = row.model.clone().or_else(|| {
            session_models.get(&row.session).and_then(|counts| {
                counts
                    .iter()
                    .max_by_key(|(_, count)| **count)
                    .map(|(model, _)| model.clone())
            })
        });
        // Reasoning is only counted when the provider's own total proves it is
        // not already inside `output_tokens`.
        let reasoning = if row.reasoning > 0 {
            let total = row
                .raw
                .as_deref()
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                .and_then(|raw| raw.get("totalTokens").and_then(Value::as_u64))
                .unwrap_or(0);
            if total == row.input + row.output + row.cache_read + row.cache_write + row.reasoning {
                row.reasoning
            } else {
                0
            }
        } else {
            0
        };
        let total = row.input + row.output + row.cache_read + row.cache_write + reasoning;
        if total == 0 {
            continue;
        }
        let identity = row
            .turn
            .clone()
            .map(|turn| format!("turn:{turn}"))
            .unwrap_or_else(|| format!("row:{}", row.id));
        let mut record = native_record(
            GatewayUsageTool::Minimax,
            &row.session,
            &identity,
            model,
            TokenUsage {
                input_tokens: Some(row.input),
                output_tokens: Some(row.output + reasoning),
                cache_read_tokens: Some(row.cache_read),
                cache_creation_tokens: Some(row.cache_write),
                ..Default::default()
            },
            row.created_at,
        );
        record.metadata.granularity = SessionUsageGranularity::Request;
        record.metadata.call_count = Some(1);
        records.push(record);
    }
    Ok(records)
}
