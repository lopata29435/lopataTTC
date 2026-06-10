//! Self-uninstall for platforms without a standard uninstall flow
//! (AppImage / .dmg drag-install / manually copied binaries). Windows users
//! go through "Apps & features", so this module is Unix-only.

#![cfg(unix)]

use anyhow::{anyhow, Result};
use std::path::Path;

/// Remove the application itself, then the user data directory.
///
/// The binary/bundle removal is scheduled in a detached shell with a short
/// delay so it happens after the process exits; the caller is expected to
/// call `app.exit(0)` right after this returns Ok.
pub fn uninstall(app_data_dir: &Path) -> Result<()> {
    let exe = std::env::current_exe()?;

    #[cfg(target_os = "macos")]
    remove_macos_bundle(&exe)?;

    #[cfg(target_os = "linux")]
    remove_linux_install(&exe)?;

    // User data: profiles, settings, downloaded clients, logs. Everything in
    // there is user-owned (the elevated client only appends to a user-created
    // log file), so a plain recursive delete works.
    let _ = std::fs::remove_dir_all(app_data_dir);

    #[cfg(target_os = "linux")]
    cleanup_desktop_entries();

    Ok(())
}

/// `sh -c <cmd>` fully detached from our (soon to exit) process.
fn detached_sh(cmd: &str) -> Result<()> {
    use std::process::{Command, Stdio};
    // Prefer setsid so the child survives our exit even on strict init
    // systems; fall back to a plain shell if util-linux isn't there.
    let with_setsid = Command::new("setsid")
        .arg("-f")
        .arg("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if with_setsid.is_ok() {
        return Ok(());
    }
    Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| anyhow!("не удалось запустить shell для удаления: {}", e))
}

fn shquote(p: &Path) -> String {
    format!("'{}'", p.display().to_string().replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
fn remove_macos_bundle(exe: &Path) -> Result<()> {
    let bundle = exe
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .ok_or_else(|| {
            anyhow!("Не найден .app-бандл (приложение запущено не из бандла) — удалите его вручную")
        })?;

    let parent_writable = bundle
        .parent()
        .map(|p| {
            // crude writability probe: metadata readonly flag is not enough on
            // macOS, so just try to check access via faccessat semantics
            std::fs::metadata(p)
                .map(|m| !m.permissions().readonly())
                .unwrap_or(false)
        })
        .unwrap_or(false);

    let rm = format!("sleep 1; rm -rf {}", shquote(bundle));
    if parent_writable {
        detached_sh(&rm)
    } else {
        // /Applications not writable by this user → native admin prompt.
        let escaped = rm.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "do shell script \"{}\" with administrator privileges",
            escaped
        );
        let quoted = script.replace('\'', "'\\''");
        detached_sh(&format!("osascript -e '{}'", quoted))
    }
}

#[cfg(target_os = "linux")]
fn remove_linux_install(exe: &Path) -> Result<()> {
    // 1. AppImage: the runtime exports the original file path.
    if let Ok(appimage) = std::env::var("APPIMAGE") {
        if !appimage.is_empty() {
            return detached_sh(&format!("sleep 1; rm -f {}", shquote(Path::new(&appimage))));
        }
    }

    let exe_str = exe.display().to_string();

    // 2. deb-managed? `dpkg -S` prints "package: /path".
    if let Some(out) = run_capture("dpkg", &["-S", &exe_str]) {
        if let Some(pkg) = out.split(':').next().map(str::trim) {
            if !pkg.is_empty() && !pkg.contains(' ') {
                return detached_sh(&format!(
                    "sleep 1; pkexec sh -c 'apt-get remove -y {pkg} || dpkg -r {pkg}'",
                    pkg = pkg
                ));
            }
        }
    }

    // 3. rpm-managed?
    if let Some(out) = run_capture("rpm", &["-qf", "--queryformat", "%{NAME}", &exe_str]) {
        let pkg = out.trim();
        if !pkg.is_empty() && !pkg.contains("not owned") && !pkg.contains(' ') {
            return detached_sh(&format!("sleep 1; pkexec rpm -e {}", pkg));
        }
    }

    // 4. plain binary (manual copy / dev build) — just delete the file.
    detached_sh(&format!("sleep 1; rm -f {}", shquote(exe)))
}

#[cfg(target_os = "linux")]
fn run_capture(bin: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(bin).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Remove desktop entries the app may have registered (tt:// deep-link
/// handler). Best effort: failures here must not abort the uninstall.
#[cfg(target_os = "linux")]
fn cleanup_desktop_entries() {
    let Some(base) = directories::BaseDirs::new() else {
        return;
    };
    let apps = base.data_dir().join("applications");
    if let Ok(entries) = std::fs::read_dir(&apps) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name.contains("trusttunnel") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let _ = std::process::Command::new("update-desktop-database")
        .arg(apps)
        .output();
}
