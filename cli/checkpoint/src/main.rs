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

fn app_checkpoint_dir(app: &str) -> PathBuf {
    checkpoint_dir().join(app)
}

fn checkpoint_path(app: &str) -> PathBuf {
    let timestamp = chrono_timestamp();
    app_checkpoint_dir(app).join(format!("{}.json", timestamp))
}

fn latest_checkpoint_path(app: &str) -> Option<PathBuf> {
    let dir = app_checkpoint_dir(app);
    if !dir.exists() {
        return None;
    }
    let mut entries: Vec<_> = fs::read_dir(&dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().extension().map(|ext| ext == "json").unwrap_or(false))
        .collect();
    entries.sort_by_key(|e| e.file_name());
    entries.last().map(|e| e.path())
}

fn chrono_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Format: YYYY-MM-DD_HH-MM-SS
    let secs_per_day = 86400;
    let secs_per_hour = 3600;
    let secs_per_min = 60;

    // Simple date calculation (not accounting for leap seconds, but good enough)
    let days_since_epoch = now / secs_per_day;
    let time_of_day = now % secs_per_day;

    let hours = time_of_day / secs_per_hour;
    let minutes = (time_of_day % secs_per_hour) / secs_per_min;
    let seconds = time_of_day % secs_per_min;

    // Calculate year/month/day from days since epoch (1970-01-01)
    let (year, month, day) = days_to_ymd(days_since_epoch);

    format!("{:04}-{:02}-{:02}_{:02}-{:02}-{:02}", year, month, day, hours, minutes, seconds)
}

fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    let mut remaining = days;
    let mut year = 1970u64;

    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        year += 1;
    }

    let days_in_months: [u64; 12] = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1u64;
    for days_in_month in days_in_months.iter() {
        if remaining < *days_in_month {
            break;
        }
        remaining -= *days_in_month;
        month += 1;
    }

    (year, month, remaining + 1)
}

fn is_leap_year(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
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

    let dir = app_checkpoint_dir(app);
    fs::create_dir_all(&dir).context("failed to create checkpoint directory")?;

    let path = checkpoint_path(app);
    let json = serde_json::to_string_pretty(&checkpoint)?;
    fs::write(&path, json)?;

    println!("Saved {} sessions for {} -> {}", checkpoint.sessions.len(), app, path.file_name().unwrap().to_string_lossy());
    for session in &checkpoint.sessions {
        println!("  {}", session.working_dir.display());
    }

    Ok(())
}

fn restore_checkpoint(app: &str) -> Result<()> {
    let path = latest_checkpoint_path(app)
        .ok_or_else(|| anyhow::anyhow!("No checkpoint found for {}", app))?;

    let data = fs::read_to_string(&path)?;
    let checkpoint: Checkpoint = serde_json::from_str(&data)?;

    match app.to_lowercase().as_str() {
        "warp" => restore_warp_sessions(&checkpoint.sessions)?,
        "zed" => restore_zed_sessions(&checkpoint.sessions)?,
        _ => anyhow::bail!("Unknown app: {}", app),
    }

    println!("Restored {} sessions for {} from {}", checkpoint.sessions.len(), app, path.file_name().unwrap().to_string_lossy());
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
        if path.is_dir() {
            let app_name = path.file_name().unwrap().to_string_lossy();
            if let Ok(files) = fs::read_dir(&path) {
                let count = files.flatten().filter(|f| {
                    f.path().extension().map(|e| e == "json").unwrap_or(false)
                }).count();
                if count > 0 {
                    found = true;
                    println!("{}: {} checkpoints", app_name, count);
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
    let dir = app_checkpoint_dir(app);
    if !dir.exists() {
        anyhow::bail!("No checkpoints found for {}", app);
    }

    let mut entries: Vec<_> = fs::read_dir(&dir)?
        .flatten()
        .filter(|e| e.path().extension().map(|ext| ext == "json").unwrap_or(false))
        .collect();
    entries.sort_by_key(|e| e.file_name());

    println!("{} checkpoints for {}:", entries.len(), app);
    for entry in entries {
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy();
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(checkpoint) = serde_json::from_str::<Checkpoint>(&data) {
                println!("  {} ({} sessions)", name, checkpoint.sessions.len());
            }
        }
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
    let home = std::env::var("HOME").unwrap_or_default();
    let mut sessions = Vec::new();
    let mut seen = HashSet::new();

    // Zed window titles are like "folder — file.ext" or just "folder"
    for name in stdout.split(", ") {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        // Extract folder name (first part before " — ")
        let folder = name.split(" — ").next().unwrap_or(name).trim();
        if folder.is_empty() {
            continue;
        }

        // Search common locations with depth
        let search_bases = [
            home.clone(),
            format!("{}/org", home),
            format!("{}/src", home),
            format!("{}/lang", home),
            format!("{}/gh", home),
            format!("{}/fork-i", home),
            format!("{}/x", home),
            format!("{}/try", home),
        ];

        'outer: for base in &search_bases {
            // Direct match
            let path = PathBuf::from(base).join(folder);
            if path.exists() && path.is_dir() {
                if seen.insert(path.clone()) {
                    sessions.push(Session { working_dir: path });
                }
                break 'outer;
            }
            // One level deep (e.g., ~/org/1f for "1f")
            if let Ok(entries) = std::fs::read_dir(base) {
                for entry in entries.flatten() {
                    let subpath = entry.path().join(folder);
                    if subpath.exists() && subpath.is_dir() {
                        if seen.insert(subpath.clone()) {
                            sessions.push(Session { working_dir: subpath });
                        }
                        break 'outer;
                    }
                }
            }
        }
    }

    sessions.sort_by(|a, b| a.working_dir.cmp(&b.working_dir));
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
