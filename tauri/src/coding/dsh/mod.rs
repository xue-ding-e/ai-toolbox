//! DeepSeek Harness (dsh) backend module.
//!
//! Shape-wise this is closest to hermes: a single-file runtime config tool whose
//! provider fact source lives on disk. dsh keeps its providers in a namespaced
//! `settings.yaml` (`llm-pi-ai.providers.<route>`), the default model in the
//! `agent-default-model` section, and API keys in a separate `.credentials.yaml`
//! (`REF: secret`). SQLite only stores the custom config directory and prompt
//! presets; it never holds provider records.

pub mod adapter;
pub mod builtin_models;
pub mod commands;
pub mod constants;
pub mod profile_patch;
pub mod session_artifact;
pub mod tray_support;
pub mod types;
pub mod web_ui;

// NOTE: tray_support is intentionally NOT glob-reexported. Its
// `apply_dsh_prompt_config` would collide with the same-name
// `#[tauri::command]` in commands. Access it via `dsh::tray_support::*`
// (same convention as pi/oh_my_pi/hermes).
pub use commands::*;
