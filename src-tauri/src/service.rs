use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::elevate::run_elevated;

/// Where the persistent service config is stored. Must be readable by SYSTEM.
#[cfg(windows)]
pub fn service_config_path() -> PathBuf {
    let program_data = std::env::var("ProgramData")
        .unwrap_or_else(|_| "C:\\ProgramData".to_string());
    PathBuf::from(program_data).join("TrustTunnelGUI").join("service.toml")
}

#[cfg(not(windows))]
pub fn service_config_path() -> PathBuf {
    directories::ProjectDirs::from("org", "trusttunnel", "TrustTunnelGUI")
        .map(|p| p.config_dir().join("service.toml"))
        .unwrap_or_else(|| PathBuf::from("./service.toml"))
}

pub fn install_service(binary: &Path, toml_text: &str) -> Result<()> {
    let cfg = service_config_path();
    if let Some(parent) = cfg.parent() {
        // Try direct write first; if it fails (no permission), use elevated copy via PowerShell.
        if std::fs::create_dir_all(parent).is_err() {
            // fall back: elevated mkdir + write
        }
    }
    if std::fs::write(&cfg, toml_text).is_err() {
        // try writing via elevated PowerShell
        write_file_elevated(&cfg, toml_text)
            .context("write service config (elevated)")?;
    }
    let args = format!("--service-install --config \"{}\"", cfg.display());
    run_elevated(binary, &args, binary.parent())?;
    Ok(())
}

pub fn uninstall_service(binary: &Path) -> Result<()> {
    run_elevated(binary, "--service-uninstall", binary.parent())?;
    Ok(())
}

#[cfg(windows)]
fn write_file_elevated(path: &Path, content: &str) -> Result<()> {
    // Use PowerShell with -Command and base64-encoded payload to avoid quoting issues.
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    let payload_b64 = STANDARD.encode(content.as_bytes());
    let target = path.display().to_string();
    let dir = path.parent().map(|p| p.display().to_string()).unwrap_or_default();
    let script = format!(
        "$dir = '{dir}'; if ($dir -and -not (Test-Path $dir)) {{ New-Item -ItemType Directory -Force -Path $dir | Out-Null }}; [IO.File]::WriteAllBytes('{target}', [Convert]::FromBase64String('{b64}'))",
        dir = dir.replace('\'', "''"),
        target = target.replace('\'', "''"),
        b64 = payload_b64,
    );
    let ps_exe = PathBuf::from("powershell.exe");
    let args = format!("-NoProfile -ExecutionPolicy Bypass -Command \"{}\"", script.replace('"', "\\\""));
    run_elevated(&ps_exe, &args, None)?;
    Ok(())
}

#[cfg(not(windows))]
fn write_file_elevated(_path: &Path, _content: &str) -> Result<()> {
    Err(anyhow::anyhow!("elevated write not implemented on non-Windows"))
}

#[derive(serde::Serialize, Clone, Debug)]
pub struct ServiceStatus {
    pub installed: bool,
    pub running: bool,
    pub raw: String,
}

#[cfg(windows)]
pub fn query_service() -> ServiceStatus {
    use std::process::Command;
    // The exact service name is set by the binary itself. Try the most likely names.
    let names = ["TrustTunnelClient", "trusttunnel_client", "TrustTunnel"];
    let mut last_raw = String::new();
    for name in names {
        let out = Command::new("sc").arg("query").arg(name).output();
        if let Ok(o) = out {
            let txt = String::from_utf8_lossy(&o.stdout).into_owned();
            last_raw = txt.clone();
            if !txt.is_empty() && !txt.to_lowercase().contains("does not exist") && o.status.success() {
                let running = txt.to_uppercase().contains("RUNNING");
                return ServiceStatus { installed: true, running, raw: txt };
            }
        }
    }
    ServiceStatus { installed: false, running: false, raw: last_raw }
}

#[cfg(not(windows))]
pub fn query_service() -> ServiceStatus {
    ServiceStatus { installed: false, running: false, raw: "service mode is Windows-only".into() }
}
