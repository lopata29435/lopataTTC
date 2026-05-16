use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const GH_API_LATEST: &str =
    "https://api.github.com/repos/TrustTunnel/TrustTunnelClient/releases/latest";
const USER_AGENT: &str = "TrustTunnel-GUI/0.1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub name: Option<String>,
    pub published_at: Option<String>,
    pub html_url: Option<String>,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateStatus {
    pub current: Option<String>,
    pub latest: Option<String>,
    pub asset_name: Option<String>,
    pub platform_supported: bool,
    pub update_available: bool,
    pub installed_path: Option<String>,
    /// True when no client binary is installed yet — i.e. first launch.
    /// UI should show a setup overlay and refuse Connect until installed.
    pub needs_initial_install: bool,
}

pub fn clients_root(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("clients")
}

/// What platform/arch asset name should we look for in the release.
pub fn asset_name_for_current_platform(tag: &str) -> Option<String> {
    let tag = tag.trim_start_matches('v');
    let tag = format!("v{}", tag);
    if cfg!(target_os = "macos") {
        return Some(format!("trusttunnel_client-{}-macos-universal.tar.gz", tag));
    }
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "x86" => "i686",
        "aarch64" => "aarch64",
        "arm" => "armv7",
        _ => return None,
    };
    if cfg!(target_os = "windows") {
        Some(format!("trusttunnel_client-{}-windows-{}.zip", tag, arch))
    } else if cfg!(target_os = "linux") {
        Some(format!("trusttunnel_client-{}-linux-{}.tar.gz", tag, arch))
    } else {
        None
    }
}

pub fn binary_file_name() -> &'static str {
    if cfg!(windows) {
        "trusttunnel_client.exe"
    } else {
        "trusttunnel_client"
    }
}

/// Look in app_data_dir/clients/v<version>/ for the newest installed version.
/// Returns (version_string, binary_path).
pub fn locate_installed_client(app_data_dir: &Path) -> Option<(String, PathBuf)> {
    let root = clients_root(app_data_dir);
    if !root.exists() {
        return None;
    }
    let mut candidates: Vec<(semver::Version, PathBuf, String)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let trimmed = name.trim_start_matches('v');
            if let Ok(ver) = semver::Version::parse(trimmed) {
                let bin = entry.path().join(binary_file_name());
                if bin.exists() {
                    candidates.push((ver, bin, name));
                }
            }
        }
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    candidates
        .into_iter()
        .next()
        .map(|(_, bin, name)| (name, bin))
}

pub async fn fetch_latest_release() -> Result<ReleaseInfo> {
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let resp = client
        .get(GH_API_LATEST)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("GitHub API request failed")?;
    if !resp.status().is_success() {
        bail!("GitHub API status {}", resp.status());
    }
    let info: ReleaseInfo = resp.json().await.context("parse release JSON")?;
    Ok(info)
}

/// Compute current vs latest status without performing any download.
///
/// `bundled_version` is the optional fallback to report as `current` when the
/// installer carries a built-in copy. In the no-bundle world we pass `None`,
/// and a missing install yields `current = None` plus `needs_initial_install`.
pub fn build_status(
    app_data_dir: &Path,
    bundled_version: Option<&str>,
    latest: Option<&ReleaseInfo>,
) -> UpdateStatus {
    let installed = locate_installed_client(app_data_dir);
    let needs_initial_install = installed.is_none() && bundled_version.is_none();

    let current = installed
        .as_ref()
        .map(|(v, _)| v.clone())
        .or_else(|| bundled_version.map(|v| format!("v{}", v.trim_start_matches('v'))));

    let installed_path = installed.as_ref().map(|(_, p)| p.display().to_string());

    let asset_name = latest.and_then(|r| asset_name_for_current_platform(&r.tag_name));
    let platform_supported = asset_name.is_some();
    let latest_tag = latest.map(|r| r.tag_name.clone());

    let update_available = match (&current, &latest_tag) {
        (Some(c), Some(l)) => {
            let cv = semver::Version::parse(c.trim_start_matches('v')).ok();
            let lv = semver::Version::parse(l.trim_start_matches('v')).ok();
            match (cv, lv) {
                (Some(a), Some(b)) => b > a,
                _ => false,
            }
        }
        // No `current` but we have a `latest` → there's something to install.
        (None, Some(_)) => true,
        _ => false,
    };

    UpdateStatus {
        current,
        latest: latest_tag,
        asset_name,
        platform_supported,
        update_available,
        installed_path,
        needs_initial_install,
    }
}

/// Download the asset matching the current platform from the given release and extract it
/// to `app_data_dir/clients/v<version>/`. Returns the new binary path.
pub async fn download_and_install(
    release: &ReleaseInfo,
    app_data_dir: &Path,
    progress: impl Fn(u64, Option<u64>) + Send + Sync + 'static,
) -> Result<PathBuf> {
    let asset_name = asset_name_for_current_platform(&release.tag_name)
        .ok_or_else(|| anyhow!("Платформа не поддерживается (нет подходящего ассета)"))?;
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .ok_or_else(|| anyhow!("В релизе {} нет файла {}", release.tag_name, asset_name))?;

    let dest_dir = clients_root(app_data_dir).join(&release.tag_name);
    if dest_dir.exists() {
        std::fs::remove_dir_all(&dest_dir).ok();
    }
    std::fs::create_dir_all(&dest_dir).with_context(|| format!("create {}", dest_dir.display()))?;

    let temp_path = dest_dir.join(format!("download.{}", extension_of(&asset.name)));

    // Stream download to temp file so we can report progress.
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(120))
        .build()?;
    let resp = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .context("download request failed")?;
    if !resp.status().is_success() {
        bail!("download HTTP {}", resp.status());
    }
    let total = resp.content_length();
    let mut file = std::fs::File::create(&temp_path)
        .with_context(|| format!("create temp {}", temp_path.display()))?;
    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("download stream error")?;
        use std::io::Write;
        file.write_all(&chunk).context("write temp")?;
        downloaded += chunk.len() as u64;
        progress(downloaded, total);
    }
    drop(file);

    // Extract.
    extract_archive(&temp_path, &dest_dir).with_context(|| format!("extract {}", asset.name))?;
    let _ = std::fs::remove_file(&temp_path);

    // Find the binary in the extracted tree (may be in a subdirectory).
    let binary = find_binary_recursive(&dest_dir).ok_or_else(|| {
        anyhow!(
            "после распаковки бинарник не найден в {}",
            dest_dir.display()
        )
    })?;

    // Ensure executable bit on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&binary) {
            let mut perms = meta.permissions();
            perms.set_mode(perms.mode() | 0o111);
            let _ = std::fs::set_permissions(&binary, perms);
        }
    }

    // Normalize: ensure the binary lives at dest_dir/<binary_file_name>().
    let canonical = dest_dir.join(binary_file_name());
    if binary != canonical {
        // Copy (rather than move) any auxiliary files alongside the binary.
        if let Some(parent) = binary.parent() {
            if let Ok(entries) = std::fs::read_dir(parent) {
                for entry in entries.flatten() {
                    let from = entry.path();
                    let to = dest_dir.join(entry.file_name());
                    if from != to && !to.exists() {
                        let _ = std::fs::copy(&from, &to);
                    }
                }
            }
        }
    }

    let final_path = if canonical.exists() {
        canonical
    } else {
        binary
    };
    Ok(final_path)
}

fn extension_of(name: &str) -> &'static str {
    if name.ends_with(".tar.gz") {
        "tar.gz"
    } else if name.ends_with(".zip") {
        "zip"
    } else if name.ends_with(".tar") {
        "tar"
    } else {
        "bin"
    }
}

fn extract_archive(archive: &Path, dest: &Path) -> Result<()> {
    let name = archive.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if name.ends_with(".zip") {
        let file = std::fs::File::open(archive)?;
        let mut zip = zip::ZipArchive::new(file)?;
        zip.extract(dest)?;
        Ok(())
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        let file = std::fs::File::open(archive)?;
        let gz = flate2::read::GzDecoder::new(file);
        let mut tar = tar::Archive::new(gz);
        tar.unpack(dest)?;
        Ok(())
    } else if name.ends_with(".tar") {
        let file = std::fs::File::open(archive)?;
        let mut tar = tar::Archive::new(file);
        tar.unpack(dest)?;
        Ok(())
    } else {
        bail!("неизвестный формат архива: {}", name)
    }
}

fn find_binary_recursive(dir: &Path) -> Option<PathBuf> {
    let target = binary_file_name();
    // BFS up to a few levels.
    let mut queue: Vec<PathBuf> = vec![dir.to_path_buf()];
    while let Some(d) = queue.pop() {
        if let Ok(entries) = std::fs::read_dir(&d) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    queue.push(p);
                } else if p.file_name().and_then(|s| s.to_str()) == Some(target) {
                    return Some(p);
                }
            }
        }
    }
    None
}
