use crate::app_updater;
use crate::deeplink;
use crate::profiles::Profile;
use crate::service;
use crate::updater;
use crate::vpn::{LogPayload, StatePayload};
use crate::AppState;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, State};

#[tauri::command]
pub fn list_profiles(state: State<AppState>) -> Vec<Profile> {
    state.profiles.list()
}

#[tauri::command]
pub fn get_active_profile_id(state: State<AppState>) -> Option<String> {
    state.profiles.active_id()
}

#[tauri::command]
pub fn set_active_profile_id(state: State<AppState>, id: Option<String>) -> Result<(), String> {
    state.profiles.set_active(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_profile(state: State<AppState>, profile: Profile) -> Result<Profile, String> {
    state.profiles.upsert(profile).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_profile(state: State<AppState>, id: String) -> Result<(), String> {
    state.profiles.delete(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn new_blank_profile() -> Profile {
    Profile::new_blank("Новый сервер")
}

#[tauri::command]
pub fn profile_to_toml(profile: Profile) -> String {
    profile.to_client_toml()
}

#[tauri::command]
pub fn import_toml_text(
    state: State<AppState>,
    text: String,
    fallback_name: Option<String>,
) -> Result<Profile, String> {
    let name = fallback_name.unwrap_or_else(|| "Imported".to_string());
    state
        .profiles
        .import_toml_text(&text, &name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn import_toml_file(state: State<AppState>, path: String) -> Result<Profile, String> {
    state
        .profiles
        .import_toml_file(&PathBuf::from(path))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn import_deeplink(state: State<AppState>, uri: String) -> Result<Profile, String> {
    let profile = deeplink::parse_tt_uri(&uri).map_err(|e| e.to_string())?;
    state.profiles.upsert(profile).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn extract_deeplink_from_text(text: String) -> Option<String> {
    deeplink::extract_tt_uri(&text)
}

#[tauri::command]
pub async fn vpn_connect(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<(), String> {
    let profile = state
        .profiles
        .get(&profile_id)
        .ok_or_else(|| format!("Профиль не найден: {}", profile_id))?;
    state.profiles.set_active(Some(profile.id.clone())).ok();
    state
        .vpn
        .connect(app, profile)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn vpn_disconnect(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.vpn.disconnect(app).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn vpn_state(state: State<AppState>) -> StatePayload {
    state.vpn.state()
}

#[tauri::command]
pub fn vpn_logs(state: State<AppState>) -> Vec<LogPayload> {
    state.vpn.logs()
}

#[tauri::command]
pub fn service_status() -> service::ServiceStatus {
    service::query_service()
}

#[tauri::command]
pub fn service_install(state: State<AppState>, profile_id: String) -> Result<(), String> {
    let profile = state
        .profiles
        .get(&profile_id)
        .ok_or_else(|| "Профиль не найден".to_string())?;
    let toml = profile.to_client_toml();
    let bin = state.vpn.binary_path();
    service::install_service(&bin, &toml).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn service_uninstall(state: State<AppState>) -> Result<(), String> {
    let bin = state.vpn.binary_path();
    service::uninstall_service(&bin).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn binary_info(state: State<AppState>) -> serde_json::Value {
    let path = state.vpn.binary_path();
    serde_json::json!({
        "path": path.display().to_string(),
        "exists": path.exists(),
    })
}

#[tauri::command]
pub fn open_app_data_folder(_app: AppHandle, state: State<AppState>) -> Result<String, String> {
    let dir = state.app_data_dir.clone();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("create_dir_all {}: {}", dir.display(), e))?;
    // Touch a marker file to make sure the directory is properly materialized.
    let marker = dir.join(".keep");
    let _ = std::fs::write(&marker, b"");

    #[cfg(windows)]
    {
        // When running elevated, talking to the user's shell directly is flaky
        // (ShellExecute and `explorer.exe path` are routed through the existing
        // user-session explorer, which sometimes refuses IPC from elevated callers).
        // Spawning a fresh detached explorer process is the most reliable approach.
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;

        let spawn_result = std::process::Command::new("explorer.exe")
            .arg(&dir)
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
            .spawn();

        if spawn_result.is_err() {
            // Last-resort fallback via `cmd /c start`.
            std::process::Command::new("cmd")
                .args(["/c", "start", "", &dir.display().to_string()])
                .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
                .spawn()
                .map_err(|e| format!("cmd /c start failed: {}", e))?;
        }
    }
    #[cfg(not(windows))]
    {
        let cmd = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        std::process::Command::new(cmd)
            .arg(&dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(dir.display().to_string())
}

#[tauri::command]
pub fn app_data_dir(state: State<AppState>) -> String {
    state.app_data_dir.display().to_string()
}

#[tauri::command]
pub fn platform_info() -> serde_json::Value {
    serde_json::json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "is_windows": cfg!(windows),
        "is_macos": cfg!(target_os = "macos"),
        "is_linux": cfg!(target_os = "linux"),
        "autostart_supported": cfg!(windows),
    })
}

/// Windows-only: relaunch the whole GUI elevated. On Linux/macOS the GUI
/// stays unprivileged by design — only the VPN client child is elevated
/// (see vpn.rs), because a WebKit GUI running as root crashes on most distros.
#[tauri::command]
pub fn restart_as_admin(app: AppHandle) -> Result<(), String> {
    #[cfg(windows)]
    {
        use crate::elevate;
        if elevate::is_elevated() {
            return Ok(());
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        elevate::run_elevated(&exe, "", exe.parent()).map_err(|e| e.to_string())?;
        // Give the new instance a moment to start before exiting.
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            app.exit(0);
        });
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Err("Не требуется: на этой платформе права запрашиваются при подключении".into())
    }
}

#[tauri::command]
pub fn is_elevated() -> bool {
    crate::elevate::is_elevated()
}

/// Linux/macOS: remove the app, its data and desktop integration, then quit.
/// Windows has a proper uninstaller in "Apps & features" instead.
#[tauri::command]
pub async fn uninstall_app(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    #[cfg(unix)]
    {
        // Stop the VPN first so the elevated client doesn't outlive the app.
        let _ = state.vpn.disconnect(app.clone()).await;
        crate::uninstall::uninstall(&state.app_data_dir).map_err(|e| e.to_string())?;
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(700));
            app.exit(0);
        });
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (app, state);
        Err("На Windows удаляйте приложение через «Установка и удаление программ»".into())
    }
}

#[tauri::command]
pub fn show_window(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    Ok(())
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
pub async fn check_for_update(state: State<'_, AppState>) -> Result<updater::UpdateStatus, String> {
    let app_data_dir = state.app_data_dir.clone();
    let settings = state.settings.clone();
    let release = updater::fetch_latest_release()
        .await
        .map_err(|e| e.to_string())?;
    let status = updater::build_status(&app_data_dir, None, Some(&release));
    // Cache for next launch.
    let _ = settings.patch(crate::settings::Settings {
        last_known_update: Some(serde_json::to_value(&status).unwrap_or_default()),
        ..Default::default()
    });
    Ok(status)
}

#[tauri::command]
pub fn cached_update_status(state: State<AppState>) -> Option<serde_json::Value> {
    state.settings.get().last_known_update
}

#[derive(serde::Serialize)]
pub struct AllUpdatesStatus {
    pub client: updater::UpdateStatus,
    pub app: app_updater::AppUpdateStatus,
}

/// One-shot check that hits BOTH update sources at once and caches the results.
/// Used by the unified "Проверить" button in Settings.
#[tauri::command]
pub async fn check_all_updates(state: State<'_, AppState>) -> Result<AllUpdatesStatus, String> {
    let app_data_dir = state.app_data_dir.clone();
    let settings_store = state.settings.clone();

    // Run both fetches concurrently.
    let (client_release, app_status) =
        tokio::join!(updater::fetch_latest_release(), app_updater::fetch_latest(),);

    let client = client_release
        .map(|rel| updater::build_status(&app_data_dir, None, Some(&rel)))
        .map_err(|e| format!("VPN-клиент: {}", e))?;

    let app = app_status.unwrap_or_else(|_| app_updater::fallback_status());

    let _ = settings_store.patch(crate::settings::Settings {
        last_known_update: Some(serde_json::to_value(&client).unwrap_or_default()),
        last_known_app_update: Some(serde_json::to_value(&app).unwrap_or_default()),
        ..Default::default()
    });

    Ok(AllUpdatesStatus { client, app })
}

#[tauri::command]
pub fn cached_app_update_status(state: State<AppState>) -> Option<serde_json::Value> {
    state.settings.get().last_known_app_update
}

#[tauri::command]
pub fn app_version() -> String {
    app_updater::current_version()
}

/// Auto-install the latest GUI version via tauri-plugin-updater.
///
/// Requires `plugins.updater.pubkey` in tauri.conf.json to be a valid Ed25519
/// public key, and the CI build to have been signed with the matching private
/// key (via `TAURI_SIGNING_PRIVATE_KEY` env var). If signing isn't configured,
/// this command returns an error and the frontend falls back to opening the
/// GitHub release page.
#[tauri::command]
pub async fn install_app_update(app: AppHandle) -> Result<bool, String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = match updater.check().await {
        Ok(Some(u)) => u,
        Ok(None) => return Ok(false), // already up-to-date
        Err(e) => return Err(format!("Проверка обновлений: {}", e)),
    };

    // Stream progress events to the frontend.
    let app_for_progress = app.clone();
    let mut total_downloaded: u64 = 0;
    update
        .download_and_install(
            move |chunk_length, content_length| {
                total_downloaded += chunk_length as u64;
                let _ = app_for_progress.emit(
                    "app-update://progress",
                    serde_json::json!({
                        "done": total_downloaded,
                        "total": content_length,
                    }),
                );
            },
            move || {
                // download finished, install starting
            },
        )
        .await
        .map_err(|e| format!("Установка: {}", e))?;

    // Restart the app — the new binary will start.
    app.restart();
}

#[tauri::command]
pub fn open_app_release_page(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    // Prefer the cached release URL (specific version page) if available;
    // otherwise the generic /releases/latest page.
    let cached = state.settings.get().last_known_app_update;
    let url = cached
        .as_ref()
        .and_then(|v| v.get("release_url"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| "https://github.com/lopata29435/lopataTTC/releases/latest".to_string());
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> crate::settings::Settings {
    state.settings.get()
}

#[tauri::command]
pub fn update_settings(
    app: AppHandle,
    state: State<AppState>,
    patch: crate::settings::Settings,
) -> Result<crate::settings::Settings, String> {
    let language_changed = patch.language.is_some();
    let result = state.settings.patch(patch).map_err(|e| e.to_string())?;
    if language_changed {
        crate::rebuild_tray_menu(&app);
    }
    Ok(result)
}

#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<updater::UpdateStatus, String> {
    let app_data_dir = state.app_data_dir.clone();
    let release = updater::fetch_latest_release()
        .await
        .map_err(|e| e.to_string())?;
    let app2 = app.clone();
    let progress = move |done: u64, total: Option<u64>| {
        let _ = app2.emit(
            "update://progress",
            serde_json::json!({ "done": done, "total": total }),
        );
    };
    let new_binary = updater::download_and_install(&release, &app_data_dir, progress)
        .await
        .map_err(|e| e.to_string())?;

    // Activate the new binary immediately.
    {
        let app_state: State<AppState> = app.state();
        *app_state.binary_path.lock().unwrap() = new_binary.clone();
        app_state.vpn.set_binary_path(new_binary.clone());
    }

    let status = updater::build_status(&app_data_dir, None, Some(&release));
    Ok(status)
}

/// Diagnostic: register the tt:// scheme handler explicitly and return whether it worked.
#[tauri::command]
pub fn register_deeplink_scheme(app: AppHandle) -> Result<String, String> {
    #[cfg(desktop)]
    {
        use tauri_plugin_deep_link::DeepLinkExt;
        app.deep_link()
            .register("tt")
            .map(|_| "tt:// зарегистрирован".to_string())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(desktop))]
    {
        let _ = app;
        Err("not supported".into())
    }
}
