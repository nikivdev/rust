use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};

fn main() {
    if let Err(err) = try_main() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

fn try_main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Save { app } => save_checkpoint(&app)?,
        Commands::Restore { app } => restore_checkpoint(&app)?,
        Commands::List => list_checkpoints()?,
        Commands::Show { app } => show_checkpoint(&app)?,
    }

    Ok(())
}

#[derive(Parser)]
#[command(name = "checkpoint", version, about = "Save and restore app sessions")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Save current sessions for an app
    Save {
        /// App name (warp, zed, etc.)
        app: String,
    },
    /// Restore saved sessions for an app
    Restore {
        /// App name (warp, zed, etc.)
        app: String,
    },
    /// List all saved checkpoints
    List,
    /// Show details of a checkpoint
    Show {
        /// App name
        app: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct Checkpoint {
    app: String,
    created_at: u64,
    sessions: Vec<Session>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Session {
    working_dir: PathBuf,
}

fn checkpoint_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".checkpoints")
}

fn checkpoint_path(app: &str) -> PathBuf {
    checkpoint_dir().join(format!("{}.json", app))
}

fn save_checkpoint(app: &str) -> Result<()> {
    let sessions = match app.to_lowercase().as_str() {
        "warp" => get_warp_sessions()?,
        "zed" => get_zed_sessions()?,
        _ => anyhow::bail!("Unknown app: {}. Supported: warp, zed", app),
    };

    if sessions.is_empty() {
        println!("No sessions found for {}", app);
        return Ok(());
    }

    let checkpoint = Checkpoint {
        app: app.to_lowercase(),
        created_at: now_epoch_sec(),
        sessions,
    };

    let dir = checkpoint_dir();
    fs::create_dir_all(&dir).context("failed to create checkpoint directory")?;

    let json = serde_json::to_string_pretty(&checkpoint)?;
    fs::write(checkpoint_path(app), json)?;

    println!("Saved {} sessions for {}", checkpoint.sessions.len(), app);
    for session in &checkpoint.sessions {
        println!("  {}", session.working_dir.display());
    }

    Ok(())
}

fn restore_checkpoint(app: &str) -> Result<()> {
    let path = checkpoint_path(app);
    if !path.exists() {
        anyhow::bail!("No checkpoint found for {}", app);
    }

    let data = fs::read_to_string(&path)?;
    let checkpoint: Checkpoint = serde_json::from_str(&data)?;

    match app.to_lowercase().as_str() {
        "warp" => restore_warp_sessions(&checkpoint.sessions)?,
        "zed" => restore_zed_sessions(&checkpoint.sessions)?,
        _ => anyhow::bail!("Unknown app: {}", app),
    }

    println!("Restored {} sessions for {}", checkpoint.sessions.len(), app);
    Ok(())
}

fn list_checkpoints() -> Result<()> {
    let dir = checkpoint_dir();
    if !dir.exists() {
        println!("No checkpoints saved yet.");
        return Ok(());
    }

    let entries = fs::read_dir(&dir)?;
    let mut found = false;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e == "json").unwrap_or(false) {
            if let Ok(data) = fs::read_to_string(&path) {
                if let Ok(checkpoint) = serde_json::from_str::<Checkpoint>(&data) {
                    found = true;
                    println!(
                        "{}: {} sessions",
                        checkpoint.app,
                        checkpoint.sessions.len()
                    );
                }
            }
        }
    }

    if !found {
        println!("No checkpoints saved yet.");
    }

    Ok(())
}

fn show_checkpoint(app: &str) -> Result<()> {
    let path = checkpoint_path(app);
    if !path.exists() {
        anyhow::bail!("No checkpoint found for {}", app);
    }

    let data = fs::read_to_string(&path)?;
    let checkpoint: Checkpoint = serde_json::from_str(&data)?;

    println!("{} ({} sessions):", checkpoint.app, checkpoint.sessions.len());
    for session in &checkpoint.sessions {
        println!("  {}", session.working_dir.display());
    }

    Ok(())
}

fn get_warp_sessions() -> Result<Vec<Session>> {
    // Get working directories from fish/zsh shell processes
    let output = Command::new("lsof")
        .args(["-c", "fish"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .context("failed to run lsof")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut dirs: HashSet<PathBuf> = HashSet::new();

    for line in stdout.lines() {
        if line.contains("cwd") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if let Some(path) = parts.last() {
                let path = PathBuf::from(path);
                if path.exists() {
                    dirs.insert(path);
                }
            }
        }
    }

    // Also check zsh
    let output = Command::new("lsof")
        .args(["-c", "zsh"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .context("failed to run lsof")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.contains("cwd") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if let Some(path) = parts.last() {
                let path = PathBuf::from(path);
                if path.exists() && !path.starts_with("/private/var") {
                    dirs.insert(path);
                }
            }
        }
    }

    let mut sessions: Vec<Session> = dirs
        .into_iter()
        .map(|working_dir| Session { working_dir })
        .collect();

    sessions.sort_by(|a, b| a.working_dir.cmp(&b.working_dir));
    Ok(sessions)
}

fn get_zed_sessions() -> Result<Vec<Session>> {
    // Get Zed window titles via AppleScript
    let script = r#"
tell application "System Events"
    tell process "Zed"
        set windowNames to name of every window
        return windowNames
    end tell
end tell
"#;

    let output = Command::new("osascript")
        .args(["-e", script])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .context("failed to run osascript")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut sessions = Vec::new();

    // Zed window titles are like "project_name — Zed" or contain path info
    for name in stdout.split(", ") {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        // Try to extract project path from window title
        // Format is usually "folder_name — Zed" or "folder_name — file.rs — Zed"
        if let Some(folder) = name.split(" — ").next() {
            let folder = folder.trim();
            // Try common locations
            for base in &[
                std::env::var("HOME").unwrap_or_default(),
                format!("{}/src", std::env::var("HOME").unwrap_or_default()),
                format!("{}/org", std::env::var("HOME").unwrap_or_default()),
                format!("{}/lang", std::env::var("HOME").unwrap_or_default()),
            ] {
                let path = PathBuf::from(base).join(folder);
                if path.exists() && path.is_dir() {
                    sessions.push(Session { working_dir: path });
                    break;
                }
            }
        }
    }

    Ok(sessions)
}

fn restore_warp_sessions(sessions: &[Session]) -> Result<()> {
    for session in sessions {
        let dir = &session.working_dir;
        if !dir.exists() {
            eprintln!("Skipping non-existent: {}", dir.display());
            continue;
        }

        // Open new Warp tab at directory
        let script = format!(
            r#"
tell application "Warp"
    activate
    delay 0.2
    tell application "System Events"
        keystroke "t" using command down
        delay 0.3
        keystroke "cd {} && clear"
        keystroke return
    end tell
end tell
"#,
            dir.display()
        );

        Command::new("osascript")
            .args(["-e", &script])
            .output()
            .context("failed to open Warp tab")?;

        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    Ok(())
}

fn restore_zed_sessions(sessions: &[Session]) -> Result<()> {
    for session in sessions {
        let dir = &session.working_dir;
        if !dir.exists() {
            eprintln!("Skipping non-existent: {}", dir.display());
            continue;
        }

        Command::new("zed")
            .arg(dir)
            .spawn()
            .context("failed to open Zed")?;

        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    Ok(())
}

fn now_epoch_sec() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
