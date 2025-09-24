use anyhow::{anyhow, Context, Result};
use once_cell::sync::Lazy;
use reqwest::blocking as http;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Public state shared with UI for update availability and progress.
#[derive(Debug, Clone)]
pub enum UpdateState {
    /// Initial state while checking GitHub
    Checking,
    /// Up-to-date: include current and latest strings for display
    UpToDate { current: String, latest: String },
    /// Update available: include selected asset for the current OS
    Available(AvailableUpdate),
    /// Failed to check
    Error(String),
}

#[derive(Debug, Clone)]
pub struct AvailableUpdate {
    pub current: String,
    pub latest: String,
    pub asset_name: String,
    pub asset_url: String,
    pub asset_size: u64,
}

#[derive(Debug, Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    #[allow(dead_code)]
    content_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiRelease {
    tag_name: String,
    // We ignore draft/prerelease since /latest filters them out
    assets: Vec<ApiAsset>,
}

static USER_AGENT: Lazy<String> = Lazy::new(|| {
    format!(
        "HootVoice/{ver} (+https://github.com/agata/hootvoice)",
        ver = env!("CARGO_PKG_VERSION")
    )
});

fn http_client() -> Result<http::Client> {
    http::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .http1_only()
        .user_agent(USER_AGENT.clone())
        .build()
        .context("build http client")
}

/// Parse repo spec from env like "owner/repo" or a full GitHub URL.
fn parse_repo_spec<S: AsRef<str>>(s: S) -> Option<(String, String)> {
    let raw = s.as_ref().trim();
    if raw.is_empty() {
        return None;
    }
    if let Some(p) = raw.find("github.com/") {
        let after = &raw[(p + "github.com/".len())..];
        let parts: Vec<&str> = after.split('/').filter(|x| !x.is_empty()).collect();
        if parts.len() >= 2 {
            return Some((parts[0].to_string(), parts[1].to_string()));
        } else {
            return None;
        }
    }
    let parts: Vec<&str> = raw.split('/').filter(|x| !x.is_empty()).collect();
    if parts.len() == 2 {
        Some((parts[0].to_string(), parts[1].to_string()))
    } else {
        None
    }
}

fn fetch_latest_release(owner: &str, repo: &str) -> Result<ApiRelease> {
    let client = http_client()?;
    let url = format!(
        "https://api.github.com/repos/{}/{}/releases/latest",
        owner, repo
    );
    let resp = client
        .get(&url)
        .send()
        .with_context(|| format!("request latest release: {}", url))?;
    if !resp.status().is_success() {
        return Err(anyhow!("GitHub API status: {}", resp.status()));
    }
    let rel: ApiRelease = resp.json().context("parse GitHub JSON")?;
    Ok(rel)
}

/// Compare semver-ish versions. Returns true if `latest` is newer than `current`.
fn is_latest_newer(current: &str, latest: &str) -> bool {
    // Strip leading v/V and whitespace
    fn parse(v: &str) -> Option<(u64, u64, u64)> {
        let v = v.trim().trim_start_matches(&['v', 'V'][..]);
        let mut it = v.split(|c| c == '.' || c == '-');
        let major = it.next()?.parse::<u64>().ok()?;
        let minor = it.next().unwrap_or("0").parse::<u64>().unwrap_or(0);
        let patch = it.next().unwrap_or("0").parse::<u64>().unwrap_or(0);
        Some((major, minor, patch))
    }
    match (parse(current), parse(latest)) {
        (Some(c), Some(l)) => l > c,
        _ => latest.trim() != current.trim(),
    }
}

/// Choose an asset best matching the running platform.
fn pick_asset_for_platform(assets: &[ApiAsset]) -> Option<&ApiAsset> {
    let os = std::env::consts::OS; // "linux" | "macos" | "windows"
    let arch = std::env::consts::ARCH; // "x86_64" | "aarch64" | ...

    // Helper scoring function
    fn score(name: &str, os: &str, arch: &str) -> i32 {
        let mut s = 0;
        if name.to_lowercase().contains(os) {
            s += 10;
        }
        // Normalize arch tags used in artifact names
        let arch_tag = match arch {
            "x86_64" | "x86-64" | "amd64" => ["x86_64", "amd64", "x86-64"],
            "aarch64" | "arm64" => ["aarch64", "arm64", "arm64e"],
            other => [other, other, other],
        };
        if arch_tag
            .iter()
            .any(|t| name.to_lowercase().contains(&t.to_string()))
        {
            s += 5;
        }
        // Filetype preference per OS
        if os == "windows" && name.ends_with(".exe") {
            s += 3;
        }
        if os == "macos" && name.ends_with(".dmg") {
            s += 3;
        }
        if os == "linux" && name.to_lowercase().contains("appimage") {
            s += 3;
        }
        s
    }

    let mut best: Option<(&ApiAsset, i32)> = None;
    for a in assets.iter() {
        let s = score(&a.name, os, arch);
        if best.as_ref().map(|(_, bs)| s > *bs).unwrap_or(true) {
            best = Some((a, s));
        }
    }
    best.map(|(a, _)| a)
}

/// Get OS Downloads directory.
pub fn downloads_dir() -> PathBuf {
    if let Some(ud) = directories::UserDirs::new() {
        if let Some(p) = ud.download_dir() {
            return p.to_path_buf();
        }
        if let Some(home) = ud.home_dir().to_str() {
            return Path::new(home).join("Downloads");
        }
    }
    // Fallback: $HOME/Downloads
    let home = std::env::var("HOME").unwrap_or_else(|_| String::from("."));
    Path::new(&home).join("Downloads")
}

/// Spawn a background thread that checks GitHub Releases and updates `state` once.
pub fn spawn_check_update(state: Arc<Mutex<UpdateState>>, logs: Option<Arc<Mutex<Vec<String>>>>) {
    // Capture env/repo at spawn time
    let current = env!("CARGO_PKG_VERSION").to_string();
    let repo_spec = std::env::var("HOOTVOICE_UPDATE_REPO").ok();
    let repo = repo_spec
        .and_then(|s| parse_repo_spec(s))
        .unwrap_or_else(|| ("agata".to_string(), "hootvoice".to_string()));

    std::thread::spawn(move || {
        // Mark as checking
        if let Ok(mut g) = state.lock() {
            *g = UpdateState::Checking;
        }

        let result = (|| -> Result<UpdateState> {
            let owner = &repo.0;
            let name = &repo.1;
            let api_url = format!(
                "https://api.github.com/repos/{}/{}/releases/latest",
                owner, name
            );
            if let Some(ref l) = logs {
                if let Ok(mut lg) = l.lock() {
                    lg.push(format!("[Update] Checking latest release: {}", api_url));
                }
            }
            let rel = fetch_latest_release(owner, name)?;
            let latest = rel.tag_name.clone();
            if !is_latest_newer(&current, &latest) {
                if let Some(ref l) = logs {
                    if let Ok(mut lg) = l.lock() {
                        lg.push(format!(
                            "[Update] Up-to-date. current={} latest={}",
                            current, latest
                        ));
                    }
                }
                return Ok(UpdateState::UpToDate {
                    current: current.clone(),
                    latest,
                });
            }
            if let Some(a) = pick_asset_for_platform(&rel.assets) {
                if let Some(ref l) = logs {
                    if let Ok(mut lg) = l.lock() {
                        lg.push(format!(
                            "[Update] New version available: {} (asset: {} -> {})",
                            latest, a.name, a.browser_download_url
                        ));
                    }
                }
                return Ok(UpdateState::Available(AvailableUpdate {
                    current: current.clone(),
                    latest,
                    asset_name: a.name.clone(),
                    asset_url: a.browser_download_url.clone(),
                    asset_size: a.size,
                }));
            }
            // No suitable asset; still report newer exists
            if let Some(ref l) = logs {
                if let Ok(mut lg) = l.lock() {
                    lg.push(String::from(
                        "[Update] New version found but no suitable asset for this platform",
                    ));
                }
            }
            Ok(UpdateState::Error(
                "No suitable asset found for this platform".to_string(),
            ))
        })();

        let new_state = match result {
            Ok(s) => s,
            Err(e) => {
                if let Some(ref l) = logs {
                    if let Ok(mut lg) = l.lock() {
                        lg.push(format!("[Update] Update check failed: {}", e));
                    }
                }
                UpdateState::Error(format!("Update check failed: {}", e))
            }
        };
        if let Ok(mut g) = state.lock() {
            *g = new_state;
        }
    });
}

/// Build the Releases page URL for the configured repository (latest page).
pub fn releases_latest_url() -> String {
    let repo_spec = std::env::var("HOOTVOICE_UPDATE_REPO").ok();
    let (owner, name) = repo_spec
        .and_then(|s| parse_repo_spec(s))
        .unwrap_or_else(|| ("agata".to_string(), "hootvoice".to_string()));
    format!("https://github.com/{}/{}/releases/latest", owner, name)
}
