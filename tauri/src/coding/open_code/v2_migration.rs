use serde_json::{json, Map, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

fn sibling_path(config_path: &Path, stem: &str) -> PathBuf {
    let file_name = match config_path
        .extension()
        .and_then(|extension| extension.to_str())
    {
        Some(extension) if !extension.is_empty() => format!("{stem}.{extension}"),
        _ => stem.to_string(),
    };
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(file_name)
}

pub fn v1_backup_path(config_path: &Path) -> PathBuf {
    sibling_path(config_path, "openvode_v1")
}

pub fn v2_backup_path(config_path: &Path) -> PathBuf {
    sibling_path(config_path, "opencode_v2")
}

pub fn is_active(config_path: &Path) -> bool {
    v1_backup_path(config_path).is_file()
}

/// Whether a parsed config carries the V2-only top-level shape.
///
/// Used only to explain a deserialization failure: the mode switch is still
/// the marker file, never the content. `providers` / `plugins` / `agents` are
/// V2 spellings of the V1 `provider` / `plugin` / `agent`, and a V2 `model`
/// selection is an object while V1 requires a string.
pub fn looks_like_v2_config(value: &Value) -> bool {
    let Some(root) = value.as_object() else {
        return false;
    };
    root.contains_key("providers")
        || root.contains_key("plugins")
        || root.contains_key("agents")
        || matches!(root.get("model"), Some(Value::Object(_)))
}

fn object_from(value: Option<&Value>) -> Map<String, Value> {
    value
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn take_object(map: &mut Map<String, Value>, key: &str) -> Result<Map<String, Value>, String> {
    match map.remove(key) {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(object)) => Ok(object),
        Some(_) => Err(format!(
            "Expected `{key}` to be an object while migrating OpenCode config"
        )),
    }
}

fn object_at_mut<'a>(
    map: &'a mut Map<String, Value>,
    key: &str,
) -> Result<&'a mut Map<String, Value>, String> {
    let value = map
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    value
        .as_object_mut()
        .ok_or_else(|| format!("Expected `{key}` to be an object while migrating OpenCode config"))
}

fn merge_object_value(
    target: &mut Map<String, Value>,
    key: &str,
    source: Map<String, Value>,
) -> Result<(), String> {
    let target_object = object_at_mut(target, key)?;
    for (source_key, source_value) in source {
        if let Some(existing) = target_object.get(&source_key) {
            if existing != &source_value {
                return Err(format!(
                    "Cannot migrate conflicting `{key}.{source_key}` values"
                ));
            }
        } else {
            target_object.insert(source_key, source_value);
        }
    }
    Ok(())
}

fn move_key(map: &mut Map<String, Value>, from: &str, to: &str) -> Result<(), String> {
    let Some(value) = map.remove(from) else {
        return Ok(());
    };
    if let Some(existing) = map.get(to) {
        if existing != &value {
            return Err(format!(
                "Cannot migrate both `{from}` and `{to}` because their values differ"
            ));
        }
        return Ok(());
    }
    map.insert(to.to_string(), value);
    Ok(())
}

fn remove_keys(map: &mut Map<String, Value>, keys: &[&str]) {
    for key in keys {
        map.remove(*key);
    }
}

fn string_value(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// OpenCode 2.x stores package specifiers with an `aisdk:` routing prefix and
/// may still carry the pre-rename `@opencode-ai/` scope. The V1 editor holds the
/// bare npm package name, which is also what the V1 runtime accepts.
fn normalize_package_name(package: &str) -> String {
    let package = package.strip_prefix("aisdk:").unwrap_or(package);
    package
        .strip_prefix("@opencode-ai/")
        .map(|rest| format!("@opencode/{rest}"))
        .unwrap_or_else(|| package.to_string())
}

/// The V2 write-side spelling of a package specifier. Only `@opencode/ai/*`
/// built-ins stay bare; every npm package goes through the AI SDK bridge with
/// the `aisdk:` prefix, exactly like the official `packageSpecifier`. The
/// released runtime treats an unprefixed package as a native package and
/// requires it to export `model(modelID, settings)`, which third-party AI SDK
/// providers do not.
fn package_specifier(package: &str) -> String {
    let name = normalize_package_name(package);
    if name.starts_with("@opencode/ai/") {
        name
    } else {
        format!("aisdk:{name}")
    }
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn header_map(value: Option<&Value>) -> Map<String, Value> {
    object_from(value)
        .into_iter()
        .filter(|(_, value)| value.is_string())
        .collect()
}

fn canonical_provider_id(provider_id: &str) -> &str {
    match provider_id {
        "azure-cognitive-services" => "azure",
        "google-vertex-anthropic" => "google-vertex",
        _ => provider_id,
    }
}

fn canonicalize_model_reference(value: &mut Value) {
    let Some(reference) = value.as_str() else {
        return;
    };
    let Some((provider_id, model_id)) = reference.split_once('/') else {
        return;
    };
    let canonical_id = canonical_provider_id(provider_id);
    if canonical_id != provider_id {
        *value = Value::String(format!("{canonical_id}/{model_id}"));
    }
}

fn canonicalize_provider_list(value: &mut Value) {
    let Some(provider_ids) = value.as_array_mut() else {
        return;
    };
    for provider_id in provider_ids {
        if let Some(current_id) = provider_id.as_str() {
            let canonical_id = canonical_provider_id(current_id);
            if canonical_id != current_id {
                *provider_id = Value::String(canonical_id.to_string());
            }
        }
    }
}

/// Provider-level option lowering, mirroring the released OpenCode 2.0
/// `packages/core/src/v1/config/provider-options.ts` `provider()` lowerer.
///
/// That lowerer only splits `headers` and `body` out of the option object;
/// every other option (including `apiKey`) stays in `settings`, because the
/// AI SDK factory receives `settings` verbatim and derives its own auth header
/// (`Authorization`, `x-api-key`, `x-goog-api-key`, `api-key`, ...). An older
/// OpenCode lowerer synthesized those headers here and deleted `apiKey` from
/// `settings`; that shape is not written by any 2.0.x release (the lowerers
/// were removed in `e57d9ca390`, shipped in v2.0.0), so writing it made the
/// editor lose the key on every save (issue #429).
///
/// The result is stored under `api.url`, `api.settings` and `request`.
#[derive(Default)]
struct RenderedProviderOptions {
    url: Option<Value>,
    headers: Map<String, Value>,
    body: Map<String, Value>,
    settings: Map<String, Value>,
}

fn render_provider_options(
    options: &Map<String, Value>,
    explicit_api: Option<&str>,
) -> RenderedProviderOptions {
    // The released 2.0 lowerer is package-agnostic: only `headers` and `body`
    // leave the option object. `apiKey`, `authToken`, `organization`,
    // `project`, `timeout`, ... stay in `settings` and reach the AI SDK
    // factory verbatim, which derives the authentication header itself.
    let mut settings = options.clone();
    remove_keys(&mut settings, &["baseURL", "headers", "body"]);

    let headers = header_map(options.get("headers"));
    let body = object_from(options.get("body"));
    let url = string_value(options.get("baseURL"))
        .map(Value::String)
        .or_else(|| explicit_api.map(|value| Value::String(value.to_string())));

    RenderedProviderOptions {
        url,
        headers,
        body,
        settings,
    }
}

/// V1 model options `@ai-sdk/openai-compatible` reads as provider settings.
/// Every other option was sent verbatim in the request body, which is why the
/// official migration splits them (`compatibleModel`, 2.0.28, #54694).
const COMPATIBLE_MODEL_KEYS: &[&str] = &[
    "apiKey",
    "baseURL",
    "headers",
    "body",
    "extraBody",
    "timeout",
    "chunkTimeout",
    "headerTimeout",
    "disabled",
    "reasoningEffort",
    "textVerbosity",
    "user",
    "strictJsonSchema",
];

fn split_model_options(
    package_name: Option<&str>,
    options: &Map<String, Value>,
) -> (Map<String, Value>, Map<String, Value>) {
    if package_name != Some("@ai-sdk/openai-compatible") {
        return (options.clone(), Map::new());
    }
    let mut settings = Map::new();
    let mut body = Map::new();
    for (key, value) in options {
        if COMPATIBLE_MODEL_KEYS.contains(&key.as_str()) {
            settings.insert(key.clone(), value.clone());
        } else {
            body.insert(key.clone(), value.clone());
        }
    }
    (settings, body)
}

fn is_hex_color(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 7 && bytes[0] == b'#' && bytes[1..].iter().all(u8::is_ascii_hexdigit)
}

/// Extract the legacy `interleaved` field name for V2
/// `compatibility.reasoningField`, mirroring the official
/// `Model.compatibility` migration. A boolean `true` carried no field name, so
/// it is dropped like the official migration does.
fn interleaved_reasoning_field(value: Option<&Value>) -> Option<String> {
    let field = match value? {
        Value::String(field) => field.clone(),
        Value::Object(object) => object.get("field").and_then(Value::as_str)?.to_string(),
        _ => return None,
    };
    let field = field.trim();
    (!field.is_empty()).then(|| field.to_string())
}

/// V1 agents and commands carry `model` and `variant` as separate fields; V2
/// folds the variant into the model selection (`provider/model#variant` or an
/// object). An invalid reference produces no selection, like the official
/// `modelSelection`.
fn compose_model_selection(model: Value, variant: Option<Value>) -> Option<Value> {
    let reference = model.as_str()?;
    let (provider_id, rest) = reference.split_once('/')?;
    // V2 rejects `#` inside either ID (`^[^/#]+$` / `^[^#]+$`); writing one
    // would make the whole config fail to decode.
    if provider_id.is_empty() || provider_id.contains('#') || rest.is_empty() {
        return None;
    }
    let (model_id, embedded_variant) = match rest.split_once('#') {
        Some((model_id, embedded)) if !model_id.is_empty() => (model_id, Some(embedded)),
        _ => (rest, None),
    };
    if model_id.is_empty() {
        return None;
    }
    let variant = variant
        .as_ref()
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && !value.contains('#'))
        .or_else(|| {
            embedded_variant
                .filter(|value| !value.is_empty() && !value.contains('#'))
        });
    let mut selection = Map::new();
    selection.insert(
        "providerID".to_string(),
        Value::String(canonical_provider_id(provider_id).to_string()),
    );
    selection.insert("model".to_string(), Value::String(model_id.to_string()));
    if let Some(variant) = variant {
        selection.insert("variant".to_string(), Value::String(variant.to_string()));
    }
    Some(Value::Object(selection))
}

/// Collapse a V2 model selection back into the V1 `(model, variant)` pair.
fn selection_to_v1(selection: &Value) -> Option<(String, Option<String>)> {
    match selection {
        Value::String(reference) => {
            let (model, variant) = reference
                .split_once('#')
                .map(|(model, variant)| (model.to_string(), Some(variant.to_string())))
                .unwrap_or_else(|| (reference.clone(), None));
            Some((model, variant))
        }
        Value::Object(selection) => {
            let provider_id = string_value(selection.get("providerID"))?;
            let model_id = string_value(selection.get("model"))?;
            let variant = string_value(selection.get("variant"));
            Some((
                format!(
                    "{}/{}",
                    canonical_provider_id(&provider_id),
                    model_id
                ),
                variant,
            ))
        }
        _ => None,
    }
}

/// The V2 short selection form: `provider/model` or `provider/model#variant`
/// with no `#` inside either id. Invalid references stay untouched so the
/// failure stays visible instead of being silently rewritten.
fn is_valid_model_selection(reference: &str) -> bool {
    let Some((provider_id, rest)) = reference.split_once('/') else {
        return false;
    };
    if provider_id.is_empty() || provider_id.contains('#') {
        return false;
    }
    let (model_id, variant) = match rest.split_once('#') {
        Some((model_id, variant)) => (model_id, Some(variant)),
        None => (rest, None),
    };
    !model_id.is_empty()
        && !model_id.contains('#')
        && variant.is_none_or(|variant| !variant.is_empty() && !variant.contains('#'))
}

/// `small_model` is still a V1 reference. V2's selection schema rejects a
/// reference containing `#` or missing a model part, so an invalid value is
/// dropped instead of making `agents.title.model` fail the whole decode.
fn is_valid_model_reference(reference: &str) -> bool {
    let Some((provider_id, model_id)) = reference.split_once('/') else {
        return false;
    };
    !provider_id.is_empty()
        && !provider_id.contains('#')
        && !model_id.is_empty()
        && !model_id.contains('#')
}

fn permission_map_to_rules(value: &Value) -> Vec<Value> {
    let mut rules = Vec::new();
    // A whole-config permission string (`"allow"`/`"ask"`/`"deny"`) applies to
    // every action and resource.
    if let Some(effect) = value.as_str() {
        rules.push(json!({ "action": "*", "resource": "*", "effect": effect }));
        return rules;
    }
    let Some(permissions) = value.as_object() else {
        return rules;
    };
    for (action, rule) in permissions {
        let action = normalize_tool_action(action);
        match rule {
            Value::String(effect) => rules.push(json!({
                "action": action,
                "resource": "*",
                "effect": effect,
            })),
            Value::Object(resources) => {
                for (resource, effect) in resources {
                    rules.push(json!({
                        "action": action,
                        "resource": resource,
                        "effect": effect,
                    }));
                }
            }
            _ => {}
        }
    }
    rules
}

/// Reverse of [`normalize_tool_action`] for the V1 editor view. `edit` stays
/// `edit` because V1 accepts it directly; only the renamed runtime tools map.
fn restore_tool_action(action: &str) -> &str {
    match action {
        "shell" => "bash",
        "subagent" => "task",
        _ => action,
    }
}

fn permission_rules_to_map(rules: &[Value]) -> Value {
    let mut grouped: std::collections::BTreeMap<String, Map<String, Value>> =
        std::collections::BTreeMap::new();
    for rule in rules {
        let Some(rule) = rule.as_object() else {
            continue;
        };
        let (Some(action), Some(resource), Some(effect)) = (
            rule.get("action").and_then(Value::as_str),
            rule.get("resource").and_then(Value::as_str),
            rule.get("effect"),
        ) else {
            continue;
        };
        let action = restore_tool_action(action);
        grouped
            .entry(action.to_string())
            .or_default()
            .insert(resource.to_string(), effect.clone());
    }
    let mut result = Map::new();
    for (action, resources) in grouped {
        if resources.len() == 1 {
            if let Some(resource) = resources.keys().next().cloned() {
                if resource == "*" {
                    if let Some(effect) = resources.get(&resource).cloned() {
                        result.insert(action, effect);
                        continue;
                    }
                } else {
                    let mut object = Map::new();
                    if let Some(effect) = resources.get(&resource).cloned() {
                        object.insert(resource, effect);
                        result.insert(action, Value::Object(object));
                        continue;
                    }
                }
            }
        }
        result.insert(action, Value::Object(resources));
    }
    Value::Object(result)
}

fn migrate_agent(value: Value) -> Result<Value, String> {
    let mut agent = value
        .as_object()
        .cloned()
        .ok_or_else(|| "Expected OpenCode agent entries to be objects".to_string())?;
    let mut out = Map::new();
    let model = agent.remove("model");
    let variant = agent.remove("variant");
    if let Some(model) = model {
        // V2 has no agent `variant`; the official migrate folds it into the
        // model selection and drops an invalid reference.
        if let Some(selection) = compose_model_selection(model, variant) {
            out.insert("model".to_string(), selection);
        }
    }
    // A variant without a model never applies (the official `modelSelection`
    // yields undefined), so it is dropped rather than written as an unknown key.
    let mut body = take_object(&mut agent, "options")?;
    if let Some(temperature) = agent.remove("temperature") {
        body.insert("temperature".to_string(), temperature);
    }
    if let Some(top_p) = agent.remove("top_p") {
        body.insert("top_p".to_string(), top_p);
    }
    if !body.is_empty() {
        out.insert("request".to_string(), json!({ "body": body }));
    }
    if let Some(prompt) = agent.remove("prompt") {
        out.insert("system".to_string(), prompt);
    }
    for key in ["description", "mode", "hidden", "steps"] {
        if let Some(value) = agent.remove(key) {
            out.insert(key.to_string(), value);
        }
    }
    // V2 only accepts `#RRGGBB`; a theme color would make the whole agent
    // entry fail to decode, so the official migration falls back to grey.
    if let Some(color) = agent.remove("color") {
        let color = match color.as_str() {
            Some(value) if is_hex_color(value) => Value::String(value.to_string()),
            _ => Value::String("#aaaaaa".to_string()),
        };
        out.insert("color".to_string(), color);
    }
    // Official migrateAgent reads `steps ?? maxSteps`; the deprecated alias
    // must not be dropped while `steps` is unset.
    if !out.contains_key("steps") {
        if let Some(max_steps) = agent.remove("maxSteps") {
            out.insert("steps".to_string(), max_steps);
        }
    } else {
        agent.remove("maxSteps");
    }
    if let Some(disable) = agent.remove("disable") {
        out.insert("disabled".to_string(), disable);
    }
    // Official order is tools first, then permission, because rules are
    // last-match-wins and `permission` overrides the deprecated boolean map.
    let mut rules = Vec::new();
    if let Some(tools) = agent
        .remove("tools")
        .and_then(|value| value.as_object().cloned())
    {
        for (action, enabled) in tools {
            rules.push(json!({
                "action": normalize_tool_action(&action),
                "resource": "*",
                "effect": if enabled.as_bool().unwrap_or(false) { "allow" } else { "deny" },
            }));
        }
    }
    if let Some(permission) = agent.remove("permission") {
        rules.extend(permission_map_to_rules(&permission));
    }
    if !rules.is_empty() {
        out.insert("permissions".to_string(), Value::Array(rules));
    }
    // Preserve unknown fields so an editor round trip does not drop provider
    // specific agent keys the V1 model itself cannot express.
    for (key, value) in agent {
        out.insert(key, value);
    }
    Ok(Value::Object(out))
}

fn migrate_agents(root: &mut Map<String, Value>) -> Result<(), String> {
    let mut entries = Map::new();
    if let Some(agent) = root.remove("agent") {
        entries.extend(
            agent
                .as_object()
                .cloned()
                .ok_or_else(|| "Expected V1 `agent` to be an object".to_string())?,
        );
    }
    if let Some(mode) = root.remove("mode") {
        for (name, mut agent) in mode
            .as_object()
            .cloned()
            .ok_or_else(|| "Expected V1 `mode` to be an object".to_string())?
        {
            if let Some(object) = agent.as_object_mut() {
                object
                    .entry("mode".to_string())
                    .or_insert_with(|| Value::String("primary".to_string()));
            }
            entries.insert(name, agent);
        }
    }
    if !entries.is_empty() {
        let mut agents = entries
            .into_iter()
            .filter_map(|(name, value)| {
                value
                    .is_null()
                    .then_some(None)
                    .unwrap_or_else(|| Some((name, value)))
            })
            .collect::<Map<_, _>>();
        for value in agents.values_mut() {
            *value = migrate_agent(std::mem::take(value))?;
        }
        merge_object_value(root, "agents", agents)?;
    }

    // V2 has no `small_model`; the title agent owns that selection. The editor
    // exposes only `small_model`, so when both exist `small_model` is the newer
    // value and wins instead of failing the whole save (the V2 read view always
    // carries both, which used to make any small-model edit unsavable).
    //
    // The value must be a plain `provider/model`: writing an invalid reference
    // into `agents.title.model` would make the whole V2 config fail to decode.
    if let Some(mut small_model) = root.remove("small_model") {
        canonicalize_model_reference(&mut small_model);
        if let Some(reference) = small_model.as_str().filter(|value| is_valid_model_reference(value)) {
            let agents = object_at_mut(root, "agents")?;
            let title_agent = object_at_mut(agents, "title")?;
            // The V2 read view mirrors the title model into `small_model`, so an
            // unchanged value must not replace a richer title selection (which
            // may carry a variant) with the bare reference.
            let unchanged = title_agent
                .get("model")
                .and_then(selection_to_v1)
                .is_some_and(|(current, _)| current == reference);
            if !unchanged {
                // The official normalize writes the selection object, not the
                // `provider/model` string.
                if let Some(selection) = compose_model_selection(small_model.clone(), None) {
                    title_agent.insert("model".to_string(), selection);
                }
            }
        }
    }
    Ok(())
}

/// V1 had one MCP timeout; V2 bounds three phases independently. The official
/// `migrateMcpTimeout` applies the bare number to every phase. The editor's
/// older `{ request }` shape is folded onto catalog/execution so files written
/// before the fix keep their bound.
fn migrate_mcp_timeout(value: Value) -> Value {
    match value {
        Value::Number(number) => json!({
            "startup": number.clone(),
            "catalog": number.clone(),
            "execution": number,
        }),
        Value::Object(mut object) => {
            if let Some(request) = object.remove("request") {
                object.entry("catalog").or_insert(request.clone());
                object.entry("execution").or_insert(request);
            }
            Value::Object(object)
        }
        other => other,
    }
}

/// The global `experimental.mcp_timeout` never bound connecting in V1, so the
/// official normalize only maps it onto catalog and execution.
fn migrate_global_mcp_timeout(value: Value) -> Value {
    match value {
        Value::Number(number) => json!({ "catalog": number.clone(), "execution": number }),
        other => migrate_mcp_timeout(other),
    }
}

fn migrate_mcp_server(value: Value) -> Result<Value, String> {
    let mut server = value
        .as_object()
        .cloned()
        .ok_or_else(|| "Expected MCP server entries to be objects".to_string())?;
    if let Some(enabled) = server.remove("enabled") {
        let disabled = !enabled
            .as_bool()
            .ok_or_else(|| "Expected V1 MCP server `enabled` to be a boolean".to_string())?;
        server.insert("disabled".to_string(), Value::Bool(disabled));
    }
    if let Some(timeout) = server.remove("timeout") {
        server.insert("timeout".to_string(), migrate_mcp_timeout(timeout));
    }
    if let Some(oauth) = server.remove("oauth") {
        if let Some(oauth) = oauth.as_object() {
            // Rename the V1 fields in place so newer V2-only keys (for example
            // `auth_server_metadata_url`) survive the conversion.
            let mut converted = oauth.clone();
            for (legacy, native) in [
                ("clientId", "client_id"),
                ("clientSecret", "client_secret"),
                ("scope", "scope"),
                ("callbackPort", "callback_port"),
                ("redirectUri", "redirect_uri"),
            ] {
                if let Some(value) = converted.remove(legacy) {
                    converted.insert(native.to_string(), value);
                }
            }
            server.insert("oauth".to_string(), Value::Object(converted));
        } else {
            server.insert("oauth".to_string(), oauth);
        }
    }
    Ok(Value::Object(server))
}

fn migrate_mcp(root: &mut Map<String, Value>) -> Result<(), String> {
    let Some(mcp) = root.remove("mcp") else {
        return Ok(());
    };
    let mut mcp = mcp
        .as_object()
        .cloned()
        .ok_or_else(|| "Expected V1 `mcp` to be an object".to_string())?;
    let mut servers = take_object(&mut mcp, "servers")?;
    let global_timeout = mcp.remove("timeout");
    // A V2 `mcp.servers` entry may still carry the V1 bare-number timeout (the
    // official normalize expands it in place). Expanding it here keeps this
    // editor from writing V1 timeouts into an already-V2 server.
    for server in servers.values_mut() {
        let Some(server) = server.as_object_mut() else {
            continue;
        };
        let Some(timeout) = server
            .get("timeout")
            .filter(|timeout| timeout.is_number())
            .cloned()
        else {
            continue;
        };
        server.insert("timeout".to_string(), migrate_mcp_timeout(timeout));
    }
    for (name, server) in std::mem::take(&mut mcp) {
        if servers.contains_key(&name) {
            return Err(format!(
                "MCP server `{name}` is defined in both legacy and V2 locations"
            ));
        }
        servers.insert(name, migrate_mcp_server(server)?);
    }
    let mut out = Map::new();
    if let Some(experimental) = root.get_mut("experimental").and_then(Value::as_object_mut) {
        if let Some(timeout) = experimental.remove("mcp_timeout") {
            out.insert("timeout".to_string(), migrate_global_mcp_timeout(timeout));
        }
        if experimental.is_empty() {
            root.remove("experimental");
        }
    }
    // The official normalize overlays `mcp.timeout` onto the values seeded by
    // `experimental.mcp_timeout`, so the direct field wins when both exist. A
    // direct bare number bounds every phase (including startup), unlike the
    // global experimental value.
    if let Some(timeout) = global_timeout {
        let timeout = migrate_mcp_timeout(timeout);
        out.insert("timeout".to_string(), timeout);
    }
    if !servers.is_empty() {
        out.insert("servers".to_string(), Value::Object(servers));
    }
    if !out.is_empty() {
        root.insert("mcp".to_string(), Value::Object(out));
    }
    Ok(())
}

fn migrate_model(value: Value, parent_package: Option<&str>) -> Result<Value, String> {
    let mut model = value
        .as_object()
        .cloned()
        .ok_or_else(|| "Expected OpenCode model entries to be objects".to_string())?;
    let model_provider = model
        .remove("provider")
        .and_then(|value| value.as_object().cloned());
    let model_package = model_provider
        .as_ref()
        .and_then(|provider| string_value(provider.get("npm")))
        .map(|package| normalize_package_name(&package));
    let package_name = model_package
        .clone()
        .or_else(|| parent_package.map(normalize_package_name));

    let id = string_value(model.get("id"));
    let family = model.remove("family");
    let name = model.remove("name");
    let limit = model.remove("limit");
    let modalities = model.remove("modalities");
    let tool_call = model.remove("tool_call");
    let reasoning = model.remove("reasoning");
    let temperature = model.remove("temperature");
    let cost = model.remove("cost");
    let mut options = take_object(&mut model, "options")?;
    let headers = model.remove("headers");
    let variants = model.remove("variants");
    let status = string_value(model.get("status"));
    // Compatibility keys the V1 editor cannot express round-trip through its
    // `extra` fields; only `interleaved` has a V1 counterpart.
    let mut compatibility = model
        .remove("compatibility")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();

    let mut out = Map::new();
    if let Some(family) = family {
        out.insert("family".to_string(), family);
    }
    if let Some(name) = name {
        out.insert("name".to_string(), name);
    }
    // V1 `interleaved` becomes V2 `compatibility.reasoningField` (2.0.0+);
    // other compatibility keys survive from the editor's `extra` fields.
    if let Some(field) = interleaved_reasoning_field(model.get("interleaved")) {
        compatibility.insert("reasoningField".to_string(), Value::String(field));
    }
    // The deprecated thinking `blockBinding: false` opt-out moves onto model
    // compatibility; leaving the key in the settings would reach the package
    // as an unknown option (official migrateModel, 2.0.28).
    let mut disable_thinking_block_binding = false;
    for key in ["thinking", "reasoningConfig"] {
        let Some(setting) = options.get_mut(key).and_then(Value::as_object_mut) else {
            continue;
        };
        if setting.remove("blockBinding") != Some(Value::Bool(false)) {
            continue;
        }
        disable_thinking_block_binding = true;
        if setting.is_empty() {
            options.remove(key);
        }
    }
    if disable_thinking_block_binding {
        compatibility.insert(
            "supportsThinkingBlockBinding".to_string(),
            Value::Bool(false),
        );
    }
    if !compatibility.is_empty() {
        out.insert("compatibility".to_string(), Value::Object(compatibility));
    }
    // The released 2.0 shape names these flat: `modelID` is the API model id
    // (`api.id` in the preview shape), `package` overrides the provider package
    // for this model, and `settings` carries the V1 options verbatim because the
    // official `model()` mapping is an identity function.
    if let Some(id) = id.clone() {
        out.insert("modelID".to_string(), Value::String(id));
    }
    if let Some(package) = model_package.as_ref() {
        out.insert(
            "package".to_string(),
            Value::String(package_specifier(package)),
        );
    }
    // Match the official `migrateModel`: non-compatible packages keep every
    // option in `settings`; `@ai-sdk/openai-compatible` splits the options the
    // package reads as settings from those V1 sent verbatim in the body. A
    // model-level `provider.api` endpoint lands in the flat `settings.baseURL`
    // (the released runtime ignores the preview `api` key), overriding any
    // `options.baseURL` the way the official migration does.
    let (mut settings, body) = split_model_options(package_name.as_deref(), &options);
    if let Some(api_url) = model_provider
        .as_ref()
        .and_then(|provider| string_value(provider.get("api")))
    {
        settings.insert("baseURL".to_string(), Value::String(api_url));
    }
    if !settings.is_empty() {
        out.insert("settings".to_string(), Value::Object(settings));
    }
    if !body.is_empty() {
        out.insert("body".to_string(), Value::Object(body));
    }
    if let Some(headers) = headers
        .as_ref()
        .and_then(|value| value.as_object().cloned())
    {
        out.insert("headers".to_string(), Value::Object(headers));
    }
    if let Some(package) = model_package.as_ref() {
        let mut api = Map::new();
        if let Some(id) = id.clone() {
            api.insert("id".to_string(), Value::String(id));
        }
        api.insert("type".to_string(), Value::String("aisdk".to_string()));
        api.insert(
            "package".to_string(),
            Value::String(package_specifier(package)),
        );
        if let Some(url) = model_provider
            .as_ref()
            .and_then(|provider| string_value(provider.get("api")))
        {
            api.insert("url".to_string(), Value::String(url));
        }
        api.insert("settings".to_string(), Value::Object(Map::new()));
        out.insert("api".to_string(), Value::Object(api));
    } else if let Some(id) = id {
        out.insert("api".to_string(), json!({ "id": id }));
    }

    // Released 2.0 keeps the legacy flags on `capabilities`: `tools`,
    // `input`/`output`, `temperature`, and `reasoning: { supported }`.
    // `attachment` has no 2.0 field — the runtime derives media support from
    // `capabilities.input` — so it is not written.
    //
    // The block is only emitted when at least one flag is configured, matching
    // the official migrate: an absent block inherits the catalog defaults, so
    // writing one unconditionally would pin every untouched model to
    // `{ tools: true, input: ["text","image"], output: ["text"] }` on any save.
    let has_capabilities = tool_call.is_some()
        || temperature.is_some()
        || reasoning.is_some()
        || modalities
            .as_ref()
            .is_some_and(|value| value.get("input").is_some() || value.get("output").is_some());
    if has_capabilities {
        // Missing siblings fall back to the official `Capabilities.default()`
        // rather than `[]`, which would silently strip image/pdf input.
        let input = modalities
            .as_ref()
            .and_then(|value| value.get("input"))
            .cloned()
            .unwrap_or_else(|| json!(["text", "image"]));
        let output = modalities
            .as_ref()
            .and_then(|value| value.get("output"))
            .cloned()
            .unwrap_or_else(|| json!(["text"]));
        let mut capabilities = Map::new();
        capabilities.insert(
            "tools".to_string(),
            Value::Bool(tool_call.as_ref().and_then(Value::as_bool).unwrap_or(true)),
        );
        capabilities.insert("input".to_string(), input);
        capabilities.insert("output".to_string(), output);
        if let Some(temperature) = temperature.as_ref().and_then(Value::as_bool) {
            capabilities.insert("temperature".to_string(), Value::Bool(temperature));
        }
        if let Some(reasoning) = reasoning.as_ref().and_then(Value::as_bool) {
            capabilities.insert("reasoning".to_string(), json!({ "supported": reasoning }));
        }
        out.insert("capabilities".to_string(), Value::Object(capabilities));
    }

    // The released 2.0 model overlay has no `request`; headers and body are
    // flat. Model-level options already went through `split_model_options`
    // above, so no protocol-specific re-rendering is needed here: the official
    // migration treats the options as settings (or a compatible body split).
    if let Some(variants) = variants.and_then(|value| value.as_object().cloned()) {
        let converted = variants
            .into_iter()
            .map(|(id, options)| {
                let options = options.as_object().cloned().unwrap_or_default();
                let (settings, body) = split_model_options(package_name.as_deref(), &options);
                let mut variant = Map::new();
                variant.insert("id".to_string(), Value::String(id));
                if !settings.is_empty() {
                    variant.insert("settings".to_string(), Value::Object(settings));
                }
                if !body.is_empty() {
                    variant.insert("body".to_string(), Value::Object(body));
                }
                Value::Object(variant)
            })
            .collect::<Vec<_>>();
        out.insert("variants".to_string(), Value::Array(converted));
    }

    if let Some(cost) = cost.and_then(|value| value.as_object().cloned()) {
        let cache = json!({
            "read": cost.get("cache_read").cloned().unwrap_or_else(|| json!(0)),
            "write": cost.get("cache_write").cloned().unwrap_or_else(|| json!(0)),
        });
        let mut entry = Map::new();
        entry.insert(
            "input".to_string(),
            cost.get("input").cloned().unwrap_or_else(|| json!(0)),
        );
        entry.insert(
            "output".to_string(),
            cost.get("output").cloned().unwrap_or_else(|| json!(0)),
        );
        entry.insert("cache".to_string(), cache);
        let mut costs = vec![Value::Object(entry)];
        if let Some(context) = cost.get("context_over_200k").and_then(Value::as_object) {
            costs.push(json!({
                "tier": { "type": "context", "size": 200000 },
                "input": context.get("input").cloned().unwrap_or_else(|| json!(0)),
                "output": context.get("output").cloned().unwrap_or_else(|| json!(0)),
                "cache": {
                    "read": context.get("cache_read").cloned().unwrap_or_else(|| json!(0)),
                    "write": context.get("cache_write").cloned().unwrap_or_else(|| json!(0)),
                },
            }));
        }
        out.insert("cost".to_string(), Value::Array(costs));
    }

    if status.as_deref() == Some("deprecated") {
        out.insert("disabled".to_string(), Value::Bool(true));
    }
    if let Some(limit) = limit.and_then(|value| value.as_object().cloned()) {
        let mut converted = Map::new();
        for key in ["context", "input", "output"] {
            if let Some(value) = limit.get(key) {
                if let Some(value) = value
                    .as_i64()
                    .or_else(|| value.as_f64().map(|value| value.trunc() as i64))
                {
                    converted.insert(key.to_string(), Value::Number(value.into()));
                }
            }
        }
        if !converted.is_empty() {
            out.insert("limit".to_string(), Value::Object(converted));
        }
    }

    // Preserve unknown model fields that are not part of the V1 schema this
    // converter understands. `reasoning` and `temperature` are lifted into
    // `capabilities` above, `interleaved` is folded into `compatibility`, and
    // flags 2.0 has no home for (`release_date`/`attachment`/`experimental`)
    // are dropped as the official migrate does.
    for key in [
        "release_date",
        "attachment",
        "experimental",
        "interleaved",
        "status",
        "id",
    ] {
        model.remove(key);
    }
    for (key, value) in model {
        out.insert(key, value);
    }
    Ok(Value::Object(out))
}

fn migrate_provider(value: Value) -> Result<Value, String> {
    let mut provider = value
        .as_object()
        .cloned()
        .ok_or_else(|| "Expected OpenCode provider entries to be objects".to_string())?;
    let package_name = string_value(provider.remove("npm").as_ref())
        .map(|package| normalize_package_name(&package));
    let explicit_api = string_value(provider.remove("api").as_ref());
    let options = take_object(&mut provider, "options")?;
    let models = provider.remove("models");
    let name = provider.remove("name");
    let env = provider.remove("env");
    // V2 `setCacheKey` is provider compatibility, not a package setting.
    let supports_prompt_cache_key = options.get("setCacheKey").and_then(Value::as_bool);
    let mut options = options;
    options.remove("setCacheKey");
    // Provider compatibility keys the V1 editor cannot express ride along in
    // its `extra` fields; `setCacheKey` is folded back into them here.
    let mut compatibility = provider
        .remove("compatibility")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let rendered = render_provider_options(&options, explicit_api.as_deref());

    let mut out = Map::new();
    if let Some(name) = name {
        out.insert("name".to_string(), name);
    }
    if let Some(env) = env {
        out.insert("env".to_string(), env);
    }
    if let Some(supports) = supports_prompt_cache_key {
        compatibility.insert(
            "supportsPromptCacheKey".to_string(),
            Value::Bool(supports),
        );
    }
    if !compatibility.is_empty() {
        out.insert("compatibility".to_string(), Value::Object(compatibility));
    }
    if let Some(package) = package_name.clone() {
        let mut api = Map::new();
        api.insert("type".to_string(), Value::String("aisdk".to_string()));
        api.insert(
            "package".to_string(),
            Value::String(package_specifier(&package)),
        );
        if let Some(url) = rendered.url.clone() {
            api.insert("url".to_string(), url);
        }
        if !rendered.settings.is_empty() {
            api.insert(
                "settings".to_string(),
                Value::Object(rendered.settings.clone()),
            );
        }
        out.insert("api".to_string(), Value::Object(api));
        // OpenCode 2.0 reads the provider flat (`package` plus `settings`/
        // `headers`/`body` overlays, with the base URL inside `settings`) and
        // rejects a provider whose package it cannot resolve, so both shapes are
        // written: the preview shape keeps the URL at `api.url`, the released
        // shape needs `settings.baseURL`. Each decoder ignores the other's keys.
        out.insert(
            "package".to_string(),
            Value::String(package_specifier(&package)),
        );
        let mut settings = rendered.settings.clone();
        if let Some(url) = rendered.url.clone() {
            settings.insert("baseURL".to_string(), url);
        }
        if !settings.is_empty() {
            out.insert("settings".to_string(), Value::Object(settings));
        }
    } else if let Some(url) = rendered.url.clone() {
        out.insert("api".to_string(), json!({ "type": "native", "url": url }));
    }
    if !rendered.headers.is_empty() || !rendered.body.is_empty() {
        let mut request = Map::new();
        if !rendered.headers.is_empty() {
            request.insert(
                "headers".to_string(),
                Value::Object(rendered.headers.clone()),
            );
        }
        if !rendered.body.is_empty() {
            request.insert("body".to_string(), Value::Object(rendered.body.clone()));
        }
        out.insert("request".to_string(), Value::Object(request));
    }
    if !rendered.headers.is_empty() {
        out.insert(
            "headers".to_string(),
            Value::Object(rendered.headers.clone()),
        );
    }
    if !rendered.body.is_empty() {
        out.insert("body".to_string(), Value::Object(rendered.body.clone()));
    }
    if let Some(models) = models.and_then(|value| value.as_object().cloned()) {
        let converted = models
            .into_iter()
            .map(|(id, model)| Ok((id, migrate_model(model, package_name.as_deref())?)))
            .collect::<Result<Map<_, _>, String>>()?;
        out.insert("models".to_string(), Value::Object(converted));
    }
    // Preserve provider fields V2 does not model (e.g. whitelist/blacklist) so a
    // read/write round trip through this editor does not silently delete them.
    for (key, value) in provider {
        out.insert(key, value);
    }
    Ok(Value::Object(out))
}

fn migrate_providers(value: Value) -> Result<Value, String> {
    let providers = value
        .as_object()
        .ok_or_else(|| "Expected V1 `provider` to be an object".to_string())?;
    let mut result = Map::new();
    for (provider_id, provider) in providers {
        let canonical_id = canonical_provider_id(provider_id);
        if result
            .insert(
                canonical_id.to_string(),
                migrate_provider(provider.clone())?,
            )
            .is_some()
        {
            return Err(format!(
                "Provider ID collision while migrating `{provider_id}` to `{canonical_id}`"
            ));
        }
    }
    Ok(Value::Object(result))
}

fn migrate_plugin_list(value: Value) -> Result<Value, String> {
    let plugins = value
        .as_array()
        .ok_or_else(|| "Expected V1 `plugin` to be an array".to_string())?;
    let mut converted = Vec::with_capacity(plugins.len());
    for plugin in plugins {
        match plugin {
            Value::String(_) => converted.push(plugin.clone()),
            Value::Array(entry) if entry.len() == 2 => {
                converted.push(json!({
                    "package": entry[0].clone(),
                    "options": entry[1].clone(),
                }));
            }
            other => return Err(format!("Unsupported V1 plugin entry: {other}")),
        }
    }
    Ok(Value::Array(converted))
}

/// Map V1 tool names onto the V2 runtime actions so migrated rules keep
/// matching (`normalizeAction` in the official migration).
fn normalize_tool_action(action: &str) -> &str {
    match action {
        "write" | "patch" => "edit",
        "task" => "subagent",
        "bash" => "shell",
        _ => action,
    }
}

fn migrate_permissions(root: &mut Map<String, Value>) -> Result<(), String> {
    // Official order: deprecated `tools` rules first, then `permission`, then
    // native `permissions` (last-match-wins, so the newer fields override).
    let mut rules = Vec::new();
    if let Some(tools) = root
        .remove("tools")
        .and_then(|value| value.as_object().cloned())
    {
        for (action, enabled) in tools {
            rules.push(json!({
                "action": normalize_tool_action(&action),
                "resource": "*",
                "effect": if enabled.as_bool().unwrap_or(false) { "allow" } else { "deny" },
            }));
        }
    }
    if let Some(permission) = root.remove("permission") {
        rules.extend(permission_map_to_rules(&permission));
    }
    if rules.is_empty() {
        return Ok(());
    }
    let existing = match root.remove("permissions") {
        None => Vec::new(),
        Some(Value::Array(existing)) => existing,
        Some(_) => return Err("Expected V2 `permissions` to be an array".to_string()),
    };
    // Native `permissions` stay last so they override migrated rules.
    let mut permissions = rules;
    permissions.extend(existing);
    root.insert("permissions".to_string(), Value::Array(permissions));
    Ok(())
}

/// Write provider-selection rules to the stable top-level `policies` entry
/// (2.0.29+). `experimental.policies` remains readable as a deprecated alias.
fn append_policies(root: &mut Map<String, Value>, rules: Vec<Value>) -> Result<(), String> {
    if rules.is_empty() {
        return Ok(());
    }
    let policies = match root.remove("policies") {
        None => Vec::new(),
        Some(Value::Array(existing)) => existing,
        Some(_) => return Err("Expected V2 `policies` to be an array".to_string()),
    };
    // Official order is `[...generated, ...experimental, ...native]`: native
    // policies stay last so they win under last-match-wins.
    let mut generated = rules;
    generated.extend(policies);
    root.insert("policies".to_string(), Value::Array(generated));
    Ok(())
}

/// V1 `command` entries fold `model` + `variant` into the V2 selection and
/// rename `subtask` to `subagent` (`migrateCommand` in the official migration).
fn migrate_commands(root: &mut Map<String, Value>) -> Result<(), String> {
    let Some(commands) = root.remove("command") else {
        return Ok(());
    };
    let mut commands = commands
        .as_object()
        .cloned()
        .ok_or_else(|| "Expected V1 `command` to be an object".to_string())?;
    for command in commands.values_mut() {
        let Some(command) = command.as_object_mut() else {
            continue;
        };
        let model = command.remove("model");
        let variant = command.remove("variant");
        if let Some(model) = model {
            if let Some(selection) = compose_model_selection(model, variant) {
                command.insert("model".to_string(), selection);
            }
        }
        if let Some(subtask) = command.remove("subtask") {
            command.insert("subagent".to_string(), subtask);
        }
    }
    merge_object_value(root, "commands", commands)
}

fn migrate_disabled_and_enabled_providers(root: &mut Map<String, Value>) -> Result<(), String> {
    let disabled = root.remove("disabled_providers");
    let enabled = root.remove("enabled_providers");
    let mut rules = Vec::new();
    // Official order: `enabled_providers` becomes deny-all + allow-list, then
    // `disabled_providers` denies over it (rules are last-match-wins).
    if let Some(enabled) = enabled {
        let ids = string_array(Some(&enabled));
        // An empty `enabled_providers` array still means "only these", so the
        // official normalize emits deny-all without any allow-list entry.
        if enabled
            .as_array()
            .is_some_and(|entries| entries.is_empty() || !ids.is_empty())
        {
            rules.push(json!({ "effect": "deny", "action": "provider.use", "resource": "*" }));
            for id in ids {
                rules.push(json!({
                    "effect": "allow",
                    "action": "provider.use",
                    "resource": canonical_provider_id(&id),
                }));
            }
        }
    }
    if let Some(disabled) = disabled {
        for id in string_array(Some(&disabled)) {
            rules.push(json!({
                "effect": "deny",
                "action": "provider.use",
                "resource": canonical_provider_id(&id),
            }));
        }
    }
    append_policies(root, rules)
}

pub fn v1_to_v2_value(mut value: Value) -> Result<Value, String> {
    let root = value
        .as_object_mut()
        .ok_or_else(|| "OpenCode config root must be an object".to_string())?;

    if root.contains_key("provider") && root.contains_key("providers") {
        return Err("OpenCode config contains both V1 `provider` and V2 `providers`".to_string());
    }
    if root.contains_key("plugin") && root.contains_key("plugins") {
        return Err("OpenCode config contains both V1 `plugin` and V2 `plugins`".to_string());
    }
    if root.contains_key("agent") && root.contains_key("agents") {
        return Err("OpenCode config contains both V1 `agent` and V2 `agents`".to_string());
    }

    if let Some(providers) = root.remove("provider") {
        root.insert("providers".to_string(), migrate_providers(providers)?);
    }
    if let Some(plugins) = root.remove("plugin") {
        root.insert("plugins".to_string(), migrate_plugin_list(plugins)?);
    }
    migrate_agents(root)?;
    migrate_permissions(root)?;
    migrate_mcp(root)?;
    migrate_disabled_and_enabled_providers(root)?;

    move_key(root, "snapshot", "snapshots")?;
    // V1 `attachment` became V2 `media` (same `{ image: {...} }` shape).
    move_key(root, "attachment", "media")?;
    move_key(root, "reference", "references")?;
    migrate_commands(root)?;
    // The official normalize decodes the top-level model selection and
    // canonicalizes its provider ID, so the retired aliases must be renamed
    // here too (2.0.17, #51901).
    if let Some(mut model) = root.remove("model") {
        match model.as_str() {
            Some(_) => {
                canonicalize_model_reference(&mut model);
                // The top-level selection accepts the `provider/model#variant`
                // short form (`ConfigModel.Selection`), unlike `small_model`.
                let selection = is_valid_model_selection(model.as_str().unwrap_or_default())
                    .then(|| compose_model_selection(model.clone(), None))
                    .flatten();
                match selection {
                    Some(selection) => root.insert("model".to_string(), selection),
                    None => root.insert("model".to_string(), model),
                };
            }
            None => {
                if let Some(selection) = model.as_object_mut() {
                    if let Some(provider_id) = selection
                        .get("providerID")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                    {
                        selection.insert(
                            "providerID".to_string(),
                            Value::String(canonical_provider_id(&provider_id).to_string()),
                        );
                    }
                }
                root.insert("model".to_string(), model);
            }
        }
    }

    if let Some(skills) = root.remove("skills") {
        if let Some(skills) = skills.as_object() {
            let mut converted = Vec::new();
            converted.extend(string_array(skills.get("paths")));
            converted.extend(string_array(skills.get("urls")));
            root.insert(
                "skills".to_string(),
                Value::Array(converted.into_iter().map(Value::String).collect()),
            );
        } else if !skills.is_null() {
            root.insert("skills".to_string(), skills);
        }
    }

    if let Some(autoshare) = root.remove("autoshare") {
        if root.get("share").is_none() && autoshare.as_bool() == Some(true) {
            root.insert("share".to_string(), Value::String("auto".to_string()));
        }
    }

    if let Some(compaction) = root.get_mut("compaction").and_then(Value::as_object_mut) {
        let auto = compaction.remove("auto");
        let prune = compaction.remove("prune");
        let preserve_recent_tokens = compaction.remove("preserve_recent_tokens");
        let reserved = compaction.remove("reserved");
        if let Some(auto) = auto {
            compaction.insert("auto".to_string(), auto);
        }
        if let Some(prune) = prune {
            compaction.insert("prune".to_string(), prune);
        }
        if let Some(tokens) = preserve_recent_tokens {
            compaction.insert("keep".to_string(), json!({ "tokens": tokens }));
        }
        if let Some(buffer) = reserved {
            compaction.insert("buffer".to_string(), buffer);
        }
    }

    Ok(value)
}

/// Authentication header the AI SDK package derives from `settings.apiKey`
/// (and, for Anthropic, `settings.authToken`). The released 2.0 writer keeps
/// the key in `settings`, but hand-written and third-party files may carry it
/// as a header instead; lifting it back keeps one source of truth in the
/// editor (issue #429).
fn auth_header_names(package_name: &str) -> &'static [&'static str] {
    match package_name {
        "@ai-sdk/openai" => &["Authorization"],
        "@ai-sdk/anthropic" | "@ai-sdk/google-vertex/anthropic" => {
            &["x-api-key", "Authorization"]
        }
        "@ai-sdk/google" | "@ai-sdk/google-vertex" => &["x-goog-api-key"],
        "@ai-sdk/azure" => &["api-key"],
        _ => &[],
    }
}

/// Find a header case-insensitively.
fn find_header(headers: &Map<String, Value>, name: &str) -> Option<(String, String)> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .and_then(|(key, value)| {
            value
                .as_str()
                .map(|value| (key.clone(), value.trim().to_string()))
        })
}

fn strip_bearer(value: &str) -> Option<String> {
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| token.trim().to_string())
        .filter(|token| !token.is_empty())
}

/// Move authentication headers back into the option fields the editor reads.
///
/// Only a header whose matching option is unset is lifted, so an explicit
/// `apiKey` keeps winning over a stale header. Non-authentication headers stay
/// untouched, and an emptied `headers` object is dropped.
fn lift_auth_headers(options: &mut Map<String, Value>, package_name: Option<&str>) {
    let Some(package_name) = package_name else {
        return;
    };
    let Some(mut headers) = options
        .remove("headers")
        .and_then(|value| value.as_object().cloned())
    else {
        return;
    };
    for header in auth_header_names(package_name) {
        let Some((key, value)) = find_header(&headers, header) else {
            continue;
        };
        let target = match (*header, package_name) {
            ("Authorization", "@ai-sdk/openai") => ("apiKey", strip_bearer(&value)),
            ("Authorization", _) => ("authToken", strip_bearer(&value)),
            _ => ("apiKey", Some(value)),
        };
        if let (field, Some(token)) = target {
            // An explicit option wins over a leftover header; in that case the
            // header stays as the deliberate override.
            if !options.contains_key(field) {
                options.insert(field.to_string(), Value::String(token));
                headers.remove(&key);
            }
        }
    }
    if !headers.is_empty() {
        options.insert("headers".to_string(), Value::Object(headers));
    }
}

fn convert_provider_v2_to_v1(value: Value) -> Result<Value, String> {
    let mut provider = value
        .as_object()
        .cloned()
        .ok_or_else(|| "Expected V2 provider entries to be objects".to_string())?;
    let mut out = Map::new();
    if let Some(name) = provider.remove("name") {
        out.insert("name".to_string(), name);
    }
    if let Some(env) = provider.remove("env") {
        out.insert("env".to_string(), env);
    }
    let mut options = Map::new();
    if let Some(api) = provider
        .remove("api")
        .and_then(|value| value.as_object().cloned())
    {
        if let Some(package) = string_value(api.get("package")) {
            out.insert(
                "npm".to_string(),
                Value::String(normalize_package_name(&package)),
            );
        }
        if let Some(url) = string_value(api.get("url")) {
            out.insert("api".to_string(), Value::String(url.clone()));
            options.insert("baseURL".to_string(), Value::String(url));
        }
        if let Some(settings) = api.get("settings").and_then(Value::as_object) {
            for (key, value) in settings {
                options.insert(key.clone(), value.clone());
            }
        }
    }
    if let Some(request) = provider
        .remove("request")
        .and_then(|value| value.as_object().cloned())
    {
        if let Some(headers) = request.get("headers") {
            options.insert("headers".to_string(), headers.clone());
        }
        if let Some(body) = request.get("body") {
            options.insert("body".to_string(), body.clone());
        }
    }
    // A provider written by OpenCode 2.0 itself only carries the flat shape, and
    // a file written by this editor carries both; `or_insert` keeps the preview
    // shape authoritative when the two overlap without dropping flat-only keys.
    let flat_settings = provider
        .remove("settings")
        .and_then(|value| value.as_object().cloned());
    let flat_package = string_value(provider.remove("package").as_ref());
    if !out.contains_key("npm") {
        if let Some(package) = flat_package {
            out.insert(
                "npm".to_string(),
                Value::String(normalize_package_name(&package)),
            );
        }
    }
    if let Some(url) = flat_settings
        .as_ref()
        .and_then(|settings| string_value(settings.get("baseURL")))
    {
        out.entry("api".to_string())
            .or_insert(Value::String(url.clone()));
        options
            .entry("baseURL".to_string())
            .or_insert(Value::String(url));
    }
    if let Some(settings) = flat_settings {
        for (key, value) in settings {
            options.entry(key).or_insert(value);
        }
    }
    for key in ["headers", "body"] {
        if !options.contains_key(key) {
            if let Some(value) = provider.remove(key) {
                options.insert(key.to_string(), value);
            }
        } else {
            provider.remove(key);
        }
    }
    let package_name = out.get("npm").and_then(Value::as_str).map(str::to_string);
    lift_auth_headers(&mut options, package_name.as_deref());
    // V2 `compatibility.supportsPromptCacheKey` is the V1 `setCacheKey` option;
    // the remaining provider compatibility keys stay on the V1 provider so a
    // read/write round trip does not drop them.
    if let Some(mut compatibility) = provider
        .remove("compatibility")
        .and_then(|value| value.as_object().cloned())
    {
        if let Some(supports) = compatibility
            .remove("supportsPromptCacheKey")
            .and_then(|value| value.as_bool())
        {
            options.insert("setCacheKey".to_string(), Value::Bool(supports));
        }
        if !compatibility.is_empty() {
            out.insert("compatibility".to_string(), Value::Object(compatibility));
        }
    }
    if !options.is_empty() {
        out.insert("options".to_string(), Value::Object(options));
    }
    if let Some(models) = provider
        .remove("models")
        .and_then(|value| value.as_object().cloned())
    {
        let converted = models
            .into_iter()
            .map(|(id, model)| Ok((id, convert_model_v2_to_v1(model)?)))
            .collect::<Result<Map<_, _>, String>>()?;
        out.insert("models".to_string(), Value::Object(converted));
    }
    for (key, value) in provider {
        out.insert(key, value);
    }
    Ok(Value::Object(out))
}

fn convert_model_v2_to_v1(value: Value) -> Result<Value, String> {
    let mut model = value
        .as_object()
        .cloned()
        .ok_or_else(|| "Expected V2 model entries to be objects".to_string())?;
    let mut options = Map::new();
    if let Some(api) = model
        .remove("api")
        .and_then(|value| value.as_object().cloned())
    {
        if let Some(id) = api.get("id") {
            model.insert("id".to_string(), id.clone());
        }
        if let Some(package) = string_value(api.get("package")) {
            let url = string_value(api.get("url"));
            let mut provider = Map::new();
            provider.insert(
                "npm".to_string(),
                Value::String(normalize_package_name(&package)),
            );
            if let Some(url) = url {
                provider.insert("api".to_string(), Value::String(url));
            }
            model.insert("provider".to_string(), Value::Object(provider));
        }
        if let Some(settings) = api.get("settings").and_then(Value::as_object) {
            for (key, value) in settings {
                options.insert(key.clone(), value.clone());
            }
        }
    }
    if let Some(request) = model
        .remove("request")
        .and_then(|value| value.as_object().cloned())
    {
        if let Some(headers) = request.get("headers") {
            model.insert("headers".to_string(), headers.clone());
        }
        if let Some(body) = request.get("body").and_then(Value::as_object) {
            for (key, value) in body {
                options.insert(key.clone(), value.clone());
            }
        }
    }
    // Released 2.0 keeps model request overrides flat (`body` next to
    // `settings`/`headers`); `request` belongs to agents only.
    if let Some(body) = model
        .remove("body")
        .and_then(|value| value.as_object().cloned())
    {
        for (key, value) in body {
            options.entry(key).or_insert(value);
        }
    }
    // Same dual-shape rule as providers: the flat keys are what OpenCode 2.0
    // writes, they only fill what the preview shape left unset, and they are
    // consumed here so a save cannot echo them back next to the preview fields.
    if !model.contains_key("id") {
        if let Some(id) = string_value(model.get("modelID")) {
            model.insert("id".to_string(), Value::String(id));
        }
    }
    model.remove("modelID");
    if !model.contains_key("provider") {
        if let Some(package) = string_value(model.get("package")) {
            model.insert(
                "provider".to_string(),
                json!({ "npm": normalize_package_name(&package) }),
            );
        }
    }
    model.remove("package");
    if let Some(settings) = model
        .remove("settings")
        .and_then(|value| value.as_object().cloned())
    {
        for (key, value) in settings {
            options.entry(key).or_insert(value);
        }
    }
    // `reasoningField` is the V1 `interleaved`; the other compatibility keys
    // stay on the V1 model so a read/write round trip does not drop them.
    if let Some(mut compatibility) = model
        .remove("compatibility")
        .and_then(|value| value.as_object().cloned())
    {
        if let Some(field) = compatibility
            .remove("reasoningField")
            .and_then(|value| value.as_str().map(str::to_string))
        {
            model.insert("interleaved".to_string(), Value::String(field));
        }
        // The editor still edits the deprecated option form; the forward
        // migration folds it back into `compatibility`. Merge into an existing
        // `thinking` object (which may come from flat `settings`) instead of
        // replacing its other settings.
        if compatibility
            .remove("supportsThinkingBlockBinding")
            .and_then(|value| value.as_bool())
            == Some(false)
        {
            let thinking = options
                .entry("thinking".to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            if let Some(thinking) = thinking.as_object_mut() {
                thinking.insert("blockBinding".to_string(), Value::Bool(false));
            }
        }
        if !compatibility.is_empty() {
            model.insert("compatibility".to_string(), Value::Object(compatibility));
        }
    }
    if !options.is_empty() {
        model.insert("options".to_string(), Value::Object(options));
    }
    if let Some(capabilities) = model
        .remove("capabilities")
        .and_then(|value| value.as_object().cloned())
    {
        if capabilities.get("input").is_some() || capabilities.get("output").is_some() {
            let mut modalities = Map::new();
            if let Some(input) = capabilities.get("input") {
                modalities.insert("input".to_string(), input.clone());
            }
            if let Some(output) = capabilities.get("output") {
                modalities.insert("output".to_string(), output.clone());
            }
            model.insert("modalities".to_string(), Value::Object(modalities));
        }
        if let Some(tools) = capabilities.get("tools") {
            model.insert("tool_call".to_string(), tools.clone());
        }
        if let Some(temperature) = capabilities.get("temperature").and_then(Value::as_bool) {
            model.insert("temperature".to_string(), Value::Bool(temperature));
        }
        // Released 2.0 spells this `{ supported: boolean }`; accept a bare
        // boolean too so hand-edited files in the older shape still round-trip.
        let reasoning = capabilities.get("reasoning").and_then(|value| {
            value
                .as_bool()
                .or_else(|| value.get("supported").and_then(Value::as_bool))
        });
        if let Some(reasoning) = reasoning {
            model.insert("reasoning".to_string(), Value::Bool(reasoning));
        }
    }
    if let Some(variants) = model
        .remove("variants")
        .and_then(|value| value.as_array().cloned())
    {
        let mut converted = Map::new();
        for variant in variants {
            let Some(variant) = variant.as_object() else {
                continue;
            };
            let Some(id) = string_value(variant.get("id")) else {
                continue;
            };
            // V1 variants are a flat option map. `body` wins over `settings`
            // when the same key appears in both, because `body` is the value
            // that actually reaches the request.
            let mut value = variant
                .get("settings")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if let Some(body) = variant.get("body").and_then(Value::as_object) {
                for (key, item) in body {
                    value.insert(key.clone(), item.clone());
                }
            }
            for (key, item) in variant {
                if key != "id" && key != "body" && key != "settings" {
                    value.insert(key.clone(), item.clone());
                }
            }
            converted.insert(id, Value::Object(value));
        }
        model.insert("variants".to_string(), Value::Object(converted));
    }
    // V2 accepts a single cost object as well as an array.
    if let Some(costs) = model.remove("cost") {
        let costs = match costs {
            Value::Array(costs) => costs,
            Value::Object(cost) => vec![Value::Object(cost)],
            _ => Vec::new(),
        };
        // The V1 shape only has one tier (`context_over_200k`); identify it by
        // its `tier` descriptor instead of trusting array position, and keep
        // the base entry for models that list a tier first.
        let tiered = costs.iter().find_map(|entry| {
            let entry = entry.as_object()?;
            let tier = entry.get("tier")?.as_object()?;
            (tier.get("type").and_then(Value::as_str) == Some("context")).then_some(entry)
        });
        let base = costs.iter().find_map(|entry| {
            let entry = entry.as_object()?;
            (entry.get("tier").is_none()).then_some(entry)
        });
        if let Some(base) = base {
            let mut cost = Map::new();
            if let Some(input) = base.get("input") {
                cost.insert("input".to_string(), input.clone());
            }
            if let Some(output) = base.get("output") {
                cost.insert("output".to_string(), output.clone());
            }
            if let Some(cache) = base.get("cache").and_then(Value::as_object) {
                if let Some(read) = cache.get("read") {
                    cost.insert("cache_read".to_string(), read.clone());
                }
                if let Some(write) = cache.get("write") {
                    cost.insert("cache_write".to_string(), write.clone());
                }
            }
            if let Some(tiered) = tiered {
                let strip_tier = |entry: &Map<String, Value>| {
                    let mut stripped = entry.clone();
                    stripped.remove("tier");
                    Value::Object(stripped)
                };
                cost.insert("context_over_200k".to_string(), strip_tier(tiered));
            }
            model.insert("cost".to_string(), Value::Object(cost));
        }
    }
    if model.remove("disabled").and_then(|value| value.as_bool()) == Some(true) {
        model.insert(
            "status".to_string(),
            Value::String("deprecated".to_string()),
        );
    }
    Ok(Value::Object(model))
}

fn convert_providers_v2_to_v1(value: Value) -> Result<Value, String> {
    let providers = value
        .as_object()
        .ok_or_else(|| "Expected V2 `providers` to be an object".to_string())?;
    let mut result = Map::new();
    for (provider_id, provider) in providers {
        result.insert(
            provider_id.clone(),
            convert_provider_v2_to_v1(provider.clone())?,
        );
    }
    Ok(Value::Object(result))
}

fn convert_plugin_list_v2_to_v1(value: Value) -> Result<Value, String> {
    let plugins = value
        .as_array()
        .ok_or_else(|| "Expected V2 `plugins` to be an array".to_string())?;
    let mut converted = Vec::with_capacity(plugins.len());
    for plugin in plugins {
        match plugin {
            Value::String(_) => converted.push(plugin.clone()),
            Value::Object(entry) => {
                let package = entry.get("package").cloned().ok_or_else(|| {
                    "Expected V2 plugin entry to have a string `package`".to_string()
                })?;
                let options = entry.get("options").cloned().unwrap_or_else(|| json!({}));
                converted.push(Value::Array(vec![package, options]));
            }
            other => return Err(format!("Unsupported V2 plugin entry: {other}")),
        }
    }
    Ok(Value::Array(converted))
}

fn convert_agent_v2_to_v1(value: Value) -> Result<Value, String> {
    let mut agent = value
        .as_object()
        .cloned()
        .ok_or_else(|| "Expected V2 agent entries to be objects".to_string())?;
    let mut out = Map::new();
    if let Some(model) = agent.remove("model") {
        if let Some((reference, variant)) = selection_to_v1(&model) {
            out.insert("model".to_string(), Value::String(reference));
            if let Some(variant) = variant {
                out.insert("variant".to_string(), Value::String(variant));
            }
        }
    }
    if let Some(system) = agent.remove("system") {
        out.insert("prompt".to_string(), system);
    }
    if let Some(disabled) = agent.remove("disabled") {
        out.insert("disable".to_string(), disabled);
    }
    let mut options = Map::new();
    if let Some(request) = agent
        .remove("request")
        .and_then(|value| value.as_object().cloned())
    {
        if let Some(headers) = request.get("headers") {
            options.insert("headers".to_string(), headers.clone());
        }
        if let Some(body) = request.get("body").and_then(Value::as_object) {
            for (key, value) in body {
                options.insert(key.clone(), value.clone());
            }
        }
    }
    if !options.is_empty() {
        out.insert("options".to_string(), Value::Object(options));
    }
    if let Some(permissions) = agent
        .remove("permissions")
        .and_then(|value| value.as_array().cloned())
    {
        out.insert(
            "permission".to_string(),
            permission_rules_to_map(&permissions),
        );
    }
    for (key, value) in agent {
        out.insert(key, value);
    }
    Ok(Value::Object(out))
}

fn convert_agents_v2_to_v1(root: &mut Map<String, Value>) -> Result<(), String> {
    let Some(agents) = root.remove("agents") else {
        return Ok(());
    };
    let mut converted = Map::new();
    if let Some(agents) = agents.as_object() {
        for (name, agent) in agents {
            converted.insert(name.clone(), convert_agent_v2_to_v1(agent.clone())?);
        }
    } else {
        return Err("Expected V2 `agents` to be an object".to_string());
    }
    let small_model = converted
        .get("title")
        .and_then(Value::as_object)
        // The title variant is runtime selection state, not part of the small
        // model reference the editor edits; only the reference is mirrored.
        .and_then(|title| title.get("model"))
        .and_then(|model| selection_to_v1(model).map(|(reference, _)| reference));
    root.insert("agent".to_string(), Value::Object(converted));
    if let Some(small_model) = small_model {
        root.insert("small_model".to_string(), Value::String(small_model));
    }
    Ok(())
}

/// Collapse a V2 `{startup, catalog, execution}` timeout back into the single
/// V1 number. `execution` is the bound a tool call needs; older editor files
/// that only carry `{request}` keep that value.
fn collapse_mcp_timeout(value: &Value) -> Option<Value> {
    let Some(timeout) = value.as_object() else {
        return None;
    };
    timeout
        .get("execution")
        .or_else(|| timeout.get("catalog"))
        .or_else(|| timeout.get("request"))
        .or_else(|| timeout.get("startup"))
        .cloned()
}

fn convert_mcp_server_v2_to_v1(value: Value) -> Result<Value, String> {
    let mut server = value
        .as_object()
        .cloned()
        .ok_or_else(|| "Expected V2 MCP server entries to be objects".to_string())?;
    if let Some(disabled) = server.remove("disabled") {
        server.insert(
            "enabled".to_string(),
            Value::Bool(!disabled.as_bool().unwrap_or(false)),
        );
    }
    if let Some(timeout) = server.remove("timeout") {
        if let Some(collapsed) =
            collapse_mcp_timeout(&timeout).or_else(|| timeout.is_number().then_some(timeout.clone()))
        {
            server.insert("timeout".to_string(), collapsed);
        } else {
            server.insert("timeout".to_string(), timeout);
        }
    }
    if let Some(oauth) = server.remove("oauth") {
        if let Some(oauth) = oauth.as_object() {
            // Keep V2-only keys (for example `auth_server_metadata_url`) in the
            // editor view so the next save does not drop them.
            let mut converted = oauth.clone();
            for (native, legacy) in [
                ("client_id", "clientId"),
                ("client_secret", "clientSecret"),
                ("scope", "scope"),
                ("callback_port", "callbackPort"),
                ("redirect_uri", "redirectUri"),
            ] {
                if let Some(value) = converted.remove(native) {
                    converted.insert(legacy.to_string(), value);
                }
            }
            server.insert("oauth".to_string(), Value::Object(converted));
        } else {
            // V1 also accepts `oauth: false` to disable auto-detection; keep the
            // literal instead of turning the opt-out into nothing.
            server.insert("oauth".to_string(), oauth);
        }
    }
    Ok(Value::Object(server))
}

fn convert_mcp_v2_to_v1(root: &mut Map<String, Value>) -> Result<(), String> {
    let Some(mcp) = root.remove("mcp") else {
        return Ok(());
    };
    let mut mcp = mcp
        .as_object()
        .cloned()
        .ok_or_else(|| "Expected V2 `mcp` to be an object".to_string())?;
    let mut servers = take_object(&mut mcp, "servers")?;
    if let Some(collapsed) = mcp.remove("timeout").and_then(|timeout| {
        collapse_mcp_timeout(&timeout).or_else(|| timeout.is_number().then_some(timeout))
    }) {
        let experimental = object_at_mut(root, "experimental")?;
        experimental.insert("mcp_timeout".to_string(), collapsed);
    }
    for (name, server) in std::mem::take(&mut servers) {
        mcp.insert(name, convert_mcp_server_v2_to_v1(server)?);
    }
    if !mcp.is_empty() {
        root.insert("mcp".to_string(), Value::Object(mcp));
    }
    Ok(())
}

fn take_provider_selection_policies(
    root: &mut Map<String, Value>,
) -> (Option<Vec<String>>, Option<Vec<String>>) {
    // Read the stable top-level entry first; `experimental.policies` is the
    // deprecated alias and is only consulted when the top-level one is absent.
    let (source, policies) = if let Some(policies) = root
        .remove("policies")
        .and_then(|value| value.as_array().cloned())
    {
        ("policies", policies)
    } else if let Some(policies) = root
        .get_mut("experimental")
        .and_then(Value::as_object_mut)
        .and_then(|experimental| experimental.remove("policies"))
        .and_then(|value| value.as_array().cloned())
    {
        ("experimental", policies)
    } else {
        return (None, None);
    };
    let provider_rules = policies.iter().all(|rule| {
        rule.as_object()
            .is_some_and(|rule| rule.get("action").and_then(Value::as_str) == Some("provider.use"))
    });
    if !provider_rules || policies.is_empty() {
        if source == "policies" {
            root.insert("policies".to_string(), Value::Array(policies));
        } else if let Some(experimental) = root.get_mut("experimental").and_then(Value::as_object_mut) {
            experimental.insert("policies".to_string(), Value::Array(policies));
        }
        return (None, None);
    }
    let every_deny = policies.iter().all(|rule| {
        rule.as_object().is_some_and(|rule| {
            rule.get("effect").and_then(Value::as_str) == Some("deny")
                && rule
                    .get("resource")
                    .and_then(Value::as_str)
                    .is_some_and(|resource| resource != "*")
        })
    });
    let (disabled, enabled) = if every_deny {
        (
            Some(
                policies
                    .iter()
                    .filter_map(|rule| {
                        rule.get("resource")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .collect::<Vec<_>>(),
            ),
            None,
        )
    } else {
        let mut allow = policies.iter();
        let first = allow.next().and_then(|rule| rule.as_object());
        let first_deny_all = first.is_some_and(|rule| {
            rule.get("effect").and_then(Value::as_str) == Some("deny")
                && rule.get("resource").and_then(Value::as_str) == Some("*")
        });
        if first_deny_all
            && allow.clone().all(|rule| {
                rule.get("effect").and_then(Value::as_str) == Some("allow")
                    && rule.get("resource").and_then(Value::as_str).is_some()
            })
        {
            (
                None,
                Some(
                    allow
                        .filter_map(|rule| {
                            rule.get("resource")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        })
                        .collect::<Vec<_>>(),
                ),
            )
        } else {
            if source == "policies" {
                root.insert("policies".to_string(), Value::Array(policies));
            } else if let Some(experimental) = root.get_mut("experimental").and_then(Value::as_object_mut) {
                experimental.insert("policies".to_string(), Value::Array(policies));
            }
            return (None, None);
        }
    };
    // Drop an `experimental` object the removal left empty.
    if root
        .get("experimental")
        .and_then(Value::as_object)
        .is_some_and(serde_json::Map::is_empty)
    {
        root.remove("experimental");
    }
    (disabled, enabled)
}

pub fn v2_to_v1_value(mut value: Value) -> Result<Value, String> {
    let root = value
        .as_object_mut()
        .ok_or_else(|| "OpenCode config root must be an object".to_string())?;

    if root.contains_key("provider") && root.contains_key("providers") {
        return Err("OpenCode config contains both V1 `provider` and V2 `providers`".to_string());
    }
    if root.contains_key("plugin") && root.contains_key("plugins") {
        return Err("OpenCode config contains both V1 `plugin` and V2 `plugins`".to_string());
    }
    if root.contains_key("agent") && root.contains_key("agents") {
        return Err("OpenCode config contains both V1 `agent` and V2 `agents`".to_string());
    }

    if let Some(providers) = root.remove("providers") {
        root.insert(
            "provider".to_string(),
            convert_providers_v2_to_v1(providers)?,
        );
    }
    if let Some(plugins) = root.remove("plugins") {
        root.insert("plugin".to_string(), convert_plugin_list_v2_to_v1(plugins)?);
    }
    convert_agents_v2_to_v1(root)?;
    convert_mcp_v2_to_v1(root)?;

    let (disabled, enabled) = take_provider_selection_policies(root);
    if let Some(disabled) = disabled {
        if root.get("disabled_providers").is_none() {
            root.insert(
                "disabled_providers".to_string(),
                Value::Array(disabled.into_iter().map(Value::String).collect()),
            );
        }
    }
    if let Some(enabled) = enabled {
        if root.get("enabled_providers").is_none() {
            root.insert(
                "enabled_providers".to_string(),
                Value::Array(enabled.into_iter().map(Value::String).collect()),
            );
        }
    }

    for key in ["disabled_providers", "enabled_providers"] {
        if let Some(provider_ids) = root.get_mut(key) {
            canonicalize_provider_list(provider_ids);
        }
    }
    // V2 `media` is the V1 `attachment` block; restore the editor's name so a
    // read/write round trip does not move it into `other` under a new key.
    if !root.contains_key("attachment") {
        if let Some(media) = root.remove("media") {
            root.insert("attachment".to_string(), media);
        }
    }
    // V2 accepts both the string and the object form of a model selection; the
    // V1 editor only understands strings. A `variant` is folded into the
    // `provider/model#variant` spelling, which V2 accepts and the forward
    // migration turns back into the object form, so the CLI's default thinking
    // preference survives a save.
    if matches!(root.get("model"), Some(Value::Object(_))) {
        if let Some(selection) = root.remove("model") {
            match selection_to_v1(&selection) {
                Some((reference, variant)) => {
                    let reference = match variant {
                        Some(variant) => format!("{reference}#{variant}"),
                        None => reference,
                    };
                    root.insert("model".to_string(), Value::String(reference));
                }
                None => {
                    // Keep an undecodable selection untouched so the failure is
                    // visible in the raw editor instead of silently discarded.
                    root.insert("model".to_string(), selection);
                }
            }
        }
    }
    // V2 commands are selected through `model`; fold it back into the V1
    // `model` + `variant` pair and restore `subagent`'s V1 alias `subtask`.
    if let Some(commands) = root
        .remove("commands")
        .and_then(|value| value.as_object().cloned())
    {
        let mut converted = Map::new();
        for (name, command) in commands {
            let Some(mut command) = command.as_object().cloned() else {
                continue;
            };
            if let Some(model) = command.remove("model") {
                if let Some((reference, variant)) = selection_to_v1(&model) {
                    command.insert("model".to_string(), Value::String(reference));
                    if let Some(variant) = variant {
                        command.insert("variant".to_string(), Value::String(variant));
                    }
                }
            }
            if let Some(subagent) = command.remove("subagent") {
                command.insert("subtask".to_string(), subagent);
            }
            converted.insert(name, Value::Object(command));
        }
        root.insert("command".to_string(), Value::Object(converted));
    }
    Ok(value)
}

pub fn parse_v1_as_v2_json(content: &str) -> Result<String, String> {
    let value = json5::from_str::<Value>(content)
        .map_err(|error| format!("Failed to parse OpenCode V1 config: {error}"))?;
    let migrated = v1_to_v2_value(value)?;
    serde_json::to_string_pretty(&migrated)
        .map_err(|error| format!("Failed to serialize OpenCode V2 config: {error}"))
}

fn archived_v2_path(config_path: &Path, attempt: usize) -> PathBuf {
    let base = config_path.parent().unwrap_or_else(|| Path::new("."));
    let extension = config_path.extension().and_then(|value| value.to_str());
    let suffix = if attempt == 0 {
        chrono::Local::now().format("%Y%m%d_%H%M%S").to_string()
    } else {
        format!(
            "{}_{}",
            chrono::Local::now().format("%Y%m%d_%H%M%S"),
            attempt
        )
    };
    let file_name = match extension {
        Some(extension) if !extension.is_empty() => format!("opencode_v2.{suffix}.{extension}"),
        _ => format!("opencode_v2.{suffix}"),
    };
    base.join(file_name)
}

fn unique_archived_v2_path(config_path: &Path) -> PathBuf {
    (0..1000)
        .map(|attempt| archived_v2_path(config_path, attempt))
        .find(|path| !path.exists())
        .unwrap_or_else(|| archived_v2_path(config_path, 1000))
}

/// OpenCode 2.x discovers only `opencode.json`/`opencode.jsonc` (the official
/// suite has a `does not load legacy config.json files` test). A default-path
/// `config.json` must be renamed before it becomes the V2 config, otherwise the
/// migrated shape would sit in a file the CLI never reads. Explicit custom,
/// env and shell paths are not renamed: the CLI loads those as given.
pub fn rename_legacy_default_config(config_path: &Path) -> Result<PathBuf, String> {
    if config_path.file_name().and_then(|name| name.to_str()) != Some("config.json") {
        return Ok(config_path.to_path_buf());
    }
    let target = config_path.with_file_name("opencode.json");
    for sibling in [target.clone(), config_path.with_file_name("opencode.jsonc")] {
        if sibling.exists() {
            return Err(format!(
                "Cannot enable OpenCode V2: {} already exists next to {}; merge them first",
                sibling.display(),
                config_path.display()
            ));
        }
    }
    fs::rename(config_path, &target).map_err(|error| {
        format!(
            "Failed to rename legacy OpenCode config {} to {}: {error}",
            config_path.display(),
            target.display()
        )
    })?;
    Ok(target)
}

pub fn set_mode(config_path: &Path, enabled: bool) -> Result<bool, String> {
    let backup_v1 = v1_backup_path(config_path);
    let backup_v2 = v2_backup_path(config_path);
    let config_path_text = config_path.to_string_lossy();
    if config_path_text.eq_ignore_ascii_case(&backup_v1.to_string_lossy())
        || config_path_text.eq_ignore_ascii_case(&backup_v2.to_string_lossy())
    {
        return Err(
            "OpenCode config path conflicts with a reserved migration backup name".to_string(),
        );
    }

    if enabled {
        if backup_v1.exists() {
            return Ok(true);
        }
        if !config_path.exists() {
            return Err(format!(
                "OpenCode config file does not exist: {}",
                config_path.display()
            ));
        }
        let content = fs::read_to_string(config_path)
            .map_err(|error| format!("Failed to read OpenCode config: {error}"))?;
        let migrated = parse_v1_as_v2_json(&content)?;
        let parent = config_path.parent().unwrap_or_else(|| Path::new("."));
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|error| format!("Failed to prepare migrated OpenCode config: {error}"))?;
        temporary
            .write_all(migrated.as_bytes())
            .map_err(|error| format!("Failed to write migrated OpenCode config: {error}"))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| format!("Failed to flush migrated OpenCode config: {error}"))?;

        fs::rename(config_path, &backup_v1).map_err(|error| {
            format!(
                "Failed to back up V1 config to {}: {error}",
                backup_v1.display()
            )
        })?;
        if let Err(error) = temporary.persist(config_path) {
            let restore_error = fs::rename(&backup_v1, config_path).err();
            let mut detail = format!("Failed to install migrated V2 config: {}", error.error);
            if let Some(restore_error) = restore_error {
                detail.push_str(&format!(
                    "; also failed to restore V1 config: {restore_error}"
                ));
            }
            return Err(detail);
        }
        return Ok(true);
    }

    if !backup_v1.exists() {
        return Ok(false);
    }

    let mut archived_previous_v2 = None;
    let has_active_config = config_path.exists();
    if has_active_config && backup_v2.exists() {
        let archive_path = unique_archived_v2_path(config_path);
        fs::rename(&backup_v2, &archive_path).map_err(|error| {
            format!(
                "Failed to archive previous V2 backup {}: {error}",
                backup_v2.display()
            )
        })?;
        archived_previous_v2 = Some(archive_path);
    }

    if has_active_config {
        if let Err(error) = fs::rename(config_path, &backup_v2) {
            if let Some(archive_path) = archived_previous_v2.as_ref() {
                let _ = fs::rename(archive_path, &backup_v2);
            }
            return Err(format!(
                "Failed to save V2 config to {}: {error}",
                backup_v2.display()
            ));
        }
    }

    if let Err(error) = fs::rename(&backup_v1, config_path) {
        let rollback_v2_error = if has_active_config {
            fs::rename(&backup_v2, config_path).err()
        } else {
            None
        };
        let rollback_archive_error = archived_previous_v2
            .as_ref()
            .and_then(|archive_path| fs::rename(archive_path, &backup_v2).err());
        let mut detail = format!(
            "Failed to restore V1 config from {}: {error}",
            backup_v1.display()
        );
        if let Some(rollback_error) = rollback_v2_error {
            detail.push_str(&format!(
                "; also failed to restore active V2 config: {rollback_error}"
            ));
        }
        if let Some(rollback_error) = rollback_archive_error {
            detail.push_str(&format!(
                "; also failed to restore older V2 backup: {rollback_error}"
            ));
        }
        return Err(detail);
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Compatibility keys the V1 editor has no field for must survive a read
    /// and write, both at provider and model level (`setCacheKey`/`interleaved`
    /// keep their dedicated V1 fields).
    #[test]
    fn provider_and_model_compatibility_survive_a_round_trip() {
        let value = json!({
            "providers": {
                "relay": {
                    "package": "@ai-sdk/openai-compatible",
                    "compatibility": {
                        "maxTokensField": "max_completion_tokens",
                        "requireFinishReason": true,
                        "supportsPromptCacheKey": false
                    },
                    "models": {
                        "gpt-x": {
                            "modelID": "gpt-x",
                            "compatibility": {
                                "reasoningField": "reasoning_content",
                                "requireReasoning": true,
                                "cacheFormat": "anthropic"
                            }
                        }
                    }
                }
            }
        });

        let restored = v2_to_v1_value(value).unwrap();
        assert_eq!(
            restored["provider"]["relay"]["compatibility"],
            json!({
                "maxTokensField": "max_completion_tokens",
                "requireFinishReason": true
            })
        );
        assert_eq!(restored["provider"]["relay"]["options"]["setCacheKey"], false);
        assert_eq!(
            restored["provider"]["relay"]["models"]["gpt-x"]["compatibility"],
            json!({ "requireReasoning": true, "cacheFormat": "anthropic" })
        );
        assert_eq!(
            restored["provider"]["relay"]["models"]["gpt-x"]["interleaved"],
            "reasoning_content"
        );

        let written = v1_to_v2_value(restored).unwrap();
        assert_eq!(
            written["providers"]["relay"]["compatibility"],
            json!({
                "maxTokensField": "max_completion_tokens",
                "requireFinishReason": true,
                "supportsPromptCacheKey": false
            })
        );
        assert_eq!(
            written["providers"]["relay"]["models"]["gpt-x"]["compatibility"],
            json!({
                "reasoningField": "reasoning_content",
                "requireReasoning": true,
                "cacheFormat": "anthropic"
            })
        );
    }

    /// The deprecated thinking `blockBinding: false` opt-out moves onto model
    /// compatibility; the read view restores the option form the editor knows.
    #[test]
    fn thinking_block_binding_folds_into_model_compatibility() {
        let v1 = json!({
            "provider": {
                "relay": {
                    "models": {
                        "claude": {
                            "options": {
                                "thinking": { "type": "adaptive", "blockBinding": false },
                                "effort": "high"
                            }
                        },
                        "claude-empty": {
                            "options": { "reasoningConfig": { "blockBinding": false } }
                        },
                        "untouched": { "options": { "thinking": { "type": "adaptive" } } }
                    }
                }
            }
        });
        let migrated = v1_to_v2_value(v1).unwrap();
        let models = &migrated["providers"]["relay"]["models"];
        assert_eq!(
            models["claude"]["settings"]["thinking"],
            json!({ "type": "adaptive" })
        );
        assert_eq!(models["claude"]["settings"]["effort"], "high");
        assert_eq!(
            models["claude"]["compatibility"],
            json!({ "supportsThinkingBlockBinding": false })
        );
        assert!(models["claude-empty"]["settings"]
            .get("reasoningConfig")
            .is_none());
        assert_eq!(
            models["claude-empty"]["compatibility"],
            json!({ "supportsThinkingBlockBinding": false })
        );
        assert!(models["untouched"].get("compatibility").is_none());

        let restored = v2_to_v1_value(migrated).unwrap();
        let restored_models = &restored["provider"]["relay"]["models"];
        assert_eq!(
            restored_models["claude"]["options"]["thinking"],
            json!({ "type": "adaptive", "blockBinding": false })
        );
        assert!(restored_models["claude"]["compatibility"].is_null());
    }

    /// V2 accepts the object form of the default model selection; the V1
    /// editor only understands `provider/model`, so the read view folds it.
    #[test]
    fn object_model_selection_folds_into_the_v1_reference() {
        let value = json!({
            "model": { "providerID": "azure-cognitive-services", "model": "gpt-x", "variant": "high" }
        });
        // The object form is what the official normalize writes; the V1 string
        // carries the variant in the `provider/model#variant` spelling so the
        // CLI's default thinking preference survives an editor save.
        let restored = v2_to_v1_value(value).unwrap();
        assert_eq!(restored["model"], json!("azure/gpt-x#high"));

        // The forward migration turns the reference back into the object form.
        let rewritten = v1_to_v2_value(restored).unwrap();
        assert_eq!(
            rewritten["model"],
            json!({ "providerID": "azure", "model": "gpt-x", "variant": "high" })
        );
    }

    /// `oauth: false` disables OAuth auto-detection in both V1 and V2; the
    /// converter must keep the literal instead of turning it into an empty
    /// object (which would re-enable detection with empty credentials).
    #[test]
    fn mcp_oauth_false_round_trips() {
        let v1 = json!({
            "mcp": {
                "remote": {
                    "type": "remote",
                    "url": "https://example.com/mcp",
                    "oauth": false
                }
            }
        });
        let migrated = v1_to_v2_value(v1).unwrap();
        assert_eq!(migrated["mcp"]["servers"]["remote"]["oauth"], json!(false));

        let restored = v2_to_v1_value(migrated).unwrap();
        assert_eq!(restored["mcp"]["remote"]["oauth"], json!(false));
    }

    /// The V1 editor keeps the bare npm package name. On write, built-in
    /// routes stay bare and every npm package regains the `aisdk:` routing
    /// prefix, matching the official `packageSpecifier`.
    #[test]
    fn package_specifier_writes_aisdk_for_npm_packages_only() {
        let v1 = json!({
            "provider": {
                "builtin": {
                    "npm": "@opencode-ai/ai/providers/openai",
                    "models": { "gpt-x": { "id": "gpt-x" } }
                },
                "third-party": {
                    "npm": "gitlab-ai-provider",
                    "models": { "claude-x": { "id": "claude-x", "provider": { "npm": "gitlab-ai-provider" } } }
                }
            }
        });
        let migrated = v1_to_v2_value(v1).unwrap();
        assert_eq!(
            migrated["providers"]["builtin"]["package"],
            "@opencode/ai/providers/openai"
        );
        assert_eq!(
            migrated["providers"]["third-party"]["package"],
            "aisdk:gitlab-ai-provider"
        );
        assert_eq!(
            migrated["providers"]["third-party"]["models"]["claude-x"]["package"],
            "aisdk:gitlab-ai-provider"
        );

        let restored = v2_to_v1_value(migrated).unwrap();
        assert_eq!(
            restored["provider"]["builtin"]["npm"],
            "@opencode/ai/providers/openai"
        );
        assert_eq!(
            restored["provider"]["third-party"]["npm"],
            "gitlab-ai-provider"
        );

        let rewritten = v1_to_v2_value(restored).unwrap();
        assert_eq!(
            rewritten["providers"]["third-party"]["package"],
            "aisdk:gitlab-ai-provider"
        );
    }

    /// MCP OAuth keys V2 added after the first migration (`auth_server_metadata_url`)
    /// must survive both directions instead of being dropped by the rename.
    #[test]
    fn mcp_oauth_unknown_keys_survive_the_conversion() {
        let v1 = json!({
            "mcp": {
                "remote": {
                    "type": "remote",
                    "url": "https://example.com/mcp",
                    "oauth": {
                        "clientId": "client",
                        "redirectUri": "http://127.0.0.1:19876/callback",
                        "auth_server_metadata_url": "https://example.com/.well-known/oauth-authorization-server"
                    }
                }
            }
        });
        let migrated = v1_to_v2_value(v1).unwrap();
        let oauth = &migrated["mcp"]["servers"]["remote"]["oauth"];
        assert_eq!(oauth["client_id"], "client");
        assert_eq!(oauth["redirect_uri"], "http://127.0.0.1:19876/callback");
        assert_eq!(
            oauth["auth_server_metadata_url"],
            "https://example.com/.well-known/oauth-authorization-server"
        );

        let restored = v2_to_v1_value(migrated).unwrap();
        let oauth = &restored["mcp"]["remote"]["oauth"];
        assert_eq!(oauth["clientId"], "client");
        assert_eq!(oauth["redirectUri"], "http://127.0.0.1:19876/callback");
        assert_eq!(
            oauth["auth_server_metadata_url"],
            "https://example.com/.well-known/oauth-authorization-server"
        );
    }

    /// `enabled_providers: []` means "only these, i.e. none" in V1, so the
    /// migration emits deny-all without an allow-list entry.
    #[test]
    fn empty_enabled_providers_denies_every_provider() {
        let value = json!({ "enabled_providers": [] });
        let migrated = v1_to_v2_value(value).unwrap();
        assert_eq!(
            migrated["policies"],
            json!([{ "effect": "deny", "action": "provider.use", "resource": "*" }])
        );
    }

    /// Native policies must stay last so they win under last-match-wins; the
    /// generated provider rules come first (official `normalizePolicies` order).
    #[test]
    fn generated_policies_precede_native_policies() {
        let value = json!({
            "disabled_providers": ["provider-a"],
            "policies": [
                { "effect": "allow", "action": "provider.use", "resource": "provider-a" }
            ]
        });
        let migrated = v1_to_v2_value(value).unwrap();
        assert_eq!(
            migrated["policies"],
            json!([
                { "effect": "deny", "action": "provider.use", "resource": "provider-a" },
                { "effect": "allow", "action": "provider.use", "resource": "provider-a" }
            ])
        );
    }

    /// Command entries fold `model`+`variant` into a selection and rename
    /// `subtask` to `subagent`; the read view restores the V1 fields.
    #[test]
    fn commands_round_trip_model_variant_and_subtask() {
        let v1 = json!({
            "command": {
                "review": {
                    "template": "Review this.",
                    "model": "anthropic/claude-sonnet",
                    "variant": "thinking",
                    "subtask": true
                }
            }
        });
        let migrated = v1_to_v2_value(v1).unwrap();
        assert_eq!(
            migrated["commands"]["review"]["model"],
            json!({ "providerID": "anthropic", "model": "claude-sonnet", "variant": "thinking" })
        );
        assert_eq!(migrated["commands"]["review"]["subagent"], json!(true));

        let restored = v2_to_v1_value(migrated).unwrap();
        assert_eq!(restored["command"]["review"]["model"], "anthropic/claude-sonnet");
        assert_eq!(restored["command"]["review"]["variant"], "thinking");
        assert_eq!(restored["command"]["review"]["subtask"], json!(true));
    }

    /// An unchanged `small_model` must not overwrite a title-agent selection
    /// that carries a variant; an edited value still wins.
    #[test]
    fn unchanged_small_model_keeps_the_title_variant() {
        let value = json!({
            "agents": {
                "title": { "model": { "providerID": "relay", "model": "gpt-x", "variant": "fast" } }
            },
            "small_model": "relay/gpt-x"
        });
        let migrated = v1_to_v2_value(value).unwrap();
        assert_eq!(
            migrated["agents"]["title"]["model"],
            json!({ "providerID": "relay", "model": "gpt-x", "variant": "fast" })
        );
        assert!(migrated.get("small_model").is_none());

        let edited = json!({
            "agents": {
                "title": { "model": { "providerID": "relay", "model": "gpt-x", "variant": "fast" } }
            },
            "small_model": "relay/gpt-y"
        });
        let migrated = v1_to_v2_value(edited).unwrap();
        assert_eq!(
            migrated["agents"]["title"]["model"],
            json!({ "providerID": "relay", "model": "gpt-y" })
        );
    }

    #[test]
    fn v1_provider_migrates_to_official_v2_provider_and_model_shapes() {
        let value = json!({
            "provider": {
                "relay": {
                    "name": "Relay",
                    "npm": "@ai-sdk/openai-compatible",
                    "api": "https://relay.example.com/v1",
                    "options": {
                        "apiKey": "sk-test",
                        "baseURL": "https://relay.example.com/v1",
                        "headers": { "X-Test": "1" }
                    },
                    "models": {
                        "gpt-x": {
                            "id": "gpt-x-upstream",
                            "name": "GPT X",
                            "modalities": { "input": ["text"], "output": ["text"] },
                            "tool_call": true,
                            "cost": {
                                "input": 1,
                                "output": 2,
                                "cache_read": 0.1,
                                "cache_write": 0.2,
                                "context_over_200k": { "input": 3, "output": 4, "cache_read": 0.3, "cache_write": 0.4 }
                            },
                            "limit": { "context": 1000.5, "output": 200 },
                            "options": { "reasoningEffort": "high" },
                            "variants": { "low": { "reasoningEffort": "low" } }
                        }
                    }
                }
            }
        });

        let migrated = v1_to_v2_value(value).unwrap();
        let provider = &migrated["providers"]["relay"];

        // Official provider V2 shape: `api` carries package/url/settings, and
        // the request carries headers/body. The old top-level `api` string and
        // `options` object must not survive.
        assert_eq!(
            provider["api"],
            json!({
                "type": "aisdk",
                "package": "aisdk:@ai-sdk/openai-compatible",
                "url": "https://relay.example.com/v1",
                "settings": { "apiKey": "sk-test" }
            })
        );
        assert_eq!(provider["request"], json!({ "headers": { "X-Test": "1" } }));
        assert!(provider.get("npm").is_none());
        assert!(provider.get("options").is_none());

        // Released 2.0 shape written alongside it: flat package plus overlays,
        // and the base URL inside `settings` because that shape has no `api.url`.
        // OpenCode 2.0 rejects a provider whose package it cannot resolve, so a
        // file that only carried the preview shape would fail to load at all.
        assert_eq!(provider["package"], "aisdk:@ai-sdk/openai-compatible");
        assert_eq!(
            provider["settings"],
            json!({ "apiKey": "sk-test", "baseURL": "https://relay.example.com/v1" })
        );
        assert_eq!(provider["headers"], json!({ "X-Test": "1" }));

        let model = &provider["models"]["gpt-x"];
        // The official migration only puts a model package on `api` when the
        // model itself declares a provider override; otherwise `api.id` is
        // enough because the provider catalog supplies the package.
        assert_eq!(model["api"], json!({ "id": "gpt-x-upstream" }));
        assert_eq!(model["modelID"], "gpt-x-upstream");
        // `reasoningEffort` is a setting the compatible package reads itself,
        // so it stays in `settings` and is not echoed into a flat body.
        assert_eq!(model["settings"], json!({ "reasoningEffort": "high" }));
        assert!(model.get("body").is_none());
        assert!(model.get("request").is_none());
        assert_eq!(
            model["capabilities"],
            json!({ "tools": true, "input": ["text"], "output": ["text"] })
        );
        assert_eq!(
            model["variants"],
            json!([{ "id": "low", "settings": { "reasoningEffort": "low" } }])
        );
        assert_eq!(
            model["cost"],
            json!([
                {
                    "input": 1,
                    "output": 2,
                    "cache": { "read": 0.1, "write": 0.2 }
                },
                {
                    "tier": { "type": "context", "size": 200000 },
                    "input": 3,
                    "output": 4,
                    "cache": { "read": 0.3, "write": 0.4 }
                }
            ])
        );
        assert_eq!(model["limit"], json!({ "context": 1000, "output": 200 }));
    }

    #[test]
    fn v1_agent_mode_and_small_model_migrate_to_v2_agents() {
        let value = json!({
            "agent": {
                "build": {
                    "prompt": "You build.",
                    "model": "anthropic/claude-sonnet",
                    "options": { "temperature": 0.2 },
                    "permission": { "bash": "ask" }
                }
            },
            "mode": {
                "plan": { "prompt": "You plan." }
            },
            "small_model": "relay/gpt-x"
        });

        let migrated = v1_to_v2_value(value).unwrap();

        assert!(migrated.get("agent").is_none());
        assert!(migrated.get("mode").is_none());
        assert!(migrated.get("small_model").is_none());
        // `small_model` becomes the title agent's object-shaped selection, like
        // the official `modelSelection` output.
        assert_eq!(
            migrated["agents"]["title"]["model"],
            json!({ "providerID": "relay", "model": "gpt-x" })
        );
        assert_eq!(migrated["agents"]["build"]["system"], json!("You build."));
        assert_eq!(
            migrated["agents"]["build"]["model"],
            json!({ "providerID": "anthropic", "model": "claude-sonnet" })
        );
        assert_eq!(
            migrated["agents"]["build"]["request"]["body"],
            json!({ "temperature": 0.2 })
        );
        // `bash` is the V2 runtime action `shell`.
        assert_eq!(
            migrated["agents"]["build"]["permissions"],
            json!([{ "action": "shell", "resource": "*", "effect": "ask" }])
        );
        assert_eq!(migrated["agents"]["plan"]["mode"], json!("primary"));
    }

    #[test]
    fn v1_mcp_nests_oauth_and_bounds_every_timeout_phase() {
        let value = json!({
            "mcp": {
                "remote": {
                    "type": "remote",
                    "url": "https://oauth.example.com/mcp",
                    "oauth": {
                        "clientId": "client",
                        "clientSecret": "secret",
                        "callbackPort": 19877
                    },
                    "enabled": false,
                    "timeout": 30
                }
            }
        });

        let migrated = v1_to_v2_value(value).unwrap();

        assert_eq!(
            migrated["mcp"]["servers"]["remote"]["oauth"],
            json!({ "client_id": "client", "client_secret": "secret", "callback_port": 19877 })
        );
        assert_eq!(
            migrated["mcp"]["servers"]["remote"]["disabled"],
            json!(true)
        );
        assert_eq!(
            migrated["mcp"]["servers"]["remote"]["timeout"],
            json!({ "startup": 30, "catalog": 30, "execution": 30 })
        );
        assert!(migrated["mcp"]["servers"]["remote"]
            .get("enabled")
            .is_none());
    }

    #[test]
    fn v1_top_level_names_and_skills_migrate() {
        let value = json!({
            "snapshot": true,
            "attachment": { "image": { "auto_resize": true } },
            "command": { "deploy": { "template": "deploy" } },
            "reference": { "sdk": { "repository": "github.com/example/sdk" } },
            "skills": { "paths": ["./skills"], "urls": ["https://example.com/skills"] },
            "autoshare": true,
            "permission": { "bash": { "git status": "allow" } },
            "tools": { "write": false }
        });

        let migrated = v1_to_v2_value(value).unwrap();

        assert_eq!(migrated["snapshots"], json!(true));
        assert_eq!(migrated["media"]["image"]["auto_resize"], json!(true));
        assert_eq!(migrated["commands"]["deploy"]["template"], json!("deploy"));
        assert_eq!(
            migrated["references"]["sdk"]["repository"],
            json!("github.com/example/sdk")
        );
        assert_eq!(
            migrated["skills"],
            json!(["./skills", "https://example.com/skills"])
        );
        assert_eq!(migrated["share"], json!("auto"));
        assert!(migrated.get("snapshot").is_none());
        assert!(migrated.get("attachment").is_none());
        assert!(migrated.get("command").is_none());
        assert!(migrated.get("reference").is_none());
        assert!(migrated.get("autoshare").is_none());
        assert!(migrated.get("permission").is_none());
        assert!(migrated.get("tools").is_none());
        // The deprecated `tools` map comes first and `permission` overrides it,
        // matching the official rule order.
        assert_eq!(
            migrated["permissions"],
            json!([
                { "action": "edit", "resource": "*", "effect": "deny" },
                { "action": "shell", "resource": "git status", "effect": "allow" }
            ])
        );

        let restored = v2_to_v1_value(migrated).unwrap();
        assert_eq!(restored["attachment"]["image"]["auto_resize"], json!(true));
        assert!(restored.get("media").is_none());
    }

    #[test]
    fn disabled_and_enabled_providers_round_trip_through_policies() {
        let value = json!({
            "disabled_providers": ["provider-a", "provider-b"],
            "provider": {}
        });

        let migrated = v1_to_v2_value(value).unwrap();
        assert_eq!(
            migrated["policies"],
            json!([
                { "effect": "deny", "action": "provider.use", "resource": "provider-a" },
                { "effect": "deny", "action": "provider.use", "resource": "provider-b" }
            ])
        );
        assert!(migrated.get("experimental").is_none());

        let restored = v2_to_v1_value(migrated).unwrap();
        assert_eq!(
            restored["disabled_providers"],
            json!(["provider-a", "provider-b"])
        );
        assert!(restored.get("policies").is_none());
        assert!(restored.get("experimental").is_none());
    }

    /// Released 2.0 keeps a provider key in `settings` and lets the AI SDK
    /// derive `x-api-key`; synthesizing the header here made the editor lose
    /// the key on every save (issue #429).
    #[test]
    fn v1_anthropic_provider_keeps_its_key_in_settings() {
        let v1 = json!({
            "provider": {
                "anthropic-relay": {
                    "name": "Anthropic Relay",
                    "npm": "@ai-sdk/anthropic",
                    "options": { "apiKey": "sk-ant-test", "baseURL": "https://relay.example.com" },
                    "models": { "claude-x": { "id": "claude-x-20260101", "name": "Claude X" } }
                }
            }
        });
        let written = v1_to_v2_value(v1).unwrap();
        let provider = &written["providers"]["anthropic-relay"];
        assert_eq!(provider["package"], "aisdk:@ai-sdk/anthropic");
        assert_eq!(
            provider["settings"],
            json!({
                "apiKey": "sk-ant-test",
                "baseURL": "https://relay.example.com"
            })
        );
        assert!(provider.get("headers").is_none());
        assert_eq!(
            provider["models"]["claude-x"]["modelID"],
            "claude-x-20260101"
        );
        assert_eq!(
            written["providers"]["anthropic-relay"]["api"]["url"],
            "https://relay.example.com"
        );

        // The editor must see the key again after a read.
        let restored = v2_to_v1_value(written).unwrap();
        assert_eq!(
            restored["provider"]["anthropic-relay"]["options"]["apiKey"],
            "sk-ant-test"
        );
    }

    /// Files written before the fix (and by other tools) carry the key as an
    /// authentication header; reading one lifts it back into `options.apiKey`
    /// so saving cannot keep rewriting the header form (issue #429).
    #[test]
    fn v2_auth_headers_lift_back_into_provider_options() {
        let value = json!({
            "providers": {
                "openai-relay": {
                    "package": "@ai-sdk/openai",
                    "settings": { "baseURL": "https://openai.example.com/v1" },
                    "headers": {
                        "Authorization": "Bearer sk-openai",
                        "OpenAI-Organization": "org-1"
                    }
                },
                "anthropic-relay": {
                    "package": "@ai-sdk/anthropic",
                    "headers": {
                        "x-api-key": "sk-ant",
                        "Authorization": "Bearer token-1"
                    }
                },
                "google-relay": {
                    "package": "@ai-sdk/google",
                    "headers": { "x-goog-api-key": "goog-key" }
                },
                "azure-relay": {
                    "package": "@ai-sdk/azure",
                    "headers": { "api-key": "azure-key" }
                }
            }
        });

        let restored = v2_to_v1_value(value).unwrap();
        let openai = &restored["provider"]["openai-relay"]["options"];
        assert_eq!(openai["apiKey"], "sk-openai");
        assert_eq!(openai["headers"], json!({ "OpenAI-Organization": "org-1" }));

        let anthropic = &restored["provider"]["anthropic-relay"]["options"];
        assert_eq!(anthropic["apiKey"], "sk-ant");
        assert_eq!(anthropic["authToken"], "token-1");
        assert!(anthropic.get("headers").is_none());

        assert_eq!(
            restored["provider"]["google-relay"]["options"]["apiKey"],
            "goog-key"
        );
        assert_eq!(
            restored["provider"]["azure-relay"]["options"]["apiKey"],
            "azure-key"
        );
    }

    /// An explicit option outranks a leftover header, so a deliberate
    /// header override is never silently deleted.
    #[test]
    fn v2_explicit_api_key_wins_over_auth_header() {
        let value = json!({
            "providers": {
                "relay": {
                    "package": "@ai-sdk/openai",
                    "settings": { "apiKey": "sk-option" },
                    "headers": { "Authorization": "Bearer sk-header" }
                }
            }
        });

        let restored = v2_to_v1_value(value).unwrap();
        let options = &restored["provider"]["relay"]["options"];
        assert_eq!(options["apiKey"], "sk-option");
        assert_eq!(
            options["headers"],
            json!({ "Authorization": "Bearer sk-header" })
        );
    }

    /// `reasoning`/`temperature` live on `capabilities` in V2 and must survive
    /// a V1 → V2 → V1 round trip; `attachment` has no V2 field and is dropped
    /// (issue #430).
    #[test]
    fn model_capabilities_round_trip_reasoning_and_temperature() {
        let v1 = json!({
            "provider": {
                "relay": {
                    "npm": "@ai-sdk/openai-compatible",
                    "models": {
                        "gpt-x": {
                            "id": "gpt-x",
                            "reasoning": true,
                            "temperature": false,
                            "attachment": true,
                            "tool_call": true
                        }
                    }
                }
            }
        });

        let written = v1_to_v2_value(v1).unwrap();
        assert_eq!(
            written["providers"]["relay"]["models"]["gpt-x"]["capabilities"],
            json!({
                "tools": true,
                // Official `Capabilities.default()` when modalities are unset.
                "input": ["text", "image"],
                "output": ["text"],
                "temperature": false,
                "reasoning": { "supported": true }
            })
        );
        assert!(
            written["providers"]["relay"]["models"]["gpt-x"]
                .get("attachment")
                .is_none()
        );

        let restored = v2_to_v1_value(written).unwrap();
        let model = &restored["provider"]["relay"]["models"]["gpt-x"];
        assert_eq!(model["reasoning"], true);
        assert_eq!(model["temperature"], false);
        assert!(model.get("attachment").is_none());
    }

    /// A model that configures no capability flag must keep inheriting the
    /// catalog defaults: the official migrate only emits `capabilities` when a
    /// V1 flag is present, so an untouched model is not pinned to
    /// `Capabilities.default()` on every save.
    #[test]
    fn untouched_model_writes_no_capabilities_block() {
        let value = json!({
            "provider": {
                "relay": {
                    "npm": "@ai-sdk/openai-compatible",
                    "models": { "gpt-x": { "id": "gpt-x", "name": "GPT X" } }
                }
            }
        });

        let migrated = v1_to_v2_value(value).unwrap();
        assert!(migrated["providers"]["relay"]["models"]["gpt-x"]
            .get("capabilities")
            .is_none());

        let restored = v2_to_v1_value(migrated).unwrap();
        let model = &restored["provider"]["relay"]["models"]["gpt-x"];
        for key in ["tool_call", "reasoning", "temperature", "attachment", "modalities"] {
            assert!(model.get(key).is_none(), "{key} should stay unset");
        }
    }

    /// A model-level `provider: { api }` endpoint must land in the flat
    /// `settings.baseURL` the released runtime reads (the preview `api` key is
    /// ignored by it), overriding `options.baseURL` like the official
    /// `migrateModel` does.
    #[test]
    fn model_level_provider_api_becomes_flat_settings_base_url() {
        let value = json!({
            "provider": {
                "relay": {
                    "npm": "@ai-sdk/openai-compatible",
                    "models": {
                        "gpt-x": {
                            "id": "gpt-x",
                            "provider": { "npm": "@ai-sdk/openai", "api": "https://model.example.com/v1" },
                            "options": { "baseURL": "https://option.example.com/v1" }
                        }
                    }
                }
            }
        });

        let migrated = v1_to_v2_value(value).unwrap();
        let model = &migrated["providers"]["relay"]["models"]["gpt-x"];
        // The endpoint override reaches the released flat shape...
        assert_eq!(
            model["settings"]["baseURL"],
            "https://model.example.com/v1"
        );
        // ...and the preview shape keeps its own URL for the dev line.
        assert_eq!(model["api"]["url"], "https://model.example.com/v1");
        assert_eq!(model["api"]["package"], "aisdk:@ai-sdk/openai");
        assert!(model.get("request").is_none());
    }

    /// `mcp.timeout` overlays the values seeded by `experimental.mcp_timeout`
    /// (official normalize order), so the direct field wins when both exist.
    #[test]
    fn mcp_timeout_overrides_experimental_mcp_timeout() {
        let value = json!({
            "experimental": { "mcp_timeout": 15 },
            "mcp": { "timeout": 45 }
        });

        let migrated = v1_to_v2_value(value).unwrap();
        assert_eq!(
            migrated["mcp"]["timeout"],
            json!({ "startup": 45, "catalog": 45, "execution": 45 })
        );
        assert!(migrated.get("experimental").is_none());
    }

    /// A file written by OpenCode 2.0 itself carries the flat shape only, so the
    /// editor has to restore it without the preview `api`/`request` keys.
    #[test]
    fn released_v2_flat_provider_restores_the_v1_editor_shape() {
        let value = json!({
            "providers": {
                "relay": {
                    "name": "Relay",
                    "package": "@ai-sdk/openai-compatible",
                    "settings": {
                        "apiKey": "sk-test",
                        "baseURL": "https://relay.example.com/v1"
                    },
                    "headers": { "X-Test": "1" },
                    "body": { "reasoning_effort": "high" },
                    "models": {
                        "gpt-x": {
                            "name": "GPT X",
                            "modelID": "gpt-x-upstream",
                            "package": "@ai-sdk/openai-compatible",
                            "settings": { "reasoningEffort": "high" },
                            "headers": { "X-Model": "2" }
                        }
                    }
                }
            }
        });

        let restored = v2_to_v1_value(value).unwrap();
        let provider = &restored["provider"]["relay"];
        assert_eq!(provider["npm"], "@ai-sdk/openai-compatible");
        assert_eq!(provider["api"], "https://relay.example.com/v1");
        assert_eq!(
            provider["options"]["baseURL"],
            "https://relay.example.com/v1"
        );
        assert_eq!(provider["options"]["apiKey"], "sk-test");
        assert_eq!(provider["options"]["headers"], json!({ "X-Test": "1" }));
        assert_eq!(
            provider["options"]["body"],
            json!({ "reasoning_effort": "high" })
        );
        // The flat keys are consumed, not echoed next to the preview shape.
        assert!(provider.get("package").is_none());
        assert!(provider.get("settings").is_none());
        assert!(provider.get("headers").is_none());
        assert!(provider.get("body").is_none());

        let model = &provider["models"]["gpt-x"];
        assert_eq!(model["id"], "gpt-x-upstream");
        assert_eq!(model["provider"]["npm"], "@ai-sdk/openai-compatible");
        assert_eq!(model["options"]["reasoningEffort"], "high");
        assert_eq!(model["headers"], json!({ "X-Model": "2" }));
        assert!(model.get("modelID").is_none());
        assert!(model.get("settings").is_none());
        assert!(model.get("package").is_none());
    }

    /// The dual-shape file this editor writes must survive a full round trip:
    /// reading it back and saving again cannot drop or duplicate either shape.
    #[test]
    fn dual_shape_provider_survives_a_read_write_round_trip() {
        let v1 = json!({
            "provider": {
                "relay": {
                    "name": "Relay",
                    "npm": "@ai-sdk/openai-compatible",
                    "api": "https://relay.example.com/v1",
                    "options": { "apiKey": "sk-test", "headers": { "X-Test": "1" } },
                    "models": { "gpt-x": { "id": "gpt-x-upstream", "name": "GPT X" } }
                }
            }
        });
        let written = v1_to_v2_value(v1.clone()).unwrap();
        let restored = v2_to_v1_value(written.clone()).unwrap();
        let again = v1_to_v2_value(restored).unwrap();
        assert_eq!(again["providers"]["relay"], written["providers"]["relay"]);
        assert_eq!(
            again["providers"]["relay"]["settings"]["baseURL"],
            "https://relay.example.com/v1"
        );
        assert_eq!(
            again["providers"]["relay"]["api"]["url"],
            "https://relay.example.com/v1"
        );
    }

    #[test]
    fn v2_provider_agent_and_mcp_restore_v1_editor_shapes() {
        let value = json!({
            "providers": {
                "relay": {
                    "name": "Relay",
                    "api": {
                        "type": "aisdk",
                        "package": "@ai-sdk/openai-compatible",
                        "url": "https://relay.example.com/v1",
                        "settings": { "apiKey": "sk-test" }
                    },
                    "request": { "headers": { "X-Test": "1" } },
                    "models": {
                        "gpt-x": {
                            "api": { "id": "gpt-x-upstream" },
                            "capabilities": { "tools": true, "input": ["text"], "output": ["text"] },
                            "request": { "body": { "reasoning_effort": "high" } },
                            "variants": [{ "id": "low", "body": { "reasoning_effort": "low" } }],
                            "cost": [
                                {
                                    "tier": { "type": "context", "size": 200000 },
                                    "input": 3,
                                    "output": 4,
                                    "cache": { "read": 0.3, "write": 0.4 }
                                },
                                { "input": 1, "output": 2, "cache": { "read": 0.1, "write": 0.2 } }
                            ],
                            "limit": { "context": 1000, "output": 200 }
                        }
                    }
                }
            },
            "agents": {
                "title": { "model": "relay/gpt-x" },
                "build": {
                    "model": { "providerID": "anthropic", "model": "claude-sonnet", "variant": "thinking" },
                    "system": "You build.",
                    "disabled": true,
                    "request": { "body": { "temperature": 0.2 } },
                    "permissions": [{ "action": "bash", "resource": "*", "effect": "ask" }]
                }
            },
            "mcp": {
                "servers": {
                    "remote": {
                        "type": "remote",
                        "url": "https://oauth.example.com/mcp",
                        "oauth": { "client_id": "client", "client_secret": "secret", "callback_port": 19877 },
                        "disabled": false,
                        "timeout": { "request": 30 }
                    }
                }
            }
        });

        let restored = v2_to_v1_value(value).unwrap();

        assert_eq!(
            restored["provider"]["relay"]["npm"],
            json!("@ai-sdk/openai-compatible")
        );
        assert_eq!(
            restored["provider"]["relay"]["api"],
            json!("https://relay.example.com/v1")
        );
        assert_eq!(
            restored["provider"]["relay"]["options"]["apiKey"],
            json!("sk-test")
        );
        assert_eq!(
            restored["provider"]["relay"]["options"]["headers"],
            json!({ "X-Test": "1" })
        );
        assert_eq!(
            restored["provider"]["relay"]["models"]["gpt-x"]["id"],
            json!("gpt-x-upstream")
        );
        assert_eq!(
            restored["provider"]["relay"]["models"]["gpt-x"]["tool_call"],
            json!(true)
        );
        assert_eq!(
            restored["provider"]["relay"]["models"]["gpt-x"]["options"]["reasoning_effort"],
            json!("high")
        );
        assert_eq!(
            restored["provider"]["relay"]["models"]["gpt-x"]["variants"]["low"],
            json!({ "reasoning_effort": "low" })
        );
        assert_eq!(
            restored["provider"]["relay"]["models"]["gpt-x"]["cost"]["cache_read"],
            json!(0.1)
        );
        // The context tier is found by its descriptor, not by array position.
        assert_eq!(
            restored["provider"]["relay"]["models"]["gpt-x"]["cost"]["context_over_200k"],
            json!({ "input": 3, "output": 4, "cache": { "read": 0.3, "write": 0.4 } })
        );
        assert_eq!(restored["small_model"], json!("relay/gpt-x"));
        assert_eq!(restored["agent"]["build"]["prompt"], json!("You build."));
        assert_eq!(
            restored["agent"]["build"]["model"],
            json!("anthropic/claude-sonnet")
        );
        assert_eq!(restored["agent"]["build"]["variant"], json!("thinking"));
        assert_eq!(restored["agent"]["build"]["disable"], json!(true));
        assert_eq!(restored["mcp"]["remote"]["enabled"], json!(true));
        assert_eq!(restored["mcp"]["remote"]["timeout"], json!(30));
        assert_eq!(
            restored["mcp"]["remote"]["oauth"],
            json!({ "clientId": "client", "clientSecret": "secret", "callbackPort": 19877 })
        );
    }

    #[test]
    fn set_mode_restores_the_original_v1_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("opencode.json");
        let original = br#"{"provider":{"relay":{"npm":"@ai-sdk/openai-compatible","models":{}}}}"#;
        fs::write(&config_path, original).unwrap();

        assert!(set_mode(&config_path, true).unwrap());
        let migrated = fs::read_to_string(&config_path).unwrap();
        assert!(migrated.contains("\"providers\""));
        assert!(v1_backup_path(&config_path).is_file());

        assert!(!set_mode(&config_path, false).unwrap());
        assert_eq!(fs::read(&config_path).unwrap(), original);
    }

    /// V2 only discovers `opencode.json`/`opencode.jsonc`; a legacy default
    /// `config.json` is renamed before it becomes the active V2 config.
    #[test]
    fn enabling_v2_renames_a_legacy_config_json() {
        let directory = tempfile::tempdir().unwrap();
        let legacy_path = directory.path().join("config.json");
        let original = br#"{"provider":{"relay":{"npm":"@ai-sdk/openai-compatible","models":{}}}}"#;
        fs::write(&legacy_path, original).unwrap();

        let renamed = rename_legacy_default_config(&legacy_path).unwrap();
        assert_eq!(renamed, directory.path().join("opencode.json"));
        assert!(!legacy_path.exists());

        assert!(set_mode(&renamed, true).unwrap());
        assert!(v1_backup_path(&renamed).is_file());

        assert!(!set_mode(&renamed, false).unwrap());
        assert_eq!(fs::read(&renamed).unwrap(), original);
    }

    /// Renaming must not silently merge into an existing V2 filename.
    #[test]
    fn enabling_v2_refuses_to_overwrite_an_existing_opencode_json() {
        let directory = tempfile::tempdir().unwrap();
        let legacy_path = directory.path().join("config.json");
        fs::write(&legacy_path, br#"{"provider":{}}"#).unwrap();
        fs::write(
            directory.path().join("opencode.json"),
            br#"{"providers":{}}"#,
        )
        .unwrap();

        let error = rename_legacy_default_config(&legacy_path).unwrap_err();
        assert!(error.contains("already exists"));
        assert!(legacy_path.exists());
    }

    #[test]
    fn explicit_provider_header_overrides_generated_auth_header() {
        let value = json!({
            "provider": {
                "relay": {
                    "npm": "@ai-sdk/openai",
                    "options": {
                        "apiKey": "sk-test",
                        "headers": { "Authorization": "Custom" }
                    },
                    "models": {}
                }
            }
        });

        let migrated = v1_to_v2_value(value).unwrap();

        assert_eq!(
            migrated["providers"]["relay"]["request"]["headers"]["Authorization"],
            json!("Custom")
        );
    }
}
