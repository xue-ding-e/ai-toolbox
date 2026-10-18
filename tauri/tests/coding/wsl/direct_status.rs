use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ai_toolbox_lib::coding::{dsh, hermes, runtime_location, wsl};
use ai_toolbox_lib::db::SqliteDbState;
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
use tauri::Manager;

static TEST_LOCK: Mutex<()> = Mutex::new(());

struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &Path) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

fn create_app() -> tauri::App<MockRuntime> {
    mock_builder()
        .manage(SqliteDbState::in_memory_for_test().expect("create test database"))
        .build(mock_context(noop_assets()))
        .expect("build mock app")
}

async fn save_root(app: &tauri::App<MockRuntime>, module: &str, root: Option<&str>) {
    let result = match module {
        "dsh" => {
            dsh::save_dsh_settings_config(
                app.state(),
                app.handle().clone(),
                dsh::types::DshSettingsConfigInput {
                    root_dir: root.map(str::to_string),
                    clear_root_dir: root.is_none(),
                    profile: None,
                },
            )
            .await
        }
        "hermes" => {
            hermes::save_hermes_settings_config(
                app.state(),
                app.handle().clone(),
                hermes::types::HermesSettingsConfigInput {
                    config_dir: root.map(str::to_string),
                    clear_config_dir: root.is_none(),
                },
            )
            .await
        }
        _ => unreachable!(),
    };
    result.expect("save runtime root through the settings command");
}

async fn assert_status(
    app: &tauri::App<MockRuntime>,
    module: &str,
    root: &str,
    expected_wsl: Option<(&str, &str)>,
) {
    let config = wsl::wsl_get_config(app.state())
        .await
        .expect("read WSL settings after saving root");
    let status = config
        .module_statuses
        .iter()
        .find(|status| status.module == module)
        .expect("module must be included in the settings status payload");
    assert_eq!(status.source_path.as_deref(), Some(root));
    assert_eq!(status.is_wsl_direct, expected_wsl.is_some());
    assert_eq!(
        status.distro.as_deref(),
        expected_wsl.map(|(distro, _)| distro)
    );
    assert_eq!(
        status.linux_path.as_deref(),
        expected_wsl.map(|(_, path)| path)
    );

    // The modal consumes this camelCase payload for disabled tabs and skipModules.
    let payload = serde_json::to_value(&config).expect("serialize frontend settings payload");
    let frontend_status = payload["moduleStatuses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|status| status["module"] == module)
        .unwrap();
    assert_eq!(frontend_status["isWslDirect"], expected_wsl.is_some());

    let state = app.state::<SqliteDbState>();
    let direct_modules = runtime_location::get_wsl_direct_modules_async(&state)
        .await
        .expect("read the backend skip set used by full and MCP sync");
    assert_eq!(direct_modules.contains(module), expected_wsl.is_some());
    let cached_status = runtime_location::get_wsl_direct_status_for_module(&state, module)
        .expect("read synchronous status from the refreshed cache");
    assert_eq!(cached_status.source_path, status.source_path);
    assert_eq!(cached_status.is_wsl_direct, status.is_wsl_direct);
}

#[tokio::test]
async fn dsh_and_hermes_saved_roots_refresh_wsl_status_without_restart() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let temp_dir = tempfile::tempdir().expect("create fallback directory");
    let fallback_root = temp_dir.path().join("fallback");
    let local_root = temp_dir.path().join("custom-local");
    let _dsh_env = EnvVarGuard::set("DSH_HOME", &fallback_root);
    let _hermes_env = EnvVarGuard::set("HERMES_HOME", &fallback_root);
    let app = create_app();

    for (module, root, linux_path) in [
        (
            "dsh",
            r"\\wsl.localhost\openSUSE-Tumbleweed\root\.dsh",
            "/root/.dsh",
        ),
        (
            "hermes",
            r"\\wsl.localhost\openSUSE-Tumbleweed\root\custom-hermes",
            "/root/custom-hermes",
        ),
    ] {
        // Warm the cache with a local path, then exercise the real save command.
        runtime_location::refresh_runtime_location_cache_for_module_async(
            &app.state::<SqliteDbState>(),
            module,
        )
        .await
        .expect("warm local runtime cache");
        assert_status(&app, module, fallback_root.to_str().unwrap(), None).await;

        save_root(&app, module, Some(root)).await;
        assert_status(
            &app,
            module,
            root,
            Some(("openSUSE-Tumbleweed", linux_path)),
        )
        .await;

        let alias_root = r"\\wsl$\Ubuntu\home\tester\custom-config";
        save_root(&app, module, Some(alias_root)).await;
        assert_status(
            &app,
            module,
            alias_root,
            Some(("Ubuntu", "/home/tester/custom-config")),
        )
        .await;

        save_root(&app, module, Some(local_root.to_str().unwrap())).await;
        assert_status(&app, module, local_root.to_str().unwrap(), None).await;

        save_root(&app, module, Some(root)).await;
        save_root(&app, module, None).await;
        assert_status(&app, module, fallback_root.to_str().unwrap(), None).await;
    }
}

#[tokio::test]
async fn dsh_and_hermes_env_roots_are_direct_and_custom_roots_take_precedence() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let root = r"\\wsl.localhost\Debian\home\tester\custom-harness";
    let _dsh_env = EnvVarGuard::set("DSH_HOME", Path::new(root));
    let _hermes_env = EnvVarGuard::set("HERMES_HOME", Path::new(root));
    let local_dir = tempfile::tempdir().expect("create custom local directory");
    let app = create_app();

    for module in ["dsh", "hermes"] {
        runtime_location::refresh_runtime_location_cache_for_module_async(
            &app.state::<SqliteDbState>(),
            module,
        )
        .await
        .expect("resolve environment root on startup");
        assert_status(
            &app,
            module,
            root,
            Some(("Debian", "/home/tester/custom-harness")),
        )
        .await;
        save_root(&app, module, Some(local_dir.path().to_str().unwrap())).await;
        assert_status(&app, module, local_dir.path().to_str().unwrap(), None).await;
        save_root(&app, module, None).await;
        assert_status(
            &app,
            module,
            root,
            Some(("Debian", "/home/tester/custom-harness")),
        )
        .await;
    }

    let state = app.state::<SqliteDbState>();
    let expected_skills = PathBuf::from(root).join("skills");
    assert_eq!(
        runtime_location::get_tool_skills_path_async(&state, "hermes").await,
        Some(expected_skills.clone())
    );
    assert_eq!(
        runtime_location::get_tool_skills_path_sync(&state, "hermes"),
        Some(expected_skills)
    );
    assert!(runtime_location::get_tool_skills_path_async(&state, "dsh")
        .await
        .is_none());
}
