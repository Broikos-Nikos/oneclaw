// Service management — install/uninstall as OS service (systemd/launchd/NSSM).

use anyhow::{Context, Result};
use std::path::Path;

pub enum ServiceAction {
    Install,
    Uninstall,
    Status,
}

/// Install OneClaw as a background OS service.
pub fn manage(action: ServiceAction, bin_path: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    return manage_systemd(action, bin_path);

    #[cfg(target_os = "macos")]
    return manage_launchd(action, bin_path);

    #[cfg(target_os = "windows")]
    return manage_windows(action, bin_path);

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = (action, bin_path);
        anyhow::bail!("Service management not supported on this platform");
    }
}

#[cfg(target_os = "linux")]
fn manage_systemd(action: ServiceAction, bin_path: &Path) -> Result<()> {
    use std::process::Command;

    let home = std::env::var("HOME").context("$HOME not set")?;
    let unit = format!(
        r#"[Unit]
Description=OneClaw AI Assistant Daemon
After=network.target

[Service]
Type=simple
ExecStart={bin} daemon
Restart=on-failure
RestartSec=5
Environment=HOME={home}
Environment=PATH={home}/.cargo/bin:/usr/local/bin:/usr/bin:/bin

[Install]
WantedBy=default.target
"#,
        bin = bin_path.display(),
        home = home,
    );

    let service_dir = Path::new(&home).join(".config/systemd/user");
    std::fs::create_dir_all(&service_dir)?;
    let service_path = service_dir.join("oneclaw.service");

    match action {
        ServiceAction::Install => {
            std::fs::write(&service_path, unit)?;
            Command::new("systemctl").args(["--user", "daemon-reload"]).status().ok();
            Command::new("systemctl").args(["--user", "enable", "--now", "oneclaw"]).status().ok();
            // Enable lingering so the user service starts on boot even without a login session.
            Command::new("loginctl").args(["enable-linger"]).status().ok();
            println!("✅ OneClaw service installed and started (systemd user service)");
            println!("   Auto-starts on reboot (linger enabled)");
            println!("   Logs: journalctl --user -u oneclaw -f");
        }
        ServiceAction::Uninstall => {
            Command::new("systemctl").args(["--user", "disable", "--now", "oneclaw"]).status().ok();
            if service_path.exists() {
                std::fs::remove_file(&service_path)?;
            }
            Command::new("systemctl").args(["--user", "daemon-reload"]).status().ok();
            println!("✅ OneClaw service removed");
        }
        ServiceAction::Status => {
            Command::new("systemctl").args(["--user", "status", "oneclaw"]).status().ok();
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn manage_launchd(action: ServiceAction, bin_path: &Path) -> Result<()> {
    use std::process::Command;

    let home = std::env::var("HOME").context("$HOME not set")?;
    let agents_dir = Path::new(&home).join("Library/LaunchAgents");
    std::fs::create_dir_all(&agents_dir)?;
    let plist_path = agents_dir.join("ai.oneclaw.daemon.plist");

    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>ai.oneclaw.daemon</string>
  <key>ProgramArguments</key>
  <array><string>{bin}</string><string>daemon</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>{home}/.oneclaw/daemon.log</string>
  <key>StandardErrorPath</key><string>{home}/.oneclaw/daemon.err</string>
</dict>
</plist>"#,
        bin = bin_path.display(),
        home = home
    );

    match action {
        ServiceAction::Install => {
            std::fs::write(&plist_path, plist)?;
            Command::new("launchctl").args(["load", &plist_path.to_string_lossy()]).status().ok();
            println!("✅ OneClaw service installed (launchd)");
            println!("   Logs: {home}/.oneclaw/daemon.log");
        }
        ServiceAction::Uninstall => {
            Command::new("launchctl").args(["unload", &plist_path.to_string_lossy()]).status().ok();
            if plist_path.exists() { std::fs::remove_file(&plist_path)?; }
            println!("✅ OneClaw service removed");
        }
        ServiceAction::Status => {
            Command::new("launchctl").args(["list", "ai.oneclaw.daemon"]).status().ok();
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn manage_windows(action: ServiceAction, bin_path: &Path) -> Result<()> {
    // On Windows, use task scheduler or NSSM if available
    use std::process::Command;

    match action {
        ServiceAction::Install => {
            // Try to create a Scheduled Task
            let bin = bin_path.to_string_lossy();
            Command::new("schtasks")
                .args([
                    "/Create", "/TN", "OneClaw Daemon", "/SC", "ONLOGON",
                    "/TR", &format!("\"{bin}\" daemon"), "/F",
                ])
                .status()
                .context("Failed to create scheduled task")?;
            println!("✅ OneClaw scheduled task created (runs at logon)");
            println!("   Manage via: Task Scheduler → OneClaw Daemon");
        }
        ServiceAction::Uninstall => {
            Command::new("schtasks")
                .args(["/Delete", "/TN", "OneClaw Daemon", "/F"])
                .status()
                .context("Failed to delete scheduled task")?;
            println!("✅ OneClaw scheduled task removed");
        }
        ServiceAction::Status => {
            Command::new("schtasks")
                .args(["/Query", "/TN", "OneClaw Daemon"])
                .status()
                .ok();
        }
    }
    Ok(())
}
