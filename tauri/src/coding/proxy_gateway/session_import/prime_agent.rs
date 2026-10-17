//! Prime Agent session usage.
//!
//! Prime Agent writes Pi-shaped JSONL transcripts under
//! `~/.prime/agent/sessions` plus RLM sub-agent runs under
//! `~/.prime/agent/session-artifacts`. The entry format is close enough to
//! Pi's that the shared entry parser reads assistant usage unchanged; the two
//! differences are handled here:
//!
//! - a `model_change` event (`{provider, modelId}`) updates the model for
//!   later entries instead of appearing on every message;
//! - the same logical session can exist in both roots, so one physical copy is
//!   chosen and the remaining events are de-duplicated by event key.

use super::parsers::{read_jsonl, string, ParsedSession};
use super::{GatewayUsageTool, SessionUsageRecord};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Prefer the most complete copy when a session exists in both roots.
///
/// The session root is the primary transcript and the artifact root is a
/// sub-agent copy; both may carry the same messages, but only the primary one
/// also carries `model_change` rows. Comparing byte size picks it without
/// having to parse both files twice.
pub(super) fn canonical_files(files: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut by_session = HashMap::<String, (PathBuf, u64)>::new();
    for path in files {
        let session = session_id_of(&path);
        let size = std::fs::metadata(&path).map(|metadata| metadata.len()).unwrap_or(0);
        match by_session.get(&session) {
            Some((_, best)) if *best >= size => {}
            _ => {
                by_session.insert(session, (path, size));
            }
        }
    }
    let mut paths = by_session
        .into_values()
        .map(|(path, _)| path)
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

/// A session id from the transcript's own header, else the file stem.
fn session_id_of(path: &Path) -> String {
    let mut session = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let _ = read_jsonl(path, |_, value| {
        if value.get("type").and_then(Value::as_str) == Some("session") {
            if let Some(id) = string(&value, &["/id"]) {
                session = id;
            }
        }
    });
    session
}

pub(super) fn parse(path: &Path, fallback: i64) -> Result<ParsedSession, String> {
    let mut session = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut model: Option<String> = None;
    let mut records = BTreeMap::<String, SessionUsageRecord>::new();
    let mut seen_events = HashSet::<String>::new();
    let pending = read_jsonl(path, |index, value| {
        match value.get("type").and_then(Value::as_str) {
            Some("session") => {
                if let Some(id) = string(&value, &["/id"]) {
                    session = id;
                }
                return;
            }
            Some("model_change") => {
                if let Some(name) = string(&value, &["/modelId", "/model"]) {
                    model = Some(name);
                }
                return;
            }
            _ => {}
        }
        // Prime Agent's sub-agent attribution events (`child_usage_attributed`)
        // carry a type the shared parser does not accept, so they never bill
        // the child's usage twice.
        let Some(mut record) = super::pi::parse_entry(
            GatewayUsageTool::PrimeAgent,
            &session,
            index,
            &value,
            fallback,
        ) else {
            return;
        };
        if record.model == "unknown" {
            if let Some(model) = &model {
                record.model = model.clone();
            }
        }
        // The same message id can appear in both session roots; one billing.
        let key = string(&value, &["/id", "/message/id", "/message/responseId"])
            .unwrap_or_else(|| format!("line:{index}"));
        if !seen_events.insert(key) {
            return;
        }
        records.insert(record.request_id.clone(), record);
    })?;
    Ok(ParsedSession {
        records: records.into_values().collect(),
        pending,
        ..Default::default()
    })
}

