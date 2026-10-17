use super::{source_identity, SessionUsageRecord};
use crate::coding::proxy_gateway::types::GatewayUsageTool;
use crate::coding::proxy_gateway::usage_parser::{from_response_body, TokenUsage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct CodexSnapshot {
    pub signature: String,
    pub timestamp: i64,
    pub request_id: Option<String>,
}

#[derive(Default)]
pub(super) struct ParsedSession {
    pub pending: bool,
    pub retired_records: Vec<SessionUsageRecord>,
    pub records: Vec<SessionUsageRecord>,
    pub snapshots: Vec<CodexSnapshot>,
    pub parent_thread_id: Option<String>,
    pub started_at: Option<i64>,
}

pub(super) fn revision(cli_key: GatewayUsageTool) -> u32 {
    match cli_key {
        // Revisit cached files to retain envelope identity in the sync ledger
        // and repair matches that joined distinct, identifiable responses.
        GatewayUsageTool::Claude | GatewayUsageTool::ClaudeDesktop => 3,
        // 4: resolve a paginated parent through its whole `history_base` lineage
        // and key each page's events by rollout id, so a child no longer re-bills
        // a copied prefix and a rollover page no longer overwrites its sibling.
        // A single-file rollout keeps its old key, so re-reading is a no-op there.
        // 5: parse `token_usage_record` as the authoritative per-response
        // source and deduct the legacy `token_count` mirror that repeats it.
        GatewayUsageTool::Codex => 5,
        GatewayUsageTool::Dsh => 4,
        // 3: subtract the cached part from `input_tokens` (the column includes
        // it), prefer `completed_at`, and re-read old databases to repair the
        // doubled cache totals.
        GatewayUsageTool::Zcode => 3,
        // 3: read `logs/unified.jsonl` as the primary source; older installs
        // still import from the `updates.jsonl` mirror.
        GatewayUsageTool::Grok => 3,
        // 3: token counts come from `conversations/<uuid>.db`; the transcript
        // mirror no longer emits zero-token rows.
        GatewayUsageTool::Antigravity => 3,
        // Tools added after the original sixteen start on 2.
        GatewayUsageTool::Mimocode
        | GatewayUsageTool::PrimeAgent
        | GatewayUsageTool::Devin
        | GatewayUsageTool::Musecode
        | GatewayUsageTool::Cmdcode
        | GatewayUsageTool::Qwencode
        | GatewayUsageTool::Workbuddy
        | GatewayUsageTool::WorkbuddyAi
        | GatewayUsageTool::Codebuddy
        | GatewayUsageTool::Minimax => 2,
        _ => 2,
    }
}

#[derive(Clone, Default, PartialEq)]
struct Counters {
    input: u64,
    output: u64,
    cached: u64,
    cache_creation: u64,
}

impl Counters {
    fn parse(value: &Value) -> Option<Self> {
        let fields = value.as_object()?;
        if ![
            "input_tokens",
            "output_tokens",
            "cached_input_tokens",
            "cache_read_input_tokens",
            "cache_write_input_tokens",
            "total_tokens",
        ]
        .iter()
        .any(|field| fields.contains_key(*field))
        {
            return None;
        }
        Some(Self {
            input: number(value, &["input_tokens"]),
            output: number(value, &["output_tokens"]),
            cached: number(value, &["cached_input_tokens", "cache_read_input_tokens"]),
            cache_creation: number(value, &["cache_write_input_tokens"]),
        })
    }

    fn delta(&self, previous: &Self) -> Self {
        Self {
            input: self.input.saturating_sub(previous.input),
            output: self.output.saturating_sub(previous.output),
            cached: self.cached.saturating_sub(previous.cached),
            cache_creation: self.cache_creation.saturating_sub(previous.cache_creation),
        }
    }

    fn update_high_water(&mut self, next: &Self) {
        self.input = self.input.max(next.input);
        self.output = self.output.max(next.output);
        self.cached = self.cached.max(next.cached);
        self.cache_creation = self.cache_creation.max(next.cache_creation);
    }

    fn into_usage(self) -> TokenUsage {
        let cached = self.cached.min(self.input);
        let cache_creation = self.cache_creation.min(self.input.saturating_sub(cached));
        TokenUsage {
            input_tokens: Some(
                self.input
                    .saturating_sub(cached)
                    .saturating_sub(cache_creation),
            ),
            output_tokens: Some(self.output),
            cache_read_tokens: Some(cached),
            cache_creation_tokens: Some(cache_creation),
            ..Default::default()
        }
    }
}

pub(super) fn parse_file(
    cli_key: GatewayUsageTool,
    path: &Path,
    fallback_timestamp: i64,
) -> Result<ParsedSession, String> {
    match cli_key {
        GatewayUsageTool::Pi | GatewayUsageTool::OhMyPi | GatewayUsageTool::OmoNative => {
            return super::pi::parse(cli_key, path, fallback_timestamp)
        }
        GatewayUsageTool::Dsh => return super::dsh::parse(path, fallback_timestamp),
        GatewayUsageTool::Grok => return super::grok::parse(path, fallback_timestamp),
        GatewayUsageTool::ClaudeDesktop => return super::desktop::parse(path, fallback_timestamp),
        GatewayUsageTool::Kimi | GatewayUsageTool::KimiCli => {
            return super::kimi::parse(cli_key, path, fallback_timestamp)
        }
        GatewayUsageTool::OpenClaw => return super::open_claw::parse(path, fallback_timestamp),
        GatewayUsageTool::Zcode => return super::zcode::parse(path, fallback_timestamp),
        GatewayUsageTool::Antigravity => {
            return super::antigravity::parse(path, fallback_timestamp)
        }
        GatewayUsageTool::PrimeAgent => {
            return super::prime_agent::parse(path, fallback_timestamp)
        }
        GatewayUsageTool::Musecode => return super::musecode::parse(path, fallback_timestamp),
        GatewayUsageTool::Cmdcode => return super::cmdcode::parse(path, fallback_timestamp),
        GatewayUsageTool::Qwencode => return super::qwencode::parse(path, fallback_timestamp),
        GatewayUsageTool::Workbuddy | GatewayUsageTool::WorkbuddyAi => {
            return super::workbuddy_family::parse(cli_key, path, fallback_timestamp)
        }
        GatewayUsageTool::Codebuddy => {
            return super::codebuddy::parse(path, fallback_timestamp)
        }
        _ => {}
    }
    parse_generic_file(cli_key, path, fallback_timestamp)
}

pub(super) fn parse_generic_file(
    cli_key: GatewayUsageTool,
    path: &Path,
    fallback_timestamp: i64,
) -> Result<ParsedSession, String> {
    if cli_key == GatewayUsageTool::Codex {
        return parse_codex(path, fallback_timestamp);
    }
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut records = BTreeMap::new();
    let mut session_id = source_identity(cli_key, path);
    if path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
        for (index, line) in BufReader::new(file).lines().enumerate() {
            let line = line.map_err(|error| error.to_string())?;
            // The transcript can contain very large prompts/tool results. Only
            // decode records that can contain usage or session metadata.
            if !line.contains("\"usage\"")
                && !line.contains("\"usageMetadata\"")
                && !line.contains("\"tokens\"")
                && !line.contains("\"$set\"")
            {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                // A CLI can be in the middle of appending its last JSONL record.
                continue;
            };
            collect_values(
                cli_key,
                path,
                &value,
                &mut session_id,
                index,
                fallback_timestamp,
                &mut records,
            );
        }
    } else {
        let value: Value =
            serde_json::from_reader(BufReader::new(file)).map_err(|error| error.to_string())?;
        collect_values(
            cli_key,
            path,
            &value,
            &mut session_id,
            0,
            fallback_timestamp,
            &mut records,
        );
    }
    Ok(ParsedSession {
        records: records.into_values().collect(),
        ..Default::default()
    })
}

fn collect_values(
    cli_key: GatewayUsageTool,
    path: &Path,
    value: &Value,
    session_id: &mut String,
    index: usize,
    fallback_timestamp: i64,
    records: &mut BTreeMap<String, SessionUsageRecord>,
) {
    if let Some(id) = string(
        value,
        &["/sessionId", "/sessionID", "/session_id", "/$set/sessionId"],
    ) {
        *session_id = id;
    }
    if let Some(items) = value.as_array().or_else(|| {
        [
            "/messages",
            "/turns",
            "/entries",
            "/records",
            "/$set/messages",
        ]
        .iter()
        .find_map(|path| value.pointer(path).and_then(Value::as_array))
    }) {
        for (item_index, item) in items.iter().enumerate() {
            collect_values(
                cli_key,
                path,
                item,
                session_id,
                item_index,
                fallback_timestamp,
                records,
            );
        }
        return;
    }
    if cli_key == GatewayUsageTool::ClaudeDesktop
        && value.get("type").and_then(Value::as_str) != Some("assistant")
        && value.pointer("/message/role").and_then(Value::as_str) != Some("assistant")
    {
        return;
    }
    let Some(mut record) = parse_value(cli_key, value, session_id, index, fallback_timestamp)
    else {
        return;
    };
    let mut legacy_hasher = std::collections::hash_map::DefaultHasher::new();
    cli_key.as_str().hash(&mut legacy_hasher);
    path.to_string_lossy().hash(&mut legacy_hasher);
    index.hash(&mut legacy_hasher);
    record
        .legacy_request_ids
        .push(format!("SESSION:{:016x}", legacy_hasher.finish()));
    if let Some(previous) = records.get(&record.request_id) {
        // Claude/Gemini may write several snapshots of one response. They are
        // one invocation, and partial snapshots must not erase known counters.
        record.usage.input_tokens = record.usage.input_tokens.max(previous.usage.input_tokens);
        record.usage.output_tokens = record.usage.output_tokens.max(previous.usage.output_tokens);
        record.usage.cache_read_tokens = record
            .usage
            .cache_read_tokens
            .max(previous.usage.cache_read_tokens);
        record.usage.cache_creation_tokens = record
            .usage
            .cache_creation_tokens
            .max(previous.usage.cache_creation_tokens);
        record
            .legacy_request_ids
            .extend(previous.legacy_request_ids.iter().cloned());
    }
    records.insert(record.request_id.clone(), record);
}

pub(super) fn parse_value(
    cli_key: GatewayUsageTool,
    value: &Value,
    session_id: &str,
    index: usize,
    fallback_timestamp: i64,
) -> Option<SessionUsageRecord> {
    let usage = if cli_key == GatewayUsageTool::Gemini && value.get("tokens").is_some() {
        if value.get("type").and_then(Value::as_str) != Some("gemini") {
            return None;
        }
        let tokens = value.get("tokens")?;
        let input = number(tokens, &["input", "input_tokens"]);
        let cached = number(tokens, &["cached", "cache_read_input_tokens"]).min(input);
        TokenUsage {
            input_tokens: Some(input.saturating_sub(cached)),
            output_tokens: Some(
                number(tokens, &["output", "output_tokens"])
                    .saturating_add(number(tokens, &["thoughts"])),
            ),
            cache_read_tokens: Some(cached),
            ..Default::default()
        }
    } else if cli_key == GatewayUsageTool::OpenCode && value.get("tokens").is_some() {
        if !is_opencode_usage_row(value)
            || value.pointer("/time/completed").is_none_or(Value::is_null)
        {
            return None;
        }
        let tokens = value.get("tokens")?;
        TokenUsage {
            // OpenCode's input is already fresh, unlike Codex/Gemini input.
            input_tokens: Some(number(tokens, &["input", "input_tokens"])),
            output_tokens: Some(
                number(tokens, &["output", "output_tokens"])
                    .saturating_add(number(tokens, &["reasoning"])),
            ),
            cache_read_tokens: Some(
                tokens
                    .pointer("/cache/read")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            ),
            cache_creation_tokens: Some(
                tokens
                    .pointer("/cache/write")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            ),
            ..Default::default()
        }
    } else {
        let candidate = usage_candidate(value)?;
        from_response_body(cli_key.gateway_cli()?, &serde_json::to_vec(candidate).ok()?)
    };
    let message_id = string(
        value,
        &[
            "/message/id",
            "/message_id",
            "/messageId",
            "/id",
            "/uuid",
            "/request_id",
        ],
    );
    let model = string(
        value,
        &[
            "/model",
            "/modelID",
            "/request/model",
            "/response/model",
            "/message/model",
            "/metadata/model",
            // OpenCode 2.0 nests the native id under `model`; the object itself
            // is skipped by `string`, and its `variant` is a reasoning overlay
            // rather than part of the catalog id the V1 `modelID` carries.
            "/model/id",
        ],
    )
    .unwrap_or_else(|| "unknown".to_string());
    let has_reported_usage = usage.input_tokens.is_some()
        || usage.output_tokens.is_some()
        || usage.cache_read_tokens.is_some()
        || usage.cache_creation_tokens.is_some();
    let is_identifiable_claude_response = matches!(
        cli_key,
        GatewayUsageTool::Claude | GatewayUsageTool::ClaudeDesktop
    ) && message_id.is_some()
        && !matches!(model.as_str(), "unknown" | "<synthetic>")
        && (value.get("type").and_then(Value::as_str) == Some("assistant")
            || value.pointer("/message/role").and_then(Value::as_str) == Some("assistant")
            || value.get("role").and_then(Value::as_str) == Some("assistant"));
    if usage.total_tokens().is_none() && !(has_reported_usage && is_identifiable_claude_response) {
        return None;
    }
    let message_id = message_id.unwrap_or_else(|| format!("{session_id}:{index}"));
    let request_id = if cli_key == GatewayUsageTool::Claude {
        format!("SESSION:{message_id}")
    } else {
        format!("SESSION:{}:{session_id}:{message_id}", cli_key.as_str())
    };
    let created_at = timestamp(value).unwrap_or(fallback_timestamp);
    Some(SessionUsageRecord {
        metadata: Default::default(),
        request_id,
        legacy_request_ids: Vec::new(),
        cli_key,
        model,
        usage,
        created_at,
        session_id: session_id.to_string(),
        reported_cost_usd: value
            .get("cost")
            .and_then(Value::as_f64)
            .filter(|cost| cost.is_finite() && *cost > 0.0)
            .map(|cost| cost.to_string()),
    })
}

fn parse_codex(path: &Path, fallback_timestamp: i64) -> Result<ParsedSession, String> {
    let Some(reader) =
        crate::coding::session_manager::codex_rollout::open_rollout_reader(path, None)
    else {
        return Err(format!("Cannot open Codex rollout {}", path.display()));
    };
    let rollout_id = crate::coding::session_manager::codex_rollout::parse_rollout_path(path)
        .map(|name| name.rollout_id);
    parse_codex_lines(
        reader,
        source_identity(GatewayUsageTool::Codex, path),
        rollout_id.as_deref(),
        fallback_timestamp,
    )
}

/// Snapshots of a Codex rollout lineage, oldest segment first.
///
/// A paginated thread spreads one conversation over several rollout files
/// chained by `history_base`, and each ancestor holds only the records its
/// child did not copy. Reading the leaf alone therefore sees a suffix, not the
/// thread's real timeline — which is what a child's replay check compares
/// against. Each segment is read only up to the byte offset its child recorded.
pub(super) fn read_codex_lineage_snapshots(
    lineage: &crate::coding::session_manager::codex_rollout::Lineage,
    fallback_timestamp: i64,
) -> Result<Vec<CodexSnapshot>, String> {
    let mut snapshots = Vec::new();
    for segment in &lineage.segments {
        let Some(reader) = crate::coding::session_manager::codex_rollout::open_rollout_reader(
            &segment.path,
            segment.end_byte_offset,
        ) else {
            return Err(format!(
                "Cannot open Codex rollout {}",
                segment.path.display()
            ));
        };
        // Only the snapshots are used here, so the thread identity and the
        // per-event request ids are irrelevant; `parse_codex_lines` computes
        // them anyway to keep one parser for every Codex rollout read.
        snapshots
            .extend(parse_codex_lines(reader, String::new(), None, fallback_timestamp)?.snapshots);
    }
    Ok(snapshots)
}

fn parse_codex_lines(
    reader: Box<dyn BufRead>,
    initial_thread_id: String,
    rollout_id: Option<&str>,
    fallback_timestamp: i64,
) -> Result<ParsedSession, String> {
    let mut parsed = ParsedSession::default();
    let mut thread_id = initial_thread_id;
    let mut model = "unknown".to_string();
    let mut high_water = Counters::default();
    let mut signatures_by_source = HashMap::<String, String>::new();
    let mut previous_signature = None;
    let mut event_index = 0u64;
    let mut metadata_seen = false;
    // Dual-write bookkeeping; see `apply_codex_token_usage_record`.
    let mut response_ids = std::collections::HashSet::<String>::new();
    let mut pending_responses = Vec::<Counters>::new();
    let mut legacy_before_response: Option<(Counters, String)> = None;
    for line in reader.lines() {
        let line = line.map_err(|error| error.to_string())?;
        if !line.contains("\"session_meta\"")
            && !line.contains("\"turn_context\"")
            && !line.contains("\"token_usage_record\"")
            && !(line.contains("\"event_msg\"")
                && (line.contains("\"token_count\"") || line.contains("\"task_started\"")))
        {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(payload) = value.get("payload") else {
            continue;
        };
        match value.get("type").and_then(Value::as_str) {
            Some("session_meta") if !metadata_seen => {
                metadata_seen = true;
                if let Some(id) = string(payload, &["/id", "/thread_id", "/threadId"]) {
                    thread_id = id;
                }
                parsed.started_at = timestamp(&value);
                parsed.parent_thread_id = string(
                    payload,
                    &[
                        "/forked_from_id",
                        "/parent_thread_id",
                        "/source/subagent/thread_spawn/parent_thread_id",
                        "/source/subagent/fork/parent_thread_id",
                    ],
                );
                if let Some(name) = string(payload, &["/model"]) {
                    model = name;
                }
            }
            Some("turn_context") => {
                if let Some(name) = string(payload, &["/model", "/info/model"]) {
                    model = name;
                }
            }
            Some("event_msg")
                if payload.get("type").and_then(Value::as_str) == Some("task_started") =>
            {
                // A new turn cannot inherit a twin from the previous one. Any
                // still-unmatched authoritative record is an orphan (its mirror
                // never arrived); keep it pending rather than risk pairing it
                // with the next turn's numbers.
                legacy_before_response = None;
            }
            Some("event_msg")
                if payload.get("type").and_then(Value::as_str) == Some("token_count") =>
            {
                let Some(info) = payload.get("info").filter(|value| value.is_object()) else {
                    continue;
                };
                let total = info.get("total_token_usage").and_then(Counters::parse);
                let last = info.get("last_token_usage").and_then(Counters::parse);
                if total.is_none() && last.is_none() {
                    continue;
                }
                if let Some(name) = string(info, &["/model", "/model_name"])
                    .or_else(|| string(payload, &["/model"]))
                {
                    model = name;
                }
                let signature = counter_signature(info);
                let lane = string(payload, &["/rate_limits/limit_id"]).unwrap_or_default();
                let duplicate = total.is_some()
                    && (signatures_by_source.get(&lane) == Some(&signature)
                        || previous_signature.as_ref() == Some(&signature));
                if total.is_some() {
                    signatures_by_source.insert(lane, signature.clone());
                }
                previous_signature = Some(signature.clone());
                // The legacy notification mirrors an authoritative
                // `token_usage_record` that already counted this response. The
                // record normally arrives first, so its usage block is in
                // `pending_responses`; a mirror that consumes one must not
                // produce a second row.
                let mirrored = last.as_ref().is_some_and(|last| {
                    pending_responses
                        .iter()
                        .position(|pending| pending == last)
                        .map(|index| pending_responses.remove(index))
                        .is_some()
                });
                let usage = if duplicate || mirrored {
                    TokenUsage::default()
                } else {
                    last.clone().unwrap_or_else(|| {
                        total
                            .as_ref()
                            .map(|value| value.delta(&high_water))
                            .unwrap_or_default()
                    })
                    .into_usage()
                };
                if let Some(total) = total {
                    high_water.update_high_water(&total);
                }
                let created_at = timestamp(&value).unwrap_or(fallback_timestamp);
                let request_id = usage.total_tokens().map(|_| {
                    event_index += 1;
                    // A paginated thread's pages all carry the same thread id
                    // in `session_meta`, so the thread id alone would give two
                    // pages the same key and the newer page would overwrite the
                    // older one. The rollout id keeps each page's events
                    // distinct — but an ordinary single-file rollout has
                    // rollout id == thread id, where adding it would only
                    // re-key rows that are already correct.
                    match rollout_id.filter(|rollout_id| *rollout_id != thread_id) {
                        Some(rollout_id) => {
                            format!("SESSION:codex:{thread_id}:{rollout_id}:token:{event_index}")
                        }
                        None => format!("SESSION:codex:{thread_id}:token:{event_index}"),
                    }
                });
                if let (Some(request_id), Some(last)) = (&request_id, last) {
                    // Remember that this call was counted before its
                    // authoritative record arrived; that record will adopt this
                    // row's identity instead of billing the same call again.
                    legacy_before_response = Some((last, request_id.clone()));
                }
                if let Some(request_id) = &request_id {
                    parsed.records.push(SessionUsageRecord {
                        metadata: Default::default(),
                        request_id: request_id.clone(),
                        legacy_request_ids: Vec::new(),
                        cli_key: GatewayUsageTool::Codex,
                        model: model.clone(),
                        usage,
                        created_at,
                        session_id: thread_id.clone(),
                        reported_cost_usd: None,
                    });
                }
                parsed.snapshots.push(CodexSnapshot {
                    signature,
                    timestamp: created_at,
                    request_id,
                });
            }
            Some("token_usage_record") => {
                let Some(usage) = payload.get("usage").and_then(Counters::parse) else {
                    continue;
                };
                let response_id = string(payload, &["/response_id"]);
                if let Some(response_id) = &response_id {
                    if !response_ids.insert(response_id.clone()) {
                        // A copied or repeated record: the same response cannot
                        // be billed twice.
                        continue;
                    }
                }
                let created_at = timestamp(&value).unwrap_or(fallback_timestamp);
                // A fork copies its parent's records wholesale; the thread id
                // names the owner, and only the owner may bill the usage.
                let owner_mismatch = string(payload, &["/thread_id", "/session_id"])
                    .is_some_and(|owner| owner != thread_id);
                if owner_mismatch {
                    // Still remember it, so a fork's legacy mirror — if one
                    // follows — is consumed instead of counted.
                    pending_responses.push(usage);
                    continue;
                }
                if let Some((mirror, request_id)) = legacy_before_response.take() {
                    if mirror == usage {
                        // The legacy notification came first and was counted.
                        // Hijack its identity so an overlapping runtime segment
                        // (or a later scan) still sees one call.
                        if let Some(response_id) = &response_id {
                            if let Some(record) =
                                parsed.records.iter_mut().find(|record| record.request_id == request_id)
                            {
                                record
                                    .legacy_request_ids
                                    .push(record.request_id.clone());
                                record.request_id =
                                    format!("SESSION:codex:{thread_id}:{response_id}");
                            }
                        }
                        continue;
                    }
                }
                let token_usage = usage.clone().into_usage();
                if token_usage.total_tokens().is_none() {
                    continue;
                }
                pending_responses.push(usage);
                // `usage` includes the cached and cache-write parts; the
                // importer's input bucket is the fresh remainder.
                let request_id = match &response_id {
                    Some(response_id) => format!("SESSION:codex:{thread_id}:{response_id}"),
                    None => {
                        event_index += 1;
                        format!("SESSION:codex:{thread_id}:token_usage:{event_index}")
                    }
                };
                parsed.records.push(SessionUsageRecord {
                    metadata: Default::default(),
                    request_id,
                    legacy_request_ids: Vec::new(),
                    cli_key: GatewayUsageTool::Codex,
                    model: model.clone(),
                    usage: token_usage,
                    created_at,
                    session_id: thread_id.clone(),
                    reported_cost_usd: None,
                });
            }
            _ => {}
        }
    }
    Ok(parsed)
}

fn counter_signature(info: &Value) -> String {
    let counters = |name: &str| {
        info.get(name).and_then(Value::as_object).map(|fields| {
            [
                "input_tokens",
                "cached_input_tokens",
                "cache_read_input_tokens",
                "cache_write_input_tokens",
                "output_tokens",
                "reasoning_output_tokens",
                "total_tokens",
            ]
            .into_iter()
            .filter_map(|key| {
                fields
                    .get(key)
                    .map(|value| (key.to_string(), value.clone()))
            })
            .collect::<BTreeMap<_, _>>()
        })
    };
    let bytes = serde_json::to_vec(&(counters("total_token_usage"), counters("last_token_usage")))
        .unwrap_or_default();
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn exclude_codex_replay(parsed: &mut ParsedSession, parent: &[CodexSnapshot]) {
    let cutoff = parsed.started_at.unwrap_or(i64::MAX);
    let parent = parent
        .iter()
        .filter(|event| event.timestamp <= cutoff)
        .collect::<Vec<_>>();
    let replay_length = (0..parent.len())
        .map(|start| {
            parsed
                .snapshots
                .iter()
                .zip(parent[start..].iter())
                .take_while(|(child, parent)| child.signature == parent.signature)
                .count()
        })
        .max()
        .unwrap_or(0);
    let inherited_ids = parsed
        .snapshots
        .iter()
        .take(replay_length)
        .filter_map(|snapshot| snapshot.request_id.as_ref())
        .collect::<std::collections::HashSet<_>>();
    parsed
        .records
        .retain(|record| !inherited_ids.contains(&record.request_id));
}

fn usage_candidate(value: &Value) -> Option<&Value> {
    if value.get("usage").is_some()
        || value.get("usageMetadata").is_some()
        || value.pointer("/message/usage").is_some()
        || value.pointer("/response/usage").is_some()
    {
        return Some(value);
    }
    ["/response", "/message", "/payload", "/data"]
        .iter()
        .find_map(|path| {
            value.pointer(path).filter(|candidate| {
                candidate.get("usage").is_some() || candidate.get("usageMetadata").is_some()
            })
        })
}

pub(super) fn number(value: &Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_u64))
        .unwrap_or(0)
}

/// OpenCode 1.x marks assistant rows with `role`; 2.0 stores the kind in the
/// `session_message.type` column instead, and its schema gives only the
/// assistant kind a `tokens` field.
pub(super) fn is_opencode_usage_row(value: &Value) -> bool {
    value.get("role").and_then(Value::as_str) == Some("assistant")
        || value.get("type").and_then(Value::as_str) == Some("assistant")
}

pub(super) fn string(value: &Value, paths: &[&str]) -> Option<String> {
    paths
        .iter()
        .filter_map(|path| value.pointer(path).and_then(Value::as_str))
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn timestamp(value: &Value) -> Option<i64> {
    [
        "/timestamp",
        "/created_at",
        "/createdAt",
        "/time/completed",
        "/time/created",
        "/time",
        "/ts",
        "/at",
        "/message/created_at",
    ]
    .iter()
    .find_map(|path| {
        let value = value.pointer(path)?;
        if let Some(value) = value
            .as_f64()
            .filter(|value| value.is_finite())
            .map(|value| value as i64)
        {
            return Some(if value > 10_000_000_000 {
                value / 1000
            } else {
                value
            });
        }
        chrono::DateTime::parse_from_rfc3339(value.as_str()?)
            .ok()
            .map(|value| value.timestamp())
    })
}

pub(super) fn read_jsonl(path: &Path, visit: impl FnMut(usize, Value)) -> Result<bool, String> {
    read_jsonl_with_policy(path, false, visit)
}

/// Tolerant variant for append-then-compact transcripts: an unparseable row in
/// the middle of the file is skipped instead of failing the whole read.
///
/// Antigravity's `transcript_full.jsonl` is rewritten by compaction, so a crash
/// mid-flush can leave a broken row anywhere. `session_manager::antigravity`
/// reads the same file with the same skip semantics; keeping both sides aligned
/// avoids one surface importing a session the other renders fine.
pub(super) fn read_jsonl_lenient(
    path: &Path,
    visit: impl FnMut(usize, Value),
) -> Result<bool, String> {
    read_jsonl_with_policy(path, true, visit)
}

fn read_jsonl_with_policy(
    path: &Path,
    lenient: bool,
    mut visit: impl FnMut(usize, Value),
) -> Result<bool, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut reader: Box<dyn BufRead> = if path
        .extension()
        .is_some_and(|ext| ext == "zstd" || ext == "zst")
    {
        Box::new(BufReader::new(
            zstd::stream::read::Decoder::new(file).map_err(|error| error.to_string())?,
        ))
    } else {
        Box::new(BufReader::new(file))
    };
    let mut line = String::new();
    let mut index = 0;
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return Ok(false),
            Ok(_) => {
                match serde_json::from_str(&line) {
                    Ok(value) => visit(index, value),
                    Err(_) if !line.ends_with('\n') => return Ok(true),
                    Err(_) if lenient => {}
                    Err(error) if !line.trim().is_empty() => {
                        return Err(format!(
                            "Invalid session record at line {}: {error}",
                            index + 1
                        ))
                    }
                    Err(_) => {}
                }
                index += 1;
            }
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(true),
            Err(error) => return Err(error.to_string()),
        }
    }
}

pub(super) fn native_record(
    tool: GatewayUsageTool,
    session: &str,
    identity: &str,
    model: Option<String>,
    usage: TokenUsage,
    created_at: i64,
) -> SessionUsageRecord {
    SessionUsageRecord {
        metadata: Default::default(),
        request_id: format!("SESSION:{}:{session}:{identity}", tool.as_str()),
        legacy_request_ids: Vec::new(),
        cli_key: tool,
        model: model.unwrap_or_else(|| "unknown".into()),
        usage,
        created_at,
        session_id: session.into(),
        reported_cost_usd: None,
    }
}

pub(super) fn reported_cost(value: Option<&Value>) -> Option<String> {
    let value = value?;
    let text = value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string());
    let parsed = text.parse::<rust_decimal::Decimal>().ok()?;
    (parsed > rust_decimal::Decimal::ZERO).then(|| parsed.to_string())
}
