use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

const GH_API_LATEST: &str =
    "https://api.github.com/repos/lopata29435/lopataTTC/releases/latest";
const GH_RELEASES_FALLBACK: &str =
    "https://github.com/lopata29435/lopataTTC/releases/latest";
const USER_AGENT: &str = "TrustTunnel-GUI/0.1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppUpdateStatus {
    /// Current GUI version baked at compile time (CARGO_PKG_VERSION).
    pub current: String,
    /// Latest published release tag on GitHub (e.g. "v0.1.1"), if reachable.
    pub latest: Option<String>,
    /// Direct URL of the latest release page on GitHub. Always present —
    /// falls back to /releases/latest if the API call fails.
    pub release_url: String,
    /// Publication date string from GitHub API, if reachable.
    pub published_at: Option<String>,
    /// `latest > current` (using semver comparison).
    pub update_available: bool,
}

pub fn current_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Fetch the latest GUI release from GitHub and compute update status.
pub async fn fetch_latest() -> Result<AppUpdateStatus> {
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
    let json: serde_json::Value = resp.json().await.context("parse release JSON")?;

    let latest = json
        .get("tag_name")
        .and_then(|v| v.as_str())
        .map(String::from);
    let release_url = json
        .get("html_url")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| GH_RELEASES_FALLBACK.to_string());
    let published_at = json
        .get("published_at")
        .and_then(|v| v.as_str())
        .map(String::from);

    let current = current_version();
    let update_available = match &latest {
        Some(l) => {
            let cv = semver::Version::parse(current.trim_start_matches('v')).ok();
            let lv = semver::Version::parse(l.trim_start_matches('v')).ok();
            matches!((cv, lv), (Some(a), Some(b)) if b > a)
        }
        None => false,
    };

    Ok(AppUpdateStatus {
        current,
        latest,
        release_url,
        published_at,
        update_available,
    })
}

/// Build a "no network" status — useful when the fetch fails but we still want
/// to render the local version.
pub fn fallback_status() -> AppUpdateStatus {
    AppUpdateStatus {
        current: current_version(),
        latest: None,
        release_url: GH_RELEASES_FALLBACK.to_string(),
        published_at: None,
        update_available: false,
    }
}
