use super::parsers::{native_record, read_jsonl_lenient, timestamp, ParsedSession};
use super::{GatewayUsageTool, SessionUsageGranularity, TokenUsage};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Sanity bounds for the reverse-engineered fields.
///
/// Google owns this format and has no public schema, so a field renumbering
/// would silently decode garbage. A single generation cannot exceed this many
/// tokens, and no real timestamp predates 2020. Garbage is dropped rather than
/// written into a ledger that can only move forward.
const MAX_GENERATION_TOKENS: u64 = 100_000_000;
const MIN_GENERATION_TIMESTAMP: u64 = 1_577_836_800;

/// One decoded `google.protobuf` field, enough for the wire types Antigravity
/// writes (`varint`, length-delimited, fixed32/64).
struct ProtoField {
    number: u64,
    varint: Option<u64>,
    bytes: Option<Vec<u8>>,
}

fn decode_varint(data: &[u8], offset: usize) -> Option<(u64, usize)> {
    let mut value = 0u64;
    let mut shift = 0u32;
    let mut index = offset;
    // Protobuf caps a 64-bit varint at 10 bytes; an unbounded loop would turn a
    // corrupted row into an O(n²) shift.
    for _ in 0..10 {
        let byte = *data.get(index)?;
        index += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some((value, index));
        }
        shift += 7;
    }
    None
}

fn parse_proto_fields(data: &[u8]) -> Vec<ProtoField> {
    let mut fields = Vec::new();
    let mut index = 0;
    while index < data.len() {
        let Some((key, next)) = decode_varint(data, index) else {
            break;
        };
        index = next;
        let number = key >> 3;
        let wire_type = key & 0x7;
        let (varint, bytes) = match wire_type {
            0 => match decode_varint(data, index) {
                Some((value, next)) => {
                    index = next;
                    (Some(value), None)
                }
                None => break,
            },
            2 => {
                let Some((length, next)) = decode_varint(data, index) else {
                    break;
                };
                index = next;
                // A truncated slice would parse as a valid-looking submessage,
                // so an out-of-range length ends the walk instead.
                let end = match usize::try_from(length)
                    .ok()
                    .and_then(|length| index.checked_add(length))
                {
                    Some(end) if end <= data.len() => end,
                    _ => break,
                };
                let value = data[index..end].to_vec();
                index = end;
                (None, Some(value))
            }
            1 => {
                let end = match index.checked_add(8) {
                    Some(end) if end <= data.len() => end,
                    _ => break,
                };
                index = end;
                (None, None)
            }
            5 => {
                let end = match index.checked_add(4) {
                    Some(end) if end <= data.len() => end,
                    _ => break,
                };
                index = end;
                (None, None)
            }
            _ => break,
        };
        fields.push(ProtoField {
            number,
            varint,
            bytes,
        });
    }
    fields
}

/// A generation timestamp stored inside a `steps.metadata` blob
/// (`google.protobuf.Timestamp` in field 1).
fn step_timestamp(metadata: &[u8]) -> Option<u64> {
    for field in parse_proto_fields(metadata) {
        if field.number != 1 {
            continue;
        }
        let Some(inner) = field.bytes else { continue };
        for sub in parse_proto_fields(&inner) {
            if sub.number != 1 {
                continue;
            }
            if let Some(value) = sub.varint.filter(|value| *value >= MIN_GENERATION_TIMESTAMP) {
                return Some(value);
            }
        }
    }
    None
}

struct Generation {
    model: String,
    input: u64,
    output: u64,
    cached: u64,
    created_at: u64,
}

/// Decodes one record out of `gen_metadata.data`.
///
/// Field numbers (reverse-engineered by tokei; no official schema):
/// - 1 = one generation record, nested:
///   - 19 = model name
///   - 4 = token buckets `{2: input (fresh), 3: output incl. thoughts,
///     5: cache read, 9: thoughts, 10: response text}`
///   - 11 / 12 = time-to-first-token / streaming duration (Duration)
///   - 9 → 4 → 1 = generation start time (seconds)
fn decode_generation(data: &[u8], fallback_timestamp: Option<u64>) -> Option<Generation> {
    let mut model = String::new();
    let mut input = 0u64;
    let mut output = 0u64;
    let mut cached = 0u64;
    let mut thoughts = 0u64;
    let mut response: Option<u64> = None;
    let mut created_at = None;
    for field in parse_proto_fields(data) {
        match field.number {
            19 => {
                if let Some(bytes) = field.bytes {
                    if let Ok(name) = String::from_utf8(bytes) {
                        if name.chars().all(|character| character.is_ascii_graphic()
                            || character == ' ')
                        {
                            model = name;
                        }
                    }
                }
            }
            4 => {
                let Some(bytes) = field.bytes else { continue };
                for token in parse_proto_fields(&bytes) {
                    let Some(value) = token.varint else { continue };
                    match token.number {
                        2 => input = value,
                        3 => output = value,
                        5 => cached = value,
                        9 => thoughts = value,
                        10 => response = Some(value),
                        _ => {}
                    }
                }
            }
            9 => {
                let Some(bytes) = field.bytes else { continue };
                for nested in parse_proto_fields(&bytes) {
                    if nested.number != 4 {
                        continue;
                    }
                    let Some(inner) = nested.bytes else { continue };
                    for stamp in parse_proto_fields(&inner) {
                        if stamp.number == 1 {
                            created_at = stamp
                                .varint
                                .filter(|value| *value >= MIN_GENERATION_TIMESTAMP);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let created_at = created_at.or(fallback_timestamp)?;
    if [input, output, cached, thoughts]
        .iter()
        .any(|value| *value > MAX_GENERATION_TOKENS)
    {
        return None;
    }
    // Field 4.3 already contains the thoughts, so adding them again would
    // double-count; the response field, when present, is the exact reply size.
    let output = response.unwrap_or_else(|| output.saturating_sub(thoughts));
    Some(Generation {
        model: if model.is_empty() {
            "unknown".to_string()
        } else {
            model
        },
        input,
        output,
        cached,
        created_at,
    })
}

/// Reads every generation out of one `conversations/<uuid>.db`.
///
/// The transcript mirror carries no token counts at all, so this database is
/// the only real usage source for Antigravity. Returns `(idx, generation)`
/// pairs in file order; a database without the table is an empty read (the
/// mirror parser still runs for session metadata).
/// Imports every `conversations/<uuid>.db` under a CLI root.
///
/// One database is one conversation. Unlike the file adapters, the row set can
/// shrink (a conversation is deleted), so missing rows are released from the
/// ledger instead of lingering as phantom usage.
pub(super) fn sync_databases(
    db: &super::SqliteDbState,
    root: &Path,
    states: &mut HashMap<String, super::SourceState>,
    claims: &mut HashMap<String, String>,
    now: i64,
) -> Result<super::GatewaySessionUsageImportResult, String> {
    let conversations = root.join("conversations");
    if !conversations.is_dir() {
        return Ok(Default::default());
    }
    let mut result = super::GatewaySessionUsageImportResult::default();
    for entry in fs::read_dir(&conversations)
        .map_err(|error| error.to_string())?
        .flatten()
    {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("db") {
            continue;
        }
        let Some(session_id) = path
            .file_stem()
            .and_then(|value| value.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        let source_id = format!("antigravity:{session_id}");
        let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
        let stamp = super::modified_nanos(&metadata);
        let old_state = states.get(&source_id).cloned().unwrap_or_default();
        if old_state.parser_revision == super::parsers::revision(GatewayUsageTool::Antigravity)
            && old_state.modified_nanos == stamp
            && old_state.size == metadata.len()
        {
            continue;
        }
        result.scanned_files += 1;
        let mut records = Vec::new();
        for (idx, record_index, generation) in read_generations(&path) {
            let created_at = generation.created_at as i64;
            if created_at > now - super::SESSION_SETTLE_SECONDS {
                continue;
            }
            let mut record = native_record(
                GatewayUsageTool::Antigravity,
                &session_id,
                &format!("gen:{idx}:{record_index}"),
                Some(generation.model),
                TokenUsage {
                    input_tokens: Some(generation.input),
                    output_tokens: Some(generation.output),
                    cache_read_tokens: Some(generation.cached),
                    cache_creation_tokens: Some(0),
                    ..Default::default()
                },
                created_at,
            );
            record.metadata.granularity = SessionUsageGranularity::Request;
            records.push(record);
        }
        let next_state = super::SourceState {
            cost_reconciliation_fingerprint: old_state.cost_reconciliation_fingerprint.clone(),
            cost_reconciliation_sources: old_state.cost_reconciliation_sources.clone(),
            cumulative_usage: old_state.cumulative_usage.clone(),
            parser_revision: super::parsers::revision(GatewayUsageTool::Antigravity),
            modified_nanos: stamp,
            size: metadata.len(),
            pending: false,
            records: old_state.records.clone(),
            codex_snapshots: Vec::new(),
        };
        let (state, changes) =
            super::persist_records(db, &source_id, next_state, records, claims, now)?;
        states.insert(source_id, state);
        result.merge(changes);
    }
    Ok(result)
}

/// The same records `sync_databases` imports, for the pricing retry.
///
/// Cost reconciliation re-parses a source to find rows whose model gained a
/// price later; the transcript mirror cannot serve that role because it no
/// longer carries usage.
pub(super) fn cost_records(path: &Path) -> Result<Vec<super::SessionUsageRecord>, String> {
    let session_id = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("unknown");
    Ok(read_generations(path)
        .into_iter()
        .map(|(idx, record_index, generation)| {
            native_record(
                GatewayUsageTool::Antigravity,
                session_id,
                &format!("gen:{idx}:{record_index}"),
                Some(generation.model),
                TokenUsage {
                    input_tokens: Some(generation.input),
                    output_tokens: Some(generation.output),
                    cache_read_tokens: Some(generation.cached),
                    cache_creation_tokens: Some(0),
                    ..Default::default()
                },
                generation.created_at as i64,
            )
        })
        .collect())
}

fn read_generations(path: &Path) -> Vec<(u64, u64, Generation)> {
    let Ok(source) = super::hermes::open_read_only(path) else {
        return Vec::new();
    };
    let Ok(mut statement) = source.prepare("SELECT idx, data FROM gen_metadata ORDER BY idx ASC")
    else {
        return Vec::new();
    };
    let step_timestamps = source
        .prepare("SELECT idx, metadata FROM steps")
        .ok()
        .map(|mut statement| {
            statement
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)? as u64, row.get::<_, Vec<u8>>(1)?))
                })
                .map(|rows| {
                    rows.flatten()
                        .filter_map(|(idx, metadata)| {
                            step_timestamp(&metadata).map(|time| (idx, time))
                        })
                        .collect::<HashMap<_, _>>()
                })
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let Ok(rows) = statement.query_map([], |row| {
        Ok((row.get::<_, i64>(0)? as u64, row.get::<_, Vec<u8>>(1)?))
    }) else {
        return Vec::new();
    };
    let mut generations = Vec::new();
    for (idx, data) in rows.flatten() {
        let fallback = parse_proto_fields(&data)
            .into_iter()
            .filter(|field| field.number == 2)
            .filter_map(|field| field.bytes)
            .filter_map(|bytes| {
                let mut values = Vec::new();
                let mut offset = 0;
                while offset < bytes.len() {
                    let (value, next) = decode_varint(&bytes, offset)?;
                    values.push(value);
                    offset = next;
                }
                Some(values)
            })
            .flatten()
            .find_map(|step| step_timestamps.get(&step).copied());
        let mut record_index = 0u64;
        for field in parse_proto_fields(&data) {
            if field.number != 1 {
                continue;
            }
            let Some(bytes) = field.bytes else { continue };
            let Some(generation) = decode_generation(&bytes, fallback) else {
                continue;
            };
            generations.push((idx, record_index, generation));
            record_index += 1;
        }
    }
    generations
}

/// Check if a path is a candidate Antigravity transcript:
/// `<cli_root>/brain/<uuid>/.system_generated/logs/transcript_full.jsonl`
/// or `<cli_root>/brain/<uuid>/.system_generated/logs/transcript.jsonl`.
pub(super) fn is_transcript(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
        return false;
    };
    if name != "transcript_full.jsonl" && name != "transcript.jsonl" {
        return false;
    }
    path.parent()
        .and_then(|p| p.file_name())
        .and_then(|v| v.to_str())
        == Some("logs")
}

/// For each conversation UUID, pick the larger mirror file between
/// `transcript_full.jsonl` and `transcript.jsonl`, matching the rule in
/// `session_manager/antigravity.rs`.
pub(super) fn select_sources(files: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut by_session: HashMap<String, PathBuf> = HashMap::new();
    for file in files {
        let Some(uuid) = extract_uuid_from_path(&file) else {
            continue;
        };
        match by_session.get(&uuid) {
            Some(existing) => {
                let existing_len = fs::metadata(existing).map(|m| m.len()).unwrap_or(0);
                let current_len = fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
                if current_len > existing_len {
                    by_session.insert(uuid, file);
                }
            }
            None => {
                by_session.insert(uuid, file);
            }
        }
    }
    by_session.into_values().collect()
}

pub(super) fn extract_uuid_from_path(path: &Path) -> Option<String> {
    let logs_dir = path.parent()?;
    let sys_dir = logs_dir.parent()?;
    let uuid_dir = sys_dir.parent()?;
    let uuid_str = uuid_dir.file_name()?.to_str()?;
    if uuid::Uuid::parse_str(uuid_str).is_ok() {
        Some(uuid_str.to_string())
    } else {
        None
    }
}

/// The transcript mirror contributes no usage of its own.
///
/// It is still read so a session is visible (started_at) even when its
/// database is missing, but token counts come exclusively from
/// [`sync_databases`]. The zero-token rows the old parser emitted only
/// inflated the request count.
pub(super) fn parse(path: &Path, _fallback: i64) -> Result<ParsedSession, String> {
    let mut started_at: Option<i64> = None;
    let _pending = read_jsonl_lenient(path, |_line_idx, value| {
        if started_at.is_none() {
            started_at = timestamp(&value);
        }
    })?;
    Ok(ParsedSession {
        started_at,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_transcript_matches_only_valid_mirror_files() {
        assert!(is_transcript(Path::new(
            "brain/11111111-2222-3333-4444-555555555555/.system_generated/logs/transcript_full.jsonl"
        )));
        assert!(is_transcript(Path::new(
            "brain/11111111-2222-3333-4444-555555555555/.system_generated/logs/transcript.jsonl"
        )));
        assert!(!is_transcript(Path::new(
            "brain/11111111-2222-3333-4444-555555555555/.system_generated/subagents/sub.json"
        )));
        assert!(!is_transcript(Path::new(
            "brain/11111111-2222-3333-4444-555555555555/.system_generated/logs/chunks/transcript/00000000.jsonl"
        )));
    }

    #[test]
    fn select_sources_picks_larger_file_for_same_session() {
        let temp = tempfile::tempdir().unwrap();
        let logs = temp
            .path()
            .join("brain")
            .join("11111111-2222-3333-4444-555555555555")
            .join(".system_generated")
            .join("logs");
        fs::create_dir_all(&logs).unwrap();

        let short_file = logs.join("transcript.jsonl");
        let long_file = logs.join("transcript_full.jsonl");

        fs::write(&short_file, "{\"short\":true}\n").unwrap();
        fs::write(
            &long_file,
            "{\"long\":true,\"content\":\"more content here\"}\n",
        )
        .unwrap();

        let selected = select_sources(vec![short_file, long_file.clone()]);
        assert_eq!(selected, vec![long_file]);
    }

    /// Encodes one varint, the way `google.protobuf` writes non-negative ints.
    fn varint(mut value: u64) -> Vec<u8> {
        let mut bytes = Vec::new();
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                bytes.push(byte);
                return bytes;
            }
            bytes.push(byte | 0x80);
        }
    }

    fn key(number: u64, wire_type: u64) -> Vec<u8> {
        varint((number << 3) | wire_type)
    }

    fn field_varint(number: u64, value: u64) -> Vec<u8> {
        [key(number, 0), varint(value)].concat()
    }

    fn field_bytes(number: u64, value: &[u8]) -> Vec<u8> {
        [key(number, 2), varint(value.len() as u64), value.to_vec()].concat()
    }

    /// One `gen_metadata.data` blob: a generation record plus a packed step
    /// reference, with the token buckets and start time nested per the
    /// reverse-engineered field numbers.
    fn generation_blob(model: &str, input: u64, output: u64, cached: u64, thoughts: u64) -> Vec<u8> {
        let tokens = [
            field_varint(2, input),
            field_varint(3, output),
            field_varint(5, cached),
            field_varint(9, thoughts),
        ]
        .concat();
        let timestamp = field_bytes(4, &field_varint(1, 1_750_000_000));
        let record = [
            field_bytes(19, model.as_bytes()),
            field_bytes(4, &tokens),
            field_bytes(9, &timestamp),
        ]
        .concat();
        [
            field_bytes(1, &record),
            // Packed references to step indexes; the timestamp above wins.
            field_bytes(2, &[1, 2, 3]),
        ]
        .concat()
    }

    #[test]
    fn proto_walker_decodes_nested_generation_fields() {
        let blob = generation_blob("gemini-2.5-pro", 1_000, 200, 400, 50);
        let fields = parse_proto_fields(&blob);
        assert_eq!(fields.len(), 2);
        let generations = fields
            .iter()
            .filter(|field| field.number == 1)
            .filter_map(|field| decode_generation(field.bytes.as_ref()?, None))
            .collect::<Vec<_>>();
        assert_eq!(generations.len(), 1);
        let generation = &generations[0];
        assert_eq!(generation.model, "gemini-2.5-pro");
        assert_eq!(generation.input, 1_000);
        // Field 4.3 already contains the 50 thoughts, and the exact response
        // field (4.10) is absent here, so the thoughts are subtracted once.
        assert_eq!(generation.output, 150);
        assert_eq!(generation.cached, 400);
        assert_eq!(generation.created_at, 1_750_000_000);
    }

    #[test]
    fn generations_outside_the_sanity_bounds_are_dropped() {
        // A tripwire that would be crossed only if the field numbers shifted.
        let broken = generation_blob("gemini-2.5-pro", MAX_GENERATION_TOKENS + 1, 1, 0, 0);
        let field = parse_proto_fields(&broken)
            .into_iter()
            .find(|field| field.number == 1)
            .unwrap();
        assert!(decode_generation(field.bytes.as_ref().unwrap(), None).is_none());
    }

    #[test]
    fn parse_transcript_no_longer_emits_zero_token_rows() {
        let temp = tempfile::tempdir().unwrap();
        let logs = temp
            .path()
            .join("brain")
            .join("11111111-2222-3333-4444-555555555555")
            .join(".system_generated")
            .join("logs");
        fs::create_dir_all(&logs).unwrap();
        let transcript = logs.join("transcript_full.jsonl");
        fs::write(
            &transcript,
            r#"{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-09-20T10:00:02Z","content":"Hello world"}
"#,
        )
        .unwrap();
        let parsed = parse(&transcript, 1_700_000_000).unwrap();
        assert!(parsed.records.is_empty());
        assert!(parsed.started_at.is_some());
    }

    fn conversation_db(root: &Path, session: &str) -> rusqlite::Connection {
        let conversations = root.join("conversations");
        fs::create_dir_all(&conversations).unwrap();
        let connection = rusqlite::Connection::open(conversations.join(format!("{session}.db")))
            .unwrap();
        connection
            .execute_batch("CREATE TABLE gen_metadata (idx INTEGER PRIMARY KEY, data BLOB)")
            .unwrap();
        connection
    }

    #[test]
    fn sync_reads_every_generation_with_fresh_cached_and_output_tokens() {
        let root = tempfile::tempdir().unwrap();
        let session = "11111111-2222-3333-4444-555555555555";
        let connection = conversation_db(root.path(), session);
        connection
            .execute(
                "INSERT INTO gen_metadata (idx, data) VALUES (1, ?1)",
                [generation_blob("gemini-2.5-pro", 1_000, 200, 400, 50)],
            )
            .unwrap();
        let db = crate::db::SqliteDbState::in_memory_for_test().unwrap();
        let mut states = HashMap::new();
        let mut claims = HashMap::new();
        let result =
            sync_databases(&db, root.path(), &mut states, &mut claims, 1_800_000_100).unwrap();
        assert_eq!(result.inserted_records, 1);
        let logs = crate::coding::proxy_gateway::usage_stats::request_logs(
            &db,
            &crate::coding::proxy_gateway::types::GatewayRequestLogFilters {
                cli_key: Some(GatewayUsageTool::Antigravity),
                ..Default::default()
            },
            0,
            10,
            true,
        )
        .unwrap();
        assert_eq!(logs.data.len(), 1);
        let row = &logs.data[0];
        assert_eq!(row.input_tokens, 1_000);
        assert_eq!(row.cache_read_tokens, 400);
        assert_eq!(row.output_tokens, 150);
        assert_eq!(row.upstream_model_id, "gemini-2.5-pro");
        // Rescanning an unchanged database must not import it again.
        let second =
            sync_databases(&db, root.path(), &mut states, &mut claims, 1_800_000_160).unwrap();
        assert_eq!(second.inserted_records, 0);
    }

    #[test]
    fn a_session_database_without_gen_metadata_is_skipped_not_fatal() {
        let root = tempfile::tempdir().unwrap();
        let conversations = root.path().join("conversations");
        fs::create_dir_all(&conversations).unwrap();
        rusqlite::Connection::open(conversations.join("empty.db")).unwrap();
        let db = crate::db::SqliteDbState::in_memory_for_test().unwrap();
        let mut states = HashMap::new();
        let mut claims = HashMap::new();
        let result =
            sync_databases(&db, root.path(), &mut states, &mut claims, 1_800_000_100).unwrap();
        assert_eq!(result.inserted_records, 0);
        assert_eq!(result.failed_files, 0);
    }
}
