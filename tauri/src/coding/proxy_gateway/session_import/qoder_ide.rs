//! Qoder Desktop IDE chat history, international and CN editions.
//!
//! Both editions keep an Electron local database under their own profile
//! directory:
//!
//! - international: `%APPDATA%/Qoder/SharedClientCache/cache/db/local.db`
//! - CN: `%APPDATA%/Qoder CN/SharedClientCache/cache/db/local.db`
//!
//! `chat_message` has one row per message, `token_info` is JSON with
//! `prompt_tokens` / `completion_tokens` / `cached_tokens`, `model_info` carries
//! the model, and `gmt_create` is a millisecond timestamp. The working directory
//! is on the owning `chat_session.cwd`.
//!
//! The two editions are separate tool keys, so an account signed into both is
//! never double-counted. Neither is read when the database is absent.
//!
//! Newer desktop builds store conversations as `chat_session_messages` rows
//! with an opaque `payload_json` instead. That schema is not read here: its
//! token fields have not been observed, and guessing them would import
//! fabricated usage.

use super::parsers::native_record;
use super::{
    GatewayUsageTool, SessionUsageGranularity, SessionUsageRecord, SourceState, SqliteDbState,
    TokenUsage,
};
use rusqlite::Connection;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

fn table_names(source: &Connection) -> Result<HashSet<String>, String> {
    let mut query = source
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .map_err(|error| error.to_string())?;
    let rows = query
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(|error| error.to_string());
    rows
}

fn read_records(tool: GatewayUsageTool, path: &Path) -> Result<Vec<SessionUsageRecord>, String> {
    let source = super::hermes::open_read_only(path)?;
    let tables = table_names(&source)?;
    if !tables.contains("chat_message") {
        return Ok(Vec::new());
    }
    let columns = super::hermes::columns(&source, "chat_message")?;
    // `message_id`/`request_id` only exist on builds that record their own ids;
    // a trimmed or fixture database falls back to the rowid / NULL. The rowid
    // is an integer, so it is cast here rather than read as TEXT.
    let id = if columns.contains("message_id") {
        "m.message_id"
    } else {
        "CAST(m.rowid AS TEXT)"
    };
    let request = if columns.contains("request_id") {
        "m.request_id"
    } else {
        "NULL"
    };
    let session = if columns.contains("session_id") {
        "m.session_id"
    } else {
        "NULL"
    };
    // Each optional column is probed first: a trimmed or fixture database that
    // lacks one must degrade to NULL instead of failing the whole scan.
    let model_info = if columns.contains("model_info") {
        "m.model_info"
    } else {
        "NULL"
    };
    let created_at = if columns.contains("gmt_create") {
        "m.gmt_create"
    } else {
        "NULL"
    };
    let project = if tables.contains("chat_session") && columns.contains("session_id") {
        "(SELECT s.cwd FROM chat_session s WHERE s.session_id = m.session_id)"
    } else {
        "NULL"
    };
    let sql = format!(
        "SELECT {id}, {session}, {request}, m.token_info, {model_info}, {created_at}, {project}
         FROM chat_message m
         WHERE m.token_info IS NOT NULL AND m.token_info != ''
         ORDER BY {created_at} ASC"
    );
    let mut statement = source
        .prepare(&sql)
        .map_err(|error| format!("Failed to prepare Qoder IDE usage query: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    // One request can be written over several message rows. The reference
    // collector sums all of them and counts the distinct request id as one
    // call, so the buckets are added here too — overwriting would drop every
    // earlier row's tokens.
    let mut records = BTreeMap::<String, SessionUsageRecord>::new();
    for (id, session, request, token_info, model_info, created_at, project) in rows {
        let Ok(tokens) = serde_json::from_str::<serde_json::Value>(&token_info) else {
            continue;
        };
        let count = |key: &str| {
            tokens
                .get(key)
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        };
        let prompt = count("prompt_tokens");
        let cached = count("cached_tokens").min(prompt);
        let model = model_info
            .and_then(|info| serde_json::from_str::<serde_json::Value>(&info).ok())
            .and_then(|info| {
                info.get("model_key")
                    .or_else(|| info.get("model"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            });
        let session_id = session.unwrap_or_else(|| id.clone());
        let identity = request
            .map(|request| format!("request:{request}"))
            .unwrap_or_else(|| format!("message:{id}"));
        let created_at = created_at.map(|value| value / 1000).unwrap_or(0);
        if let Some(existing) = records.get_mut(&identity) {
            existing.usage.input_tokens =
                Some(existing.usage.input_tokens.unwrap_or(0) + prompt.saturating_sub(cached));
            existing.usage.output_tokens =
                Some(existing.usage.output_tokens.unwrap_or(0) + count("completion_tokens"));
            existing.usage.cache_read_tokens =
                Some(existing.usage.cache_read_tokens.unwrap_or(0) + cached);
            // One request id is one call however many rows it spans, matching
            // the reference collector's `COUNT(DISTINCT request_id)`.
            // A request spans several rows; its start is the earliest one.
            existing.created_at = existing.created_at.min(created_at);
            continue;
        }
        let mut record = native_record(
            tool,
            &session_id,
            &identity,
            model,
            TokenUsage {
                // The IDE's `prompt_tokens` is the provider's prompt count and
                // already includes the cached part; the importer's input bucket
                // means fresh input, so the cached tokens are removed once.
                input_tokens: Some(prompt.saturating_sub(cached)),
                output_tokens: Some(count("completion_tokens")),
                cache_read_tokens: Some(cached),
                ..Default::default()
            },
            created_at,
        );
        record.metadata.granularity = SessionUsageGranularity::Request;
        record.metadata.call_count = Some(1);
        record.project_path = project.filter(|path| !path.trim().is_empty());
        records.insert(identity, record);
    }
    Ok(records.into_values().collect())
}

pub(super) fn sync_database(
    db: &SqliteDbState,
    tool: GatewayUsageTool,
    path: &Path,
    states: &mut HashMap<String, SourceState>,
    claims: &mut HashMap<String, String>,
    now: i64,
) -> Result<super::GatewaySessionUsageImportResult, String> {
    if !path.is_file() {
        return Ok(Default::default());
    }
    // The database is only read once the file itself changed; a sync that finds
    // the recorded stamp and size still current does no work.
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    let stamp = super::modified_nanos(&metadata);
    let source_id = format!("{}:database", tool.as_str());
    let old_state = states.get(&source_id).cloned().unwrap_or_default();
    let parser_revision = super::parsers::revision(tool);
    if old_state.parser_revision == parser_revision
        && old_state.modified_nanos == stamp
        && old_state.size == metadata.len()
        && !old_state.pending
    {
        return Ok(Default::default());
    }
    let mut records = read_records(tool, path)?;
    let parsed_records = records.len();
    // Messages still streaming are imported once the database has been quiet
    // for the settle window; a row without a timestamp cannot be dated and is
    // skipped rather than pinned to "now".
    records.retain(|record| {
        record.created_at > 0 && record.created_at <= now - super::SESSION_SETTLE_SECONDS
    });
    // A deferred row must keep the source pending: otherwise the next sync sees
    // an unchanged database, skips the read, and the row never lands.
    let pending = records.len() != parsed_records;
    let next_state = SourceState {
        cost_reconciliation_fingerprint: old_state.cost_reconciliation_fingerprint.clone(),
        cost_reconciliation_sources: old_state.cost_reconciliation_sources.clone(),
        cumulative_usage: old_state.cumulative_usage.clone(),
        parser_revision,
        modified_nanos: stamp,
        size: metadata.len(),
        pending,
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

pub(super) fn cost_records(
    tool: GatewayUsageTool,
    path: &Path,
) -> Result<Vec<SessionUsageRecord>, String> {
    read_records(tool, path)
}

/// The database each edition writes, under its own profile directory: the
/// international build keeps `Qoder/SharedClientCache`, the CN build the same
/// layout under `Qoder CN` (the CN desktop's own AppData directory name).
/// Neither is read when the file is absent.
pub(super) fn database_paths(tool: GatewayUsageTool) -> Vec<PathBuf> {
    let profile = match tool {
        GatewayUsageTool::QoderIde => "Qoder",
        GatewayUsageTool::QoderIdeCn => "Qoder CN",
        _ => return Vec::new(),
    };
    let mut paths = Vec::new();
    for variable in ["APPDATA", "LOCALAPPDATA"] {
        let Some(base) = std::env::var_os(variable) else {
            continue;
        };
        let path = PathBuf::from(base)
            .join(profile)
            .join("SharedClientCache/cache/db/local.db");
        if path.is_file() {
            paths.push(path);
        }
    }
    // macOS keeps the profile under `Application Support` rather than AppData;
    // probing it on Windows is harmless because the file never exists there.
    if let Some(home) = dirs::home_dir() {
        let path = home
            .join("Library/Application Support")
            .join(profile)
            .join("SharedClientCache/cache/db/local.db");
        if path.is_file() {
            paths.push(path);
        }
    }
    paths.sort();
    paths.dedup();
    paths
}
