// Self-update — download latest release from GitHub.

use anyhow::{Context, Result};
use serde::Deserialize;

const REPO: &str = "Broikos-Nikos/oneclaw";
const API_BASE: &str = "https://api.github.com";

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// Get the latest release tag from GitHub.
pub async fn latest_version() -> Result<String> {
    let url = format!("{API_BASE}/repos/{REPO}/releases/latest");
    let client = reqwest::Client::new();
    let resp: Release = client
        .get(&url)
        .header("User-Agent", "oneclaw-updater")
        .send()
        .await
        .context("Failed to fetch latest release")?
        .json()
        .await
        .context("Failed to parse release JSON")?;
    Ok(resp.tag_name)
}

/// Current version from Cargo.toml.
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Check for updates without installing.
pub async fn check() -> Result<()> {
    println!("Current version : {}", current_version());
    let latest = match latest_version().await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("⚠️  Could not check for updates: {e}");
            return Ok(());
        }
    };
    println!("Latest version  : {latest}");

    if latest.trim_start_matches('v') == current_version() {
        println!("✅ You are up to date.");
    } else {
        println!("🔄 Update available! Run: oneclaw update");
        println!("   Release: https://github.com/{REPO}/releases/tag/{latest}");
    }
    Ok(())
}

/// Download and install the latest binary.
pub async fn update(force: bool) -> Result<()> {
    let latest = latest_version().await?;
    let latest_ver = latest.trim_start_matches('v');
    let current = current_version();

    if !force && latest_ver == current {
        println!("✅ Already at latest version ({current}).");
        return Ok(());
    }

    println!("🔄 Updating {current} → {latest}...");

    // Determine platform asset name
    let target = get_target();
    let client = reqwest::Client::new();
    let url = format!("{API_BASE}/repos/{REPO}/releases/latest");
    let release: Release = client
        .get(&url)
        .header("User-Agent", "oneclaw-updater")
        .send()
        .await?
        .json()
        .await?;

    let asset = release.assets.iter().find(|a| a.name.contains(&target));
    let Some(asset) = asset else {
        println!("⚠️  No pre-built binary found for {target}.");
        println!("   Build from source: cargo install --git https://github.com/{REPO}");
        println!("   Release page: {}", release.html_url);
        return Ok(());
    };

    // Download asset
    println!("⬇️  Downloading {}...", asset.name);
    let bytes = client
        .get(&asset.browser_download_url)
        .header("User-Agent", "oneclaw-updater")
        .send()
        .await
        .context("Download failed")?
        .bytes()
        .await?;

    // Write to current binary location
    let current_bin = std::env::current_exe().context("Cannot determine current binary path")?;
    let tmp = current_bin.with_extension("tmp");
    std::fs::write(&tmp, &bytes).context("Failed to write new binary")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }

    // Atomic replace
    std::fs::rename(&tmp, &current_bin).context("Failed to replace binary")?;

    println!("✅ Updated to {latest}! Restart oneclaw to use the new version.");
    Ok(())
}

fn get_target() -> String {
    let arch = if cfg!(target_arch = "x86_64") { "x86_64" }
    else if cfg!(target_arch = "aarch64") { "aarch64" }
    else { "x86_64" };

    let os = if cfg!(target_os = "linux") { "linux" }
    else if cfg!(target_os = "macos") { "darwin" }
    else if cfg!(target_os = "windows") { "windows" }
    else { "linux" };

    format!("{arch}-{os}")
}
