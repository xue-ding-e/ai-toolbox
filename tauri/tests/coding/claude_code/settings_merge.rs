use ai_toolbox_lib::coding::claude_code::settings_merge::{
    extract_provider_settings_for_storage, merge_claude_settings_for_provider,
    merge_claude_settings_for_provider_with_strategy,
    sanitize_claude_settings_for_non_windows_target, split_settings_into_provider_and_common,
    strip_claude_common_config_from_settings, KNOWN_ENV_FIELDS,
};
use ai_toolbox_lib::coding::claude_code::types::ClaudeSettingsMergeStrategy;
use serde_json::json;

fn merge_with_strategy(
    current_disk_settings: &serde_json::Value,
    previous_common_config: Option<&serde_json::Value>,
    next_common_config: &serde_json::Value,
    previous_extra_settings_config: Option<&serde_json::Value>,
    previous_merge_strategy: Option<ClaudeSettingsMergeStrategy>,
    next_extra_settings_config: Option<&serde_json::Value>,
    next_merge_strategy: ClaudeSettingsMergeStrategy,
) -> serde_json::Value {
    merge_claude_settings_for_provider_with_strategy(
        Some(current_disk_settings),
        previous_common_config,
        next_common_config,
        previous_extra_settings_config,
        previous_merge_strategy,
        next_extra_settings_config,
        next_merge_strategy,
        &json!({}),
        &KNOWN_ENV_FIELDS,
    )
    .expect("strategy merge should succeed")
}

#[test]
fn provider_override_strategy_preserves_existing_top_level_replacement_behavior() {
    let common_config = json!({
        "permissions": {
            "allow": ["Read"],
            "deny": ["Read(~/.ssh/**)"]
        }
    });
    let extra_settings = json!({
        "permissions": {
            "deny": ["WebSearch"]
        }
    });

    let merged = merge_with_strategy(
        &json!({}),
        None,
        &common_config,
        None,
        None,
        Some(&extra_settings),
        ClaudeSettingsMergeStrategy::ProviderOverridesCommon,
    );

    assert_eq!(
        merged["permissions"],
        json!({
            "deny": ["WebSearch"]
        })
    );
}

#[test]
fn common_override_strategy_recursively_merges_objects_and_replaces_conflicting_arrays() {
    let common_config = json!({
        "permissions": {
            "deny": ["Read(~/.ssh/**)"],
            "defaultMode": "ask"
        }
    });
    let extra_settings = json!({
        "permissions": {
            "allow": ["Bash"],
            "deny": ["WebSearch"],
            "defaultMode": "deny"
        }
    });

    let merged = merge_with_strategy(
        &json!({}),
        None,
        &common_config,
        None,
        None,
        Some(&extra_settings),
        ClaudeSettingsMergeStrategy::CommonOverridesProvider,
    );

    assert_eq!(merged.pointer("/permissions/allow"), Some(&json!(["Bash"])));
    assert_eq!(
        merged.pointer("/permissions/deny"),
        Some(&json!(["Read(~/.ssh/**)"]))
    );
    assert_eq!(
        merged.pointer("/permissions/defaultMode"),
        Some(&json!("ask"))
    );
}

#[test]
fn merge_strategy_combines_and_deduplicates_arrays_with_provider_scalar_priority() {
    let common_config = json!({
        "permissions": {
            "deny": ["Read(~/.ssh/**)", "WebSearch"],
            "defaultMode": "ask"
        }
    });
    let extra_settings = json!({
        "permissions": {
            "deny": ["WebSearch", "Bash"],
            "defaultMode": "deny"
        }
    });

    let merged = merge_with_strategy(
        &json!({}),
        None,
        &common_config,
        None,
        None,
        Some(&extra_settings),
        ClaudeSettingsMergeStrategy::MergeCommonAndProvider,
    );

    assert_eq!(
        merged.pointer("/permissions/deny"),
        Some(&json!(["Read(~/.ssh/**)", "WebSearch", "Bash"]))
    );
    assert_eq!(
        merged.pointer("/permissions/defaultMode"),
        Some(&json!("deny"))
    );
}

#[test]
fn switching_from_merge_strategy_removes_previous_array_union_before_reapplying() {
    let common_config = json!({
        "permissions": {
            "deny": ["Read(~/.ssh/**)"]
        }
    });
    let extra_settings = json!({
        "permissions": {
            "deny": ["WebSearch"]
        }
    });
    let previously_merged = json!({
        "permissions": {
            "deny": ["Read(~/.ssh/**)", "WebSearch"]
        }
    });

    let merged = merge_with_strategy(
        &previously_merged,
        Some(&common_config),
        &common_config,
        Some(&extra_settings),
        Some(ClaudeSettingsMergeStrategy::MergeCommonAndProvider),
        Some(&extra_settings),
        ClaudeSettingsMergeStrategy::ProviderOverridesCommon,
    );

    assert_eq!(
        merged.pointer("/permissions/deny"),
        Some(&json!(["WebSearch"]))
    );
}

#[test]
fn empty_object_extra_paths_are_removed_for_non_default_strategies() {
    for strategy in [
        ClaudeSettingsMergeStrategy::CommonOverridesProvider,
        ClaudeSettingsMergeStrategy::MergeCommonAndProvider,
    ] {
        let merged = merge_with_strategy(
            &json!({
                "foo": { "oldExtraField": true },
                "keep": true
            }),
            Some(&json!({})),
            &json!({}),
            Some(&json!({ "foo": {} })),
            Some(strategy),
            None,
            strategy,
        );

        assert!(merged.get("foo").is_none());
        assert_eq!(merged.get("keep"), Some(&json!(true)));
    }
}

#[test]
fn merge_preserves_existing_nested_status_line_details() {
    let current_disk_settings = json!({
        "statusLine": {
            "command": "ccline",
            "type": "command",
            "padding": 2
        },
        "enabledPlugins": ["jarrodwatts/claude-hud"],
        "skipWebFetchPreflight": true,
        "env": {
            "ANTHROPIC_AUTH_TOKEN": "old-token",
            "ANTHROPIC_BASE_URL": "https://old.example.com",
            "CLAUDE_CODE_ENABLE_TELEMETRY": false
        }
    });
    let previous_common_config = json!({
        "statusLine": {},
        "skipWebFetchPreflight": true
    });
    let next_common_config = json!({
        "statusLine": {},
        "skipWebFetchPreflight": false
    });
    let provider_config = json!({
        "env": {
            "ANTHROPIC_AUTH_TOKEN": "new-token",
            "ANTHROPIC_BASE_URL": "https://new.example.com"
        },
        "model": "claude-sonnet-4-5"
    });

    let merged_settings = merge_claude_settings_for_provider(
        Some(&current_disk_settings),
        Some(&previous_common_config),
        &next_common_config,
        None,
        None,
        &provider_config,
        &KNOWN_ENV_FIELDS,
    )
    .expect("merge should succeed");

    assert_eq!(
        merged_settings.get("statusLine"),
        current_disk_settings.get("statusLine")
    );
    assert_eq!(
        merged_settings.get("enabledPlugins"),
        current_disk_settings.get("enabledPlugins")
    );
    assert_eq!(
        merged_settings.get("skipWebFetchPreflight"),
        Some(&json!(false))
    );
    assert_eq!(
        merged_settings.pointer("/env/CLAUDE_CODE_ENABLE_TELEMETRY"),
        Some(&json!(false))
    );
    assert_eq!(
        merged_settings.pointer("/env/ANTHROPIC_AUTH_TOKEN"),
        Some(&json!("new-token"))
    );
    assert_eq!(
        merged_settings.pointer("/env/ANTHROPIC_BASE_URL"),
        Some(&json!("https://new.example.com"))
    );
    assert_eq!(
        merged_settings.pointer("/env/ANTHROPIC_MODEL"),
        Some(&json!("claude-sonnet-4-5"))
    );
}

#[test]
fn non_windows_target_sanitizer_removes_powershell_env_only() {
    let settings_value = json!({
        "env": {
            "CLAUDE_CODE_USE_POWERSHELL_TOOL": "1",
            "CLAUDE_CODE_SHELL": "pwsh",
            "HTTP_PROXY": "http://127.0.0.1:7890",
            "CUSTOM_ENV": "keep"
        },
        "statusLine": {
            "command": "ccline"
        }
    });

    let sanitized = sanitize_claude_settings_for_non_windows_target(&settings_value)
        .expect("sanitize should succeed")
        .expect("settings should change");

    assert!(sanitized
        .pointer("/env/CLAUDE_CODE_USE_POWERSHELL_TOOL")
        .is_none());
    assert!(sanitized.pointer("/env/CLAUDE_CODE_SHELL").is_none());
    assert_eq!(
        sanitized.pointer("/env/HTTP_PROXY"),
        Some(&json!("http://127.0.0.1:7890"))
    );
    assert_eq!(sanitized.pointer("/env/CUSTOM_ENV"), Some(&json!("keep")));
    assert_eq!(
        sanitized.pointer("/statusLine/command"),
        Some(&json!("ccline"))
    );
}

#[test]
fn clearing_common_env_preserves_user_written_env_keys() {
    let current_disk_settings = json!({
        "env": {
            "COMMON_MANAGED_ENV": "old-common-value",
            "USER_WRITTEN_ENV": "keep-user-value"
        }
    });
    let previous_common_config = json!({
        "env": {
            "COMMON_MANAGED_ENV": "old-common-value"
        }
    });

    let merged_settings = merge_claude_settings_for_provider(
        Some(&current_disk_settings),
        Some(&previous_common_config),
        &json!({}),
        None,
        None,
        &json!({}),
        &KNOWN_ENV_FIELDS,
    )
    .expect("merge should succeed");

    assert!(merged_settings.pointer("/env/COMMON_MANAGED_ENV").is_none());
    assert_eq!(
        merged_settings.pointer("/env/USER_WRITTEN_ENV"),
        Some(&json!("keep-user-value"))
    );
}

#[test]
fn merge_removes_deleted_top_level_status_line_key() {
    let current_disk_settings = json!({
        "statusLine": {
            "command": "ccline",
            "type": "command"
        },
        "skipWebFetchPreflight": true
    });
    let previous_common_config = json!({
        "statusLine": {},
        "skipWebFetchPreflight": true
    });
    let next_common_config = json!({
        "skipWebFetchPreflight": false
    });

    let merged_settings = merge_claude_settings_for_provider(
        Some(&current_disk_settings),
        Some(&previous_common_config),
        &next_common_config,
        None,
        None,
        &json!({}),
        &KNOWN_ENV_FIELDS,
    )
    .expect("merge should succeed");

    assert!(merged_settings.get("statusLine").is_none());
    assert_eq!(
        merged_settings.get("skipWebFetchPreflight"),
        Some(&json!(false))
    );
}

#[test]
fn split_excludes_runtime_owned_fields_but_keeps_status_line_in_common_config() {
    let settings_value = json!({
        "statusLine": {
            "command": "ccline",
            "type": "command"
        },
        "enabledPlugins": ["jarrodwatts/claude-hud"],
        "hooks": {
            "preToolUse": []
        },
        "skipWebFetchPreflight": true,
        "env": {
            "ANTHROPIC_AUTH_TOKEN": "token",
            "ANTHROPIC_BASE_URL": "https://example.com",
            "CLAUDE_CODE_ENABLE_TELEMETRY": false
        }
    });

    let (provider_settings, common_settings) =
        split_settings_into_provider_and_common(&settings_value, &KNOWN_ENV_FIELDS)
            .expect("split should succeed");

    assert_eq!(
        provider_settings.pointer("/env/ANTHROPIC_AUTH_TOKEN"),
        Some(&json!("token"))
    );
    assert_eq!(
        provider_settings.pointer("/env/ANTHROPIC_BASE_URL"),
        Some(&json!("https://example.com"))
    );

    assert_eq!(
        common_settings.get("skipWebFetchPreflight"),
        Some(&json!(true))
    );
    assert_eq!(
        common_settings.get("statusLine"),
        settings_value.get("statusLine")
    );
    assert!(common_settings.get("enabledPlugins").is_none());
    assert!(common_settings.get("hooks").is_none());
    assert_eq!(
        common_settings.pointer("/env/CLAUDE_CODE_ENABLE_TELEMETRY"),
        Some(&json!(false))
    );
}

#[test]
fn strip_common_config_preserves_status_line_details_for_empty_object_marker() {
    let settings_value = json!({
        "statusLine": {
            "command": "ccline",
            "type": "command",
            "padding": 2
        },
        "skipWebFetchPreflight": true
    });
    let common_config = json!({
        "statusLine": {},
        "skipWebFetchPreflight": true
    });

    let stripped = strip_claude_common_config_from_settings(&settings_value, &common_config)
        .expect("strip should succeed");

    assert_eq!(stripped.get("statusLine"), settings_value.get("statusLine"));
    assert!(stripped.get("skipWebFetchPreflight").is_none());
}

#[test]
fn strip_common_config_ignores_protected_runtime_owned_fields() {
    let settings_value = json!({
        "enabledPlugins": {
            "claude-hud": true
        },
        "hooks": {
            "preToolUse": []
        },
        "statusLine": {
            "command": "ccline"
        }
    });
    let common_config = json!({
        "enabledPlugins": {},
        "hooks": {},
        "statusLine": {}
    });

    let stripped = strip_claude_common_config_from_settings(&settings_value, &common_config)
        .expect("strip should succeed");

    assert_eq!(
        stripped.get("enabledPlugins"),
        settings_value.get("enabledPlugins")
    );
    assert_eq!(stripped.get("hooks"), settings_value.get("hooks"));
    assert_eq!(stripped.get("statusLine"), settings_value.get("statusLine"));
}

#[test]
fn extract_provider_settings_for_storage_drops_common_fields_after_strip() {
    let settings_value = json!({
        "statusLine": {
            "command": "ccline",
            "type": "command",
            "padding": 2
        },
        "skipWebFetchPreflight": true,
        "env": {
            "ANTHROPIC_AUTH_TOKEN": "token",
            "ANTHROPIC_BASE_URL": "https://example.com",
            "ANTHROPIC_REASONING_MODEL": "claude-reasoning",
            "CLAUDE_CODE_ENABLE_TELEMETRY": false
        }
    });
    let common_config = json!({
        "statusLine": {},
        "skipWebFetchPreflight": true,
        "env": {
            "CLAUDE_CODE_ENABLE_TELEMETRY": false
        }
    });

    let provider_settings = extract_provider_settings_for_storage(
        &settings_value,
        Some(&common_config),
        &KNOWN_ENV_FIELDS,
    )
    .expect("extract should succeed");

    assert!(provider_settings.get("statusLine").is_none());
    assert!(provider_settings.get("skipWebFetchPreflight").is_none());
    assert_eq!(
        provider_settings.pointer("/env/ANTHROPIC_AUTH_TOKEN"),
        Some(&json!("token"))
    );
    assert_eq!(
        provider_settings.pointer("/env/ANTHROPIC_BASE_URL"),
        Some(&json!("https://example.com"))
    );
    assert_eq!(
        provider_settings.get("reasoningModel"),
        Some(&json!("claude-reasoning"))
    );
    assert!(provider_settings
        .pointer("/env/CLAUDE_CODE_ENABLE_TELEMETRY")
        .is_none());
}

#[test]
fn extract_provider_settings_for_storage_migrates_legacy_model_fields_to_env() {
    let settings_value = json!({
        "env": {
            "ANTHROPIC_AUTH_TOKEN": "token",
            "ANTHROPIC_BASE_URL": "https://example.com"
        },
        "model": "claude-sonnet-4-5",
        "haikuModel": "claude-3-5-haiku",
        "sonnetModel": "claude-3-7-sonnet",
        "opusModel": "claude-3-opus",
        "fableModel": "claude-fable-5",
        "reasoningModel": "claude-3-7-thinking",
        "statusLine": {
            "type": "command"
        }
    });

    let provider_settings =
        extract_provider_settings_for_storage(&settings_value, None, &KNOWN_ENV_FIELDS)
            .expect("extract should succeed");

    assert_eq!(
        provider_settings.pointer("/env/ANTHROPIC_AUTH_TOKEN"),
        Some(&json!("token"))
    );
    assert_eq!(
        provider_settings.pointer("/env/ANTHROPIC_BASE_URL"),
        Some(&json!("https://example.com"))
    );
    assert_eq!(
        provider_settings.pointer("/env/ANTHROPIC_MODEL"),
        Some(&json!("claude-sonnet-4-5"))
    );
    assert_eq!(
        provider_settings.pointer("/env/ANTHROPIC_DEFAULT_HAIKU_MODEL"),
        Some(&json!("claude-3-5-haiku"))
    );
    assert_eq!(
        provider_settings.pointer("/env/ANTHROPIC_DEFAULT_SONNET_MODEL"),
        Some(&json!("claude-3-7-sonnet"))
    );
    assert_eq!(
        provider_settings.pointer("/env/ANTHROPIC_DEFAULT_OPUS_MODEL"),
        Some(&json!("claude-3-opus"))
    );
    assert_eq!(
        provider_settings.pointer("/env/ANTHROPIC_DEFAULT_FABLE_MODEL"),
        Some(&json!("claude-fable-5"))
    );
    assert_eq!(
        provider_settings.get("reasoningModel"),
        Some(&json!("claude-3-7-thinking"))
    );
    assert!(provider_settings.get("statusLine").is_none());
}

#[test]
fn split_settings_into_provider_and_common_maps_api_key_to_auth_token() {
    let settings_value = json!({
        "env": {
            "ANTHROPIC_API_KEY": "legacy-api-key",
            "ANTHROPIC_BASE_URL": "https://example.com"
        }
    });

    let (provider_settings, common_settings) =
        split_settings_into_provider_and_common(&settings_value, &KNOWN_ENV_FIELDS)
            .expect("split should succeed");

    assert_eq!(
        provider_settings.pointer("/env/ANTHROPIC_AUTH_TOKEN"),
        Some(&json!("legacy-api-key"))
    );
    assert!(provider_settings
        .pointer("/env/ANTHROPIC_API_KEY")
        .is_none());
    assert_eq!(common_settings, json!({}));
}

#[test]
fn split_settings_into_provider_and_common_keeps_model_fields_out_of_common_config() {
    let settings_value = json!({
        "env": {
            "ANTHROPIC_AUTH_TOKEN": "token",
            "ANTHROPIC_BASE_URL": "https://example.com",
            "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME": "Sonnet display",
            "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME": "Fable display"
        },
        "model": "claude-sonnet-4-5",
        "sonnetModel": "claude-3-7-sonnet",
        "fableModel": "claude-fable-5",
        "reasoningModel": "claude-3-7-thinking",
        "skipWebFetchPreflight": true
    });

    let (provider_settings, common_settings) =
        split_settings_into_provider_and_common(&settings_value, &KNOWN_ENV_FIELDS)
            .expect("split should succeed");

    assert_eq!(
        provider_settings.pointer("/env/ANTHROPIC_MODEL"),
        Some(&json!("claude-sonnet-4-5"))
    );
    assert_eq!(
        provider_settings.pointer("/env/ANTHROPIC_DEFAULT_SONNET_MODEL"),
        Some(&json!("claude-3-7-sonnet"))
    );
    assert_eq!(
        provider_settings.pointer("/env/ANTHROPIC_DEFAULT_SONNET_MODEL_NAME"),
        Some(&json!("Sonnet display"))
    );
    assert_eq!(
        provider_settings.pointer("/env/ANTHROPIC_DEFAULT_FABLE_MODEL"),
        Some(&json!("claude-fable-5"))
    );
    assert_eq!(
        provider_settings.pointer("/env/ANTHROPIC_DEFAULT_FABLE_MODEL_NAME"),
        Some(&json!("Fable display"))
    );
    assert_eq!(
        provider_settings.get("reasoningModel"),
        Some(&json!("claude-3-7-thinking"))
    );
    assert!(common_settings.get("model").is_none());
    assert!(common_settings.get("sonnetModel").is_none());
    assert!(common_settings.get("fableModel").is_none());
    assert!(common_settings.get("reasoningModel").is_none());
    assert_eq!(
        common_settings.get("skipWebFetchPreflight"),
        Some(&json!(true))
    );
}

#[test]
fn merge_writes_env_model_names_and_drops_legacy_reasoning_model() {
    let current_disk_settings = json!({
        "env": {
            "ANTHROPIC_MODEL": "stale-model",
            "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME": "stale-sonnet",
            "ANTHROPIC_DEFAULT_FABLE_MODEL": "stale-fable",
            "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME": "stale-fable-name",
            "ANTHROPIC_REASONING_MODEL": "stale-reasoning",
            "CUSTOM_ENV": "keep"
        }
    });
    let provider_config = json!({
        "env": {
            "ANTHROPIC_AUTH_TOKEN": "token",
            "ANTHROPIC_BASE_URL": "https://example.com",
            "ANTHROPIC_MODEL": "fallback-model",
            "ANTHROPIC_DEFAULT_SONNET_MODEL": "sonnet-model[1M]",
            "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME": "Sonnet display",
            "ANTHROPIC_DEFAULT_OPUS_MODEL": "opus-model",
            "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME": "Opus display",
            "ANTHROPIC_DEFAULT_FABLE_MODEL": "fable-model[1M]",
            "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME": "Fable display",
            "ANTHROPIC_REASONING_MODEL": "legacy-should-not-apply"
        },
        "reasoningModel": "legacy-top-level-should-not-apply"
    });

    let merged_settings = merge_claude_settings_for_provider(
        Some(&current_disk_settings),
        None,
        &json!({}),
        None,
        None,
        &provider_config,
        &KNOWN_ENV_FIELDS,
    )
    .expect("merge should succeed");

    assert_eq!(
        merged_settings.pointer("/env/CUSTOM_ENV"),
        Some(&json!("keep"))
    );
    assert_eq!(
        merged_settings.pointer("/env/ANTHROPIC_MODEL"),
        Some(&json!("fallback-model"))
    );
    assert_eq!(
        merged_settings.pointer("/env/ANTHROPIC_DEFAULT_SONNET_MODEL"),
        Some(&json!("sonnet-model[1M]"))
    );
    assert_eq!(
        merged_settings.pointer("/env/ANTHROPIC_DEFAULT_SONNET_MODEL_NAME"),
        Some(&json!("Sonnet display"))
    );
    assert_eq!(
        merged_settings.pointer("/env/ANTHROPIC_DEFAULT_OPUS_MODEL_NAME"),
        Some(&json!("Opus display"))
    );
    assert_eq!(
        merged_settings.pointer("/env/ANTHROPIC_DEFAULT_FABLE_MODEL"),
        Some(&json!("fable-model[1M]"))
    );
    assert_eq!(
        merged_settings.pointer("/env/ANTHROPIC_DEFAULT_FABLE_MODEL_NAME"),
        Some(&json!("Fable display"))
    );
    assert!(merged_settings
        .pointer("/env/ANTHROPIC_REASONING_MODEL")
        .is_none());
}

#[test]
fn split_keeps_subagent_and_custom_model_fields_out_of_common_config() {
    // Both bindings are provider state, not shared config: the split has to
    // hand them to the provider side and leave nothing behind in common, or a
    // channel switch would keep routing subagents through the old channel.
    let settings_value = json!({
        "env": {
            "ANTHROPIC_AUTH_TOKEN": "token",
            "ANTHROPIC_BASE_URL": "https://example.com",
            "CLAUDE_CODE_SUBAGENT_MODEL": "subagent-model[1M]",
            "ANTHROPIC_CUSTOM_MODEL_OPTION": "my-gateway/deepseek-v4-pro",
            "ANTHROPIC_CUSTOM_MODEL_OPTION_NAME": "DeepSeek via gateway"
        },
        "skipWebFetchPreflight": true
    });

    let (provider_settings, common_settings) =
        split_settings_into_provider_and_common(&settings_value, &KNOWN_ENV_FIELDS)
            .expect("split should succeed");

    assert_eq!(
        provider_settings.pointer("/env/CLAUDE_CODE_SUBAGENT_MODEL"),
        Some(&json!("subagent-model[1M]"))
    );
    assert_eq!(
        provider_settings.pointer("/env/ANTHROPIC_CUSTOM_MODEL_OPTION"),
        Some(&json!("my-gateway/deepseek-v4-pro"))
    );
    assert_eq!(
        provider_settings.pointer("/env/ANTHROPIC_CUSTOM_MODEL_OPTION_NAME"),
        Some(&json!("DeepSeek via gateway"))
    );
    assert_eq!(
        common_settings.get("skipWebFetchPreflight"),
        Some(&json!(true))
    );
}

#[test]
fn merge_clears_the_previous_channels_subagent_and_custom_bindings() {
    // The disk still holds the channel we are leaving. Its subagent pin and
    // picker entry describe models only that channel has, so carrying them into
    // the next one would be wrong; the next channel's own values replace them.
    let current_disk_settings = json!({
        "env": {
            "CLAUDE_CODE_SUBAGENT_MODEL": "stale-subagent",
            "ANTHROPIC_CUSTOM_MODEL_OPTION": "stale-custom",
            "ANTHROPIC_CUSTOM_MODEL_OPTION_NAME": "Stale custom",
            "CUSTOM_ENV": "keep"
        }
    });
    let provider_config = json!({
        "env": {
            "ANTHROPIC_AUTH_TOKEN": "token",
            "ANTHROPIC_BASE_URL": "https://example.com",
            "CLAUDE_CODE_SUBAGENT_MODEL": "subagent-model[1M]",
            "ANTHROPIC_CUSTOM_MODEL_OPTION": "my-gateway/deepseek-v4-pro",
            "ANTHROPIC_CUSTOM_MODEL_OPTION_NAME": "DeepSeek via gateway"
        }
    });

    let merged = merge_claude_settings_for_provider(
        Some(&current_disk_settings),
        None,
        &json!({}),
        None,
        None,
        &provider_config,
        &KNOWN_ENV_FIELDS,
    )
    .expect("merge should succeed");

    assert_eq!(
        merged.pointer("/env/CLAUDE_CODE_SUBAGENT_MODEL"),
        Some(&json!("subagent-model[1M]"))
    );
    assert_eq!(
        merged.pointer("/env/ANTHROPIC_CUSTOM_MODEL_OPTION"),
        Some(&json!("my-gateway/deepseek-v4-pro"))
    );
    assert_eq!(
        merged.pointer("/env/ANTHROPIC_CUSTOM_MODEL_OPTION_NAME"),
        Some(&json!("DeepSeek via gateway"))
    );
    // Untouched runtime-owned env survives the channel switch.
    assert_eq!(merged.pointer("/env/CUSTOM_ENV"), Some(&json!("keep")));
}

#[test]
fn extra_settings_cannot_smuggle_in_the_subagent_or_custom_bindings() {
    // `extra_settings_config` is provider-private, but these three are derived
    // from the form. Letting extra settings carry them would make the form and
    // the file disagree on which model subagents run.
    let extra_settings = json!({
        "env": {
            "CLAUDE_CODE_SUBAGENT_MODEL": "smuggled-subagent",
            "ANTHROPIC_CUSTOM_MODEL_OPTION": "smuggled-custom",
            "ANTHROPIC_CUSTOM_MODEL_OPTION_NAME": "Smuggled custom",
            "KEEP_ME": "keep"
        }
    });

    let merged = merge_with_strategy(
        &json!({}),
        None,
        &json!({}),
        None,
        None,
        Some(&extra_settings),
        ClaudeSettingsMergeStrategy::ProviderOverridesCommon,
    );

    assert!(merged.pointer("/env/CLAUDE_CODE_SUBAGENT_MODEL").is_none());
    assert!(merged.pointer("/env/ANTHROPIC_CUSTOM_MODEL_OPTION").is_none());
    assert!(merged
        .pointer("/env/ANTHROPIC_CUSTOM_MODEL_OPTION_NAME")
        .is_none());
    assert_eq!(merged.pointer("/env/KEEP_ME"), Some(&json!("keep")));
}
