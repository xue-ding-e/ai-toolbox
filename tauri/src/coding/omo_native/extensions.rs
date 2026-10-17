//! OmO Native 的扩展管理命令层。
//!
//! **逻辑全在 `crate::coding::cli_extensions`**——`omo` 与 `pi` 共用 senpi 引擎的
//! 包体系（同一份 `settings.json` 形状、同一套子命令、逐字相同的 `list` 输出，
//! 2026-10-07 实测 `diff` 只差路径），差异只有 [`OMO_EXTENSION_SPEC`] 里那几项。
//!
//! **实测确认的等价性**（改这里的常量前先看）：
//! - `omo list` 对空数组过滤的包同样打印 `(filtered)`；
//! - `omo list` 同样认顶层 `extensions` 数组的 `-path` 强制排除；
//! - `omo` 二进制里只有 `PI_OFFLINE`，没有 `OMO_OFFLINE`——离线开关沿用 Pi 的拼写；
//! - agent 目录环境变量用 `OMO_CODING_AGENT_DIR`（引擎认三个别名，它排第一）。

use std::path::Path;

use super::types::{
    OmoNativeExtensionActionInput, OmoNativeExtensionCommandResult,
    OmoNativeExtensionEnabledInput, OmoNativeExtensionInstallInput, OmoNativeExtensionKind,
    OmoNativeExtensionListResult, OmoNativeExtensionScope, OmoNativeExtensionSummary,
    OmoNativeExtensionUpdateInput,
};
use crate::coding::cli_extensions::{
    self as shared, CliExtensionKind, CliExtensionScope, CliExtensionSpec,
};
use crate::coding::cli_resolver::resolve_local_cli_by_name;
use crate::coding::runtime_location;
use crate::db::SqliteDbState;

/// OmO Native 与共享实现之间的全部差异。
pub const OMO_EXTENSION_SPEC: CliExtensionSpec = CliExtensionSpec {
    cli_name: "omo",
    env_key: "OMO_CODING_AGENT_DIR",
    // 引擎里只有这个拼写（`omo` 二进制里搜不到 `OMO_OFFLINE`）。
    offline_env_key: "PI_OFFLINE",
    extensions_dir: "extensions",
    // 本机两个 CLI 的内置扩展前缀相同；`omo` 侧暂无自己的内置包。
    protected_prefixes: &["pi-deck-", "ai-toolbox-"],
    // senpi 引擎的包加载器认第五类资源 `hooks`（`RESOURCE_TYPES2`，文件模式
    // `*.json`）；Pi 的 package-manager 只有四类。判「整包不加载」与禁用写
    // `[]` 都必须带上它，否则只含 hooks 的包会被误判已禁用、禁用后 hooks
    // 仍在加载。
    resource_type_keys: &["extensions", "skills", "prompts", "themes", "hooks"],
    scan_timeout: std::time::Duration::from_secs(20),
    wsl_sync_event: "wsl-sync-request-omo-native",
};

fn to_omo_scope(scope: CliExtensionScope) -> OmoNativeExtensionScope {
    match scope {
        CliExtensionScope::User => OmoNativeExtensionScope::User,
        CliExtensionScope::Project => OmoNativeExtensionScope::Project,
        CliExtensionScope::Unknown => OmoNativeExtensionScope::Unknown,
    }
}

fn from_omo_scope(scope: OmoNativeExtensionScope) -> CliExtensionScope {
    match scope {
        OmoNativeExtensionScope::User => CliExtensionScope::User,
        OmoNativeExtensionScope::Project => CliExtensionScope::Project,
        OmoNativeExtensionScope::Unknown => CliExtensionScope::Unknown,
    }
}

fn to_omo_kind(kind: CliExtensionKind) -> OmoNativeExtensionKind {
    match kind {
        CliExtensionKind::Package => OmoNativeExtensionKind::Package,
        CliExtensionKind::LocalFile => OmoNativeExtensionKind::LocalFile,
        CliExtensionKind::LocalDirectory => OmoNativeExtensionKind::LocalDirectory,
    }
}

fn from_omo_kind(kind: OmoNativeExtensionKind) -> CliExtensionKind {
    match kind {
        OmoNativeExtensionKind::Package => CliExtensionKind::Package,
        OmoNativeExtensionKind::LocalFile => CliExtensionKind::LocalFile,
        OmoNativeExtensionKind::LocalDirectory => CliExtensionKind::LocalDirectory,
    }
}

fn to_omo_summary(summary: shared::CliExtensionSummary) -> OmoNativeExtensionSummary {
    OmoNativeExtensionSummary {
        id: summary.id,
        source: summary.source,
        scope: to_omo_scope(summary.scope),
        kind: to_omo_kind(summary.kind),
        path: summary.path,
        built_in: summary.built_in,
        current_version: summary.current_version,
        latest_version: summary.latest_version,
        update_available: summary.update_available,
        enabled: summary.enabled,
        switch_supported: summary.switch_supported,
    }
}

/// OmO 的宿主侧二进制与 runtime location。
///
/// `omo` 装在 `~/.local/bin`（安装器放的位置），`resolve_local_cli_by_name` 的候选
/// 目录里正好有它；引擎状态目录由 `get_omo_native_runtime_location_async` 决议。
struct OmoExtensionContext {
    program: shared::ResolvedCliProgram,
    runtime_location: runtime_location::RuntimeLocationInfo,
    settings_path: std::path::PathBuf,
}

async fn resolve_context(db: &SqliteDbState) -> Result<OmoExtensionContext, String> {
    let runtime_location = runtime_location::get_omo_native_runtime_location_async(db).await?;
    let program = resolve_local_cli_by_name("omo").ok_or_else(|| {
        crate::coding::cli_resolver::local_cli_missing_hint("omo")
    })?;
    Ok(OmoExtensionContext {
        program: shared::ResolvedCliProgram { path: program.path },
        settings_path: runtime_location.host_path.join(
            super::providers::resolve_omo_native_settings_file_name(&runtime_location.host_path),
        ),
        runtime_location,
    })
}

#[tauri::command]
pub async fn list_omo_native_extensions(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<OmoNativeExtensionListResult, String> {
    let db = state.db();
    let context = resolve_context(&db).await?;
    let result = shared::list_cli_extensions(
        &OMO_EXTENSION_SPEC,
        &db,
        &context.program,
        &context.runtime_location,
        &context.settings_path,
    )
    .await?;

    Ok(OmoNativeExtensionListResult {
        extensions_path: result.extensions_path,
        packages_path: result.packages_path,
        extensions: result.extensions.into_iter().map(to_omo_summary).collect(),
        raw: result.raw,
        cli_path: result.cli_path,
        cli_version: result.cli_version,
    })
}

#[tauri::command]
pub async fn install_omo_native_extension(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: OmoNativeExtensionInstallInput,
) -> Result<OmoNativeExtensionCommandResult, String> {
    let db = state.db();
    let context = resolve_context(&db).await?;
    let result = shared::install_cli_extension(
        &OMO_EXTENSION_SPEC,
        &app,
        "omo-native-extensions",
        &context.program,
        &context.runtime_location,
        &shared::CliExtensionInstallInput {
            source: input.source,
        },
    )
    .await?;
    Ok(OmoNativeExtensionCommandResult {
        command: result.command,
        output: result.output,
    })
}

#[tauri::command]
pub async fn uninstall_omo_native_extension(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: OmoNativeExtensionActionInput,
) -> Result<OmoNativeExtensionCommandResult, String> {
    let db = state.db();
    let context = resolve_context(&db).await?;
    let result = shared::uninstall_cli_extension(
        &OMO_EXTENSION_SPEC,
        &app,
        "omo-native-extensions",
        &context.program,
        &context.runtime_location,
        &shared::CliExtensionActionInput {
            source: input.source,
            scope: input.scope.map(from_omo_scope),
            kind: input.kind.map(from_omo_kind),
            path: input.path,
        },
    )
    .await?;
    Ok(OmoNativeExtensionCommandResult {
        command: result.command,
        output: result.output,
    })
}

#[tauri::command]
pub async fn update_omo_native_extensions(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: Option<OmoNativeExtensionUpdateInput>,
) -> Result<OmoNativeExtensionCommandResult, String> {
    let db = state.db();
    let context = resolve_context(&db).await?;
    let shared_input = input.map(|value| shared::CliExtensionUpdateInput {
        source: value.source,
    });
    let result = shared::update_cli_extensions(
        &OMO_EXTENSION_SPEC,
        &app,
        "omo-native-extensions",
        &context.program,
        &context.runtime_location,
        shared_input.as_ref(),
    )
    .await?;
    Ok(OmoNativeExtensionCommandResult {
        command: result.command,
        output: result.output,
    })
}

#[tauri::command]
pub async fn set_omo_native_extension_enabled(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: OmoNativeExtensionEnabledInput,
) -> Result<(), String> {
    let db = state.db();
    let context = resolve_context(&db).await?;
    shared::set_cli_extension_enabled(
        &OMO_EXTENSION_SPEC,
        &app,
        "omo-native-extensions",
        &context.settings_path,
        &shared::CliExtensionEnabledInput {
            source: input.source,
            kind: from_omo_kind(input.kind),
            enabled: input.enabled,
        },
    )
    .await
}

/// 本地扩展目录（`<agentDir>/extensions`）。
pub fn omo_native_extensions_path_from_root(root_dir: &Path) -> std::path::PathBuf {
    shared::extensions_path_from_root(root_dir, &OMO_EXTENSION_SPEC)
}

pub fn omo_native_packages_path_from_root(root_dir: &Path) -> std::path::PathBuf {
    shared::packages_path_from_root(root_dir)
}
