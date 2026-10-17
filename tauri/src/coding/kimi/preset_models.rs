//! Preset model lookup for Kimi providers.
//!
//! Kimi Code CLI's `provider catalog add` imports provider + model entries from
//! the public models.dev catalog. ai-toolbox bundles the same catalog
//! (`tauri/resources/models.dev.json`, shared with the OpenCode module), so a
//! provider whose `base_url` matches a catalog entry can offer that entry's
//! models without any network round trip.

use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::coding::open_code::free_models;

/// Catalog entries marked deprecated are skipped: the CLI's own importer does
/// not offer them, so neither should the preset hint.
const MODEL_STATUS_DEPRECATED: &str = "deprecated";

/// One catalog model, reduced to the fields the Kimi model catalog consumes.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiPresetModel {
    /// Upstream model id (`models.<id>` in the catalog).
    pub id: String,
    /// Display name the catalog declares, when present.
    pub display_name: Option<String>,
    /// `limit.context` — feeds `max_context_size`.
    pub max_context_size: Option<i64>,
    /// `limit.input` — feeds `max_input_size`.
    pub max_input_size: Option<i64>,
    /// `limit.output` — feeds `max_output_size`.
    pub max_output_size: Option<i64>,
    /// Whether the catalog marks the model as reasoning-capable.
    pub reasoning: bool,
    /// Whether the catalog marks the model as tool-capable. The frontend only
    /// declares `tool_use` when this is true — the output modality alone does
    /// not imply tool support (969 catalog models set it false).
    pub tool_call: bool,
    /// Input modalities, mapped to Kimi capability names by the frontend.
    pub input_modalities: Vec<String>,
    /// Output modalities, mapped to Kimi capability names by the frontend.
    pub output_modalities: Vec<String>,
}

/// Normalize a base URL for catalog comparison: lowercase, no scheme, no
/// trailing slash. The catalog stores bare hosts (`api.moonshot.ai/v1`).
fn normalize_base_url(raw: &str) -> String {
    let trimmed = raw.trim().to_ascii_lowercase();
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(&trimmed);
    without_scheme.trim_end_matches('/').to_string()
}

/// Host portion of a normalized base URL, used as the fuzzy-match key.
fn host_of(normalized: &str) -> &str {
    normalized.split('/').next().unwrap_or(normalized)
}

fn read_modalities(value: &Value, key: &str) -> Vec<String> {
    value
        .get("modalities")
        .and_then(|modalities| modalities.get(key))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_preset_models(provider: &Value) -> Vec<KimiPresetModel> {
    let Some(models) = provider.get("models").and_then(Value::as_object) else {
        return Vec::new();
    };
    models
        .iter()
        .filter(|(_, model)| {
            model.get("status").and_then(Value::as_str) != Some(MODEL_STATUS_DEPRECATED)
        })
        .map(|(id, model)| KimiPresetModel {
            id: id.clone(),
            display_name: model
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string),
            max_context_size: model
                .pointer("/limit/context")
                .and_then(Value::as_i64)
                .filter(|size| *size > 0),
            max_input_size: model
                .pointer("/limit/input")
                .and_then(Value::as_i64)
                .filter(|size| *size > 0),
            max_output_size: model
                .pointer("/limit/output")
                .and_then(Value::as_i64)
                .filter(|size| *size > 0),
            reasoning: model.get("reasoning").and_then(Value::as_bool).unwrap_or(false),
            tool_call: model.get("tool_call").and_then(Value::as_bool).unwrap_or(false),
            input_modalities: read_modalities(model, "input"),
            output_modalities: read_modalities(model, "output"),
        })
        .collect()
}

/// Catalog index built once from the bundled models.dev data.
///
/// The bundled file is 1.9 MB; parsing it per lookup would re-decode the whole
/// document on every base-URL blur. The index keeps the parsed entries plus the
/// two lookup maps, so a lookup is a hash probe.
struct CatalogIndex {
    /// Normalized `api` URL -> entries sharing that URL, in catalog order. The
    /// bundled data carries three such URLs (e.g. `api.minimax.io/anthropic/v1`
    /// for both `minimax` and `minimax-coding-plan`), and picking either entry
    /// would be a guess, so an ambiguous exact match refuses like the host one.
    by_url: HashMap<String, Vec<Value>>,
    /// Host -> entries, in catalog order. A host with more than one entry (six
    /// bundled hosts do, e.g. `api.z.ai` carries both `zai` and
    /// `zai-coding-plan`) is ambiguous, so the host fallback refuses to guess.
    by_host: HashMap<String, Vec<Value>>,
}

static CATALOG_INDEX: OnceLock<Option<CatalogIndex>> = OnceLock::new();

fn catalog_index() -> Option<&'static CatalogIndex> {
    CATALOG_INDEX
        .get_or_init(|| {
            let catalog = free_models::bundled_models_dev_catalog()?;
            let entries = catalog.as_object()?;
            let mut by_url: HashMap<String, Vec<Value>> = HashMap::new();
            let mut by_host: HashMap<String, Vec<Value>> = HashMap::new();
            for entry in entries.values() {
                let Some(api) = entry.get("api").and_then(Value::as_str) else {
                    continue;
                };
                let normalized = normalize_base_url(api);
                if normalized.is_empty() {
                    continue;
                }
                by_host
                    .entry(host_of(&normalized).to_string())
                    .or_default()
                    .push(entry.clone());
                by_url.entry(normalized).or_default().push(entry.clone());
            }
            Some(CatalogIndex { by_url, by_host })
        })
        .as_ref()
}

/// Look up the bundled catalog entry matching `base_url`.
///
/// Exact match on the normalized URL wins. A URL carried by more than one
/// catalog entry is ambiguous and returns nothing — falling through to the
/// host probe could not help, since the same entries share that host. A
/// host-only match is used solely when that host resolves to exactly one entry
/// (so `api.moonshot.cn/v1` still finds `moonshotai-cn`); an ambiguous host
/// returns nothing, because guessing would offer model ids the endpoint may
/// not serve. Returns an empty list when nothing matches — callers show no
/// preset hint rather than a wrong one, so there is deliberately no name-based
/// fallback.
pub fn preset_models_for_base_url(base_url: &str) -> Vec<KimiPresetModel> {
    let normalized = normalize_base_url(base_url);
    if normalized.is_empty() {
        return Vec::new();
    }
    let Some(index) = catalog_index() else {
        return Vec::new();
    };

    if let Some(entries) = index.by_url.get(&normalized) {
        if entries.len() == 1 {
            return parse_preset_models(&entries[0]);
        }
        return Vec::new();
    }
    match index.by_host.get(host_of(&normalized)) {
        Some(entries) if entries.len() == 1 => parse_preset_models(&entries[0]),
        _ => Vec::new(),
    }
}

/// Tauri command: preset catalog models for a provider base URL.
#[tauri::command]
pub async fn get_kimi_preset_models(base_url: String) -> Result<Vec<KimiPresetModel>, String> {
    Ok(preset_models_for_base_url(&base_url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_base_url_strips_scheme_and_trailing_slash() {
        assert_eq!(normalize_base_url("HTTPS://API.Moonshot.AI/v1/"), "api.moonshot.ai/v1");
        assert_eq!(normalize_base_url("  http://api.moonshot.cn/v1  "), "api.moonshot.cn/v1");
        assert_eq!(normalize_base_url("api.kimi.com/coding/v1"), "api.kimi.com/coding/v1");
        assert_eq!(normalize_base_url("   "), "");
    }

    #[test]
    fn preset_lookup_matches_moonshot_catalog_entry() {
        // The bundled catalog carries `moonshotai`; its models must come back
        // with the context/output limits the CLI projects.
        let models = preset_models_for_base_url("https://api.moonshot.ai/v1");
        assert!(!models.is_empty(), "expected bundled moonshotai models");
        assert!(models.iter().any(|model| model.id.starts_with("kimi-")));
        assert!(models.iter().all(|model| model.max_context_size.is_some()));
    }

    #[test]
    fn preset_lookup_falls_back_to_host_match() {
        // A path the catalog does not declare still resolves through the host.
        let models = preset_models_for_base_url("https://api.moonshot.ai/v1/extra");
        assert!(!models.is_empty());
    }

    #[test]
    fn preset_lookup_returns_empty_for_unknown_or_blank_base_url() {
        assert!(preset_models_for_base_url("https://relay.example.com/v1").is_empty());
        assert!(preset_models_for_base_url("").is_empty());
        assert!(preset_models_for_base_url("   ").is_empty());
    }

    #[test]
    fn preset_lookup_refuses_an_ambiguous_host() {
        // `api.z.ai` carries both `zai` and `zai-coding-plan`; guessing would
        // offer model ids the endpoint may not serve.
        assert!(preset_models_for_base_url("https://api.z.ai/api/paas/v5").is_empty());
        assert!(preset_models_for_base_url("https://open.bigmodel.cn/api/paas/v5").is_empty());
    }

    #[test]
    fn preset_lookup_refuses_an_ambiguous_exact_url() {
        // `api.minimax.io/anthropic/v1` is declared by both `minimax` and
        // `minimax-coding-plan` (same for the firepass/fireworks pair and the
        // minimimax-cn pair). The exact match used to silently keep whichever
        // entry the HashMap insert happened to write last.
        assert!(preset_models_for_base_url("https://api.minimax.io/anthropic/v1").is_empty());
        assert!(preset_models_for_base_url("https://api.fireworks.ai/inference/v1").is_empty());
        assert!(preset_models_for_base_url("https://api.minimaxi.com/anthropic/v1").is_empty());
    }

    #[test]
    fn preset_lookup_skips_deprecated_catalog_models() {
        // `opencode` carries deprecated entries; the CLI's own importer does not
        // offer them, so the preset hint must not either.
        let models = preset_models_for_base_url("https://opencode.ai/zen/v1");
        assert!(!models.is_empty(), "expected bundled opencode models");
        assert!(
            models.iter().all(|model| !model.id.is_empty()),
            "every returned model keeps its id"
        );
    }

    #[test]
    fn preset_lookup_reads_tool_call_and_input_limit() {
        let models = preset_models_for_base_url("https://api.moonshot.ai/v1");
        assert!(!models.is_empty());
        // The catalog declares both flags; a missing read would surface as the
        // serde default (false), so at least one model must report tool support.
        assert!(
            models.iter().any(|model| model.tool_call),
            "moonshot models declare tool_call"
        );
    }
}
