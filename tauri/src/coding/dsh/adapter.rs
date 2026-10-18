use chrono::Local;
use serde_json::{json, Map, Value};

use super::types::{DshPromptConfig, DshPromptConfigContent, DshSettingsConfig};
use crate::coding::db_id::db_extract_id;

pub fn settings_from_db_value(value: Value) -> DshSettingsConfig {
    DshSettingsConfig {
        config_dir: value
            .get("config_dir")
            .and_then(Value::as_str)
            .map(str::to_string),
        profile: value
            .get("profile")
            .and_then(Value::as_str)
            .map(str::to_string),
        updated_at: value
            .get("updated_at")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| Local::now().to_rfc3339()),
    }
}

pub fn settings_to_db_value(config_dir: Option<&str>, profile: Option<&str>) -> Value {
    let now = Local::now().to_rfc3339();
    let mut value = json!({ "updated_at": now });
    if let Some(config_dir) = config_dir.filter(|dir| !dir.trim().is_empty()) {
        value["config_dir"] = json!(config_dir);
    }
    if let Some(profile) = profile.filter(|profile| !profile.trim().is_empty()) {
        value["profile"] = json!(profile);
    }
    value
}

pub fn prompt_from_db_value(value: Value) -> DshPromptConfig {
    DshPromptConfig {
        id: db_extract_id(&value),
        name: value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        content: value
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        is_applied: value
            .get("is_applied")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        sort_index: value
            .get("sort_index")
            .and_then(Value::as_i64)
            .map(|value| value as i32),
        created_at: value
            .get("created_at")
            .and_then(Value::as_str)
            .map(str::to_string),
        updated_at: value
            .get("updated_at")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

pub fn prompt_to_db_value(content: &DshPromptConfigContent) -> Value {
    let mut map = Map::new();
    map.insert("name".to_string(), Value::String(content.name.clone()));
    map.insert(
        "content".to_string(),
        Value::String(content.content.clone()),
    );
    map.insert("is_applied".to_string(), Value::Bool(content.is_applied));
    if let Some(sort_index) = content.sort_index {
        map.insert("sort_index".to_string(), json!(sort_index));
    }
    map.insert(
        "created_at".to_string(),
        Value::String(content.created_at.clone()),
    );
    map.insert(
        "updated_at".to_string(),
        Value::String(content.updated_at.clone()),
    );
    Value::Object(map)
}
