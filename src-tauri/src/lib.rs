pub mod app_updater;
pub mod commands;
pub mod deeplink;
pub mod elevate;
pub mod logging;
pub mod profiles;
pub mod service;
pub mod settings;
pub mod uninstall;
pub mod updater;
pub mod vpn;

use crate::logging::log;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};

use crate::profiles::ProfileStore;
use crate::settings::SettingsStore;
use crate::vpn::VpnService;

pub struct AppState {
    pub profiles: Arc<ProfileStore>,
    pub vpn: Arc<VpnService>,
    pub app_data_dir: PathBuf,
    pub binary_path: Arc<Mutex<PathBuf>>,
    pub settings: Arc<SettingsStore>,
    /// tt:// URIs the app was launched with (cold start via browser click).
    /// The frontend drains these once it is ready to import.
    pub startup_deeplinks: Mutex<Vec<String>>,
}

/// Entry point invoked by main.rs (and by tauri::Builder on mobile in the future).
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // NOTE on privileges:
    //  * Windows — the whole GUI runs elevated (requireAdministrator in
    //    app.manifest), the VPN child inherits admin.
    //  * Linux/macOS — the GUI deliberately stays unprivileged. Running a
    //    WebKit GUI as root is unsupported (WebKitGTK crashes under root on
    //    most distros). Only the client process is elevated, via
    //    pkexec/osascript at connect time — see vpn.rs.

    // Set up file logging + panic hook as the very first thing: the GUI has
    // no console, so this is the only way to see why something died.
    let log_dir = pick_app_data_dir().unwrap_or_else(std::env::temp_dir);
    logging::init(&log_dir);
    log(&format!(
        "=== start v{} pid={} argv={:?}",
        env!("CARGO_PKG_VERSION"),
        std::process::id(),
        std::env::args()
            .map(|a| if a.starts_with("tt://") {
                logging::redact_link(&a)
            } else {
                a
            })
            .collect::<Vec<_>>()
    ));

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // A second instance was launched. If args contain a tt:// URI,
            // forward it as a deep-link event. Then bring the existing window to the front.
            log(&format!(
                "single-instance: second instance argv={:?}",
                argv.iter()
                    .map(|a| if a.starts_with("tt://") {
                        logging::redact_link(a)
                    } else {
                        a.clone()
                    })
                    .collect::<Vec<_>>()
            ));
            // IMPORTANT: emit under our OWN event name. Emitting as
            // "deep-link://new-url" (the deep-link plugin's internal event)
            // makes the plugin re-fire on_open_url, whose handler emits
            // again — an infinite event storm that kills the app.
            for arg in argv.iter().skip(1) {
                if arg.starts_with("tt://") {
                    let _ = app.emit("lopata://deeplink", vec![arg.clone()]);
                }
            }
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
            log("single-instance: forwarded and focused");
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // Resolve app data dir. We override Tauri's default (which uses the
            // dotted identifier and produces folder names like `org.trusttunnel.gui`)
            // with a human-friendly `TrustTunnel` directory in the platform's
            // standard user-config location.
            let app_data_dir = pick_app_data_dir().unwrap_or_else(|| {
                app.path()
                    .app_data_dir()
                    .unwrap_or_else(|_| std::env::temp_dir().join("TrustTunnel"))
            });
            std::fs::create_dir_all(&app_data_dir).ok();

            // One-time migration: if user previously had the dotted folder
            // (`<roaming>/org.trusttunnel.gui`) and the new clean folder is empty,
            // move profiles + current.toml across so they don't have to re-import.
            if let Ok(old_dir) = app.path().app_data_dir() {
                if old_dir != app_data_dir && old_dir.exists() {
                    let new_empty = std::fs::read_dir(&app_data_dir)
                        .map(|mut it| it.next().is_none())
                        .unwrap_or(true);
                    if new_empty {
                        let _ = copy_dir_recursive(&old_dir, &app_data_dir);
                    }
                }
            }

            let profiles_dir = app_data_dir.join("profiles");
            let profiles = ProfileStore::open(profiles_dir)?;
            let profiles = Arc::new(profiles);

            // Resolve initial binary path: prefer a previously downloaded version under
            // app_data_dir/clients/v*, otherwise fall back to the bundled resource.
            let resource_dir = app.path().resource_dir().ok();
            let initial_binary = updater::locate_installed_client(&app_data_dir)
                .map(|(_, p)| p)
                .unwrap_or_else(|| resolve_binary_path(resource_dir.as_deref(), &app_data_dir));
            let binary_path: Arc<Mutex<PathBuf>> = Arc::new(Mutex::new(initial_binary));
            let config_path = app_data_dir.join("current.toml");

            let vpn = Arc::new(VpnService::new(binary_path.clone(), config_path));

            // Register :: tt:// deep-link scheme at runtime (dev convenience).
            #[cfg(any(windows, target_os = "linux"))]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                let _ = app.deep_link().register("tt");
            }

            // Listen for deeplink events
            #[cfg(desktop)]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                let app_handle = app.handle().clone();
                app.deep_link().on_open_url(move |event| {
                    let urls: Vec<String> = event.urls().iter().map(|u| u.to_string()).collect();
                    log(&format!(
                        "deep-link plugin: on_open_url x{} [{}]",
                        urls.len(),
                        urls.iter()
                            .map(|u| logging::redact_link(u))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                    let _ = app_handle.emit("lopata://deeplink", urls);
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.unminimize();
                        let _ = window.set_focus();
                    }
                });
            }

            let app_data_dir_clone = app_data_dir.clone();
            let settings_store = Arc::new(SettingsStore::open(&app_data_dir)?);
            // Cold-start deep link: when the browser launches us with a tt://
            // argument there is no running instance to forward to, and nobody
            // else reads argv — without this the link would be silently lost.
            // Kept as raw strings: tt:// payloads are not valid url::Url's.
            let startup_deeplinks: Vec<String> = std::env::args()
                .skip(1)
                .filter(|a| a.starts_with("tt://"))
                .collect();

            let state = AppState {
                profiles,
                vpn,
                app_data_dir,
                binary_path: binary_path.clone(),
                settings: settings_store,
                startup_deeplinks: Mutex::new(startup_deeplinks),
            };
            app.manage(state);

            // Build system tray
            build_tray(app.handle())?;

            // Background: check for a newer client release and download it silently.
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = auto_update_check(app_handle, app_data_dir_clone).await {
                    eprintln!("auto-update check failed: {}", e);
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::check_for_update,
            commands::cached_update_status,
            commands::install_update,
            commands::check_all_updates,
            commands::cached_app_update_status,
            commands::app_version,
            commands::open_app_release_page,
            commands::install_app_update,
            commands::get_settings,
            commands::update_settings,
            commands::list_profiles,
            commands::get_active_profile_id,
            commands::set_active_profile_id,
            commands::save_profile,
            commands::delete_profile,
            commands::new_blank_profile,
            commands::profile_to_toml,
            commands::import_toml_text,
            commands::import_toml_file,
            commands::import_deeplink,
            commands::extract_deeplink_from_text,
            commands::take_startup_deeplinks,
            commands::vpn_connect,
            commands::vpn_disconnect,
            commands::vpn_state,
            commands::vpn_logs,
            commands::service_status,
            commands::service_install,
            commands::service_uninstall,
            commands::binary_info,
            commands::open_app_data_folder,
            commands::app_data_dir,
            commands::platform_info,
            commands::is_elevated,
            commands::uninstall_app,
            commands::restart_as_admin,
            commands::register_deeplink_scheme,
            commands::show_window,
            commands::quit_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running TrustTunnel GUI");
}

/// Background update check: pulls the latest release info, emits `update://status`
/// and (if a newer version is available) downloads + installs it, then emits
/// `update://installed` so the UI can offer a switch.
async fn auto_update_check(app: AppHandle, app_data_dir: PathBuf) -> anyhow::Result<()> {
    // Kick off the GUI self-update check in parallel, independent of the client
    // check (so a network failure for one doesn't block the other).
    {
        let app_handle = app.clone();
        let state: tauri::State<AppState> = app.state();
        let settings_for_app_check = state.settings.clone();
        tauri::async_runtime::spawn(async move {
            if let Ok(app_status) = app_updater::fetch_latest().await {
                let _ = settings_for_app_check.patch(settings::Settings {
                    last_known_app_update: Some(
                        serde_json::to_value(&app_status).unwrap_or_default(),
                    ),
                    ..Default::default()
                });
                let _ = app_handle.emit("update://app-status", &app_status);
            }
        });
    }

    let result = updater::fetch_latest_release().await;
    let release = match result {
        Ok(r) => r,
        Err(e) => {
            // Surface a failure status so the UI can drop out of the "Checking…"
            // state instead of hanging forever on a network error.
            let _ = app.emit(
                "update://status",
                &updater::UpdateStatus {
                    current: None,
                    latest: None,
                    asset_name: None,
                    platform_supported: true,
                    update_available: false,
                    installed_path: None,
                    needs_initial_install: true,
                },
            );
            return Err(e);
        }
    };
    let status = updater::build_status(&app_data_dir, None, Some(&release));
    // Cache for the frontend (which may not be ready to receive the event yet)
    // and for the next launch.
    let state: tauri::State<AppState> = app.state();
    let _ = state.settings.patch(settings::Settings {
        last_known_update: Some(serde_json::to_value(&status).unwrap_or_default()),
        ..Default::default()
    });
    let _ = app.emit("update://status", &status);

    if !status.update_available || !status.platform_supported {
        return Ok(());
    }

    let app2 = app.clone();
    let progress = move |done: u64, total: Option<u64>| {
        let _ = app2.emit(
            "update://progress",
            serde_json::json!({ "done": done, "total": total }),
        );
    };
    let new_binary = updater::download_and_install(&release, &app_data_dir, progress).await?;

    // Swap in the new binary.
    let state: tauri::State<AppState> = app.state();
    *state.binary_path.lock().unwrap() = new_binary.clone();
    state.vpn.set_binary_path(new_binary.clone());

    let final_status = updater::build_status(&app_data_dir, None, Some(&release));
    let _ = app.emit("update://installed", &final_status);
    Ok(())
}

/// Pick a clean, platform-native app data directory whose folder name is just
/// `TrustTunnel` — no dotted identifier.
///
/// * Windows : `%APPDATA%\TrustTunnel`              (Roaming)
/// * macOS   : `~/Library/Application Support/TrustTunnel`
/// * Linux   : `$XDG_CONFIG_HOME/TrustTunnel` or `~/.config/TrustTunnel`
fn pick_app_data_dir() -> Option<PathBuf> {
    const APP_FOLDER: &str = "TrustTunnel";

    #[cfg(windows)]
    {
        if let Ok(roaming) = std::env::var("APPDATA") {
            return Some(PathBuf::from(roaming).join(APP_FOLDER));
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = directories::BaseDirs::new() {
            return Some(
                home.home_dir()
                    .join("Library/Application Support")
                    .join(APP_FOLDER),
            );
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(base) = directories::BaseDirs::new() {
            return Some(base.config_dir().join(APP_FOLDER));
        }
    }
    None
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn resolve_binary_path(
    resource_dir: Option<&std::path::Path>,
    app_data_dir: &std::path::Path,
) -> PathBuf {
    let bin_name = if cfg!(windows) {
        "trusttunnel_client.exe"
    } else {
        "trusttunnel_client"
    };

    // 1. resources/<binary>
    if let Some(rd) = resource_dir {
        let candidate = rd.join("resources").join(bin_name);
        if candidate.exists() {
            return candidate;
        }
        let candidate = rd.join(bin_name);
        if candidate.exists() {
            return candidate;
        }
    }
    // 2. next to the exe
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            let candidate = parent.join(bin_name);
            if candidate.exists() {
                return candidate;
            }
            let candidate = parent.join("resources").join(bin_name);
            if candidate.exists() {
                return candidate;
            }
        }
    }
    // 3. fallback: user data dir
    app_data_dir.join(bin_name)
}

struct TrayTexts {
    show: &'static str,
    connect: &'static str,
    disconnect: &'static str,
    quit: &'static str,
}

fn tray_texts(lang: &str) -> TrayTexts {
    match lang {
        "ru" => TrayTexts {
            show: "Открыть",
            connect: "Подключить",
            disconnect: "Отключить",
            quit: "Выход",
        },
        _ => TrayTexts {
            show: "Open",
            connect: "Connect",
            disconnect: "Disconnect",
            quit: "Quit",
        },
    }
}

/// Tray language: explicit user setting if present, otherwise the system
/// locale (mirrors the frontend default in i18n.js: ru-* → ru, else en).
fn tray_lang(app: &AppHandle) -> String {
    let state: tauri::State<AppState> = app.state();
    if let Some(lang) = state.settings.get().language {
        return lang;
    }
    let sys = sys_locale::get_locale().unwrap_or_default().to_lowercase();
    if sys.starts_with("ru") {
        "ru".into()
    } else {
        "en".into()
    }
}

fn build_tray_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let texts = tray_texts(&tray_lang(app));
    let show = MenuItem::with_id(app, "show", texts.show, true, None::<&str>)?;
    let connect = MenuItem::with_id(app, "connect", texts.connect, true, None::<&str>)?;
    let disconnect = MenuItem::with_id(app, "disconnect", texts.disconnect, true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", texts.quit, true, None::<&str>)?;
    Menu::with_items(app, &[&show, &connect, &disconnect, &separator, &quit])
}

/// Re-apply tray menu texts after the UI language changed.
pub fn rebuild_tray_menu(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id("main") {
        if let Ok(menu) = build_tray_menu(app) {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_tray_menu(app)?;

    let icon = app
        .default_window_icon()
        .cloned()
        .unwrap_or_else(|| Image::new_owned(vec![0u8; 4], 1, 1));

    let _tray = TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("Lopata")
        .menu(&menu)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
            "connect" => {
                let app_handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    let state: tauri::State<AppState> = app_handle.state();
                    if let Some(active) = state.profiles.active_id() {
                        if let Some(profile) = state.profiles.get(&active) {
                            let _ = state.vpn.connect(app_handle.clone(), profile).await;
                        }
                    } else if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                });
            }
            "disconnect" => {
                let app_handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    let state: tauri::State<AppState> = app_handle.state();
                    let _ = state.vpn.disconnect(app_handle.clone()).await;
                });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;
    Ok(())
}
