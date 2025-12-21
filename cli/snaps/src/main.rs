use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::Serialize;

fn main() {
    if let Err(err) = try_main() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

fn try_main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Commands::Zed { out: None, no_quit: false }) {
        Commands::Zed { out, no_quit } => snapshot_zed(out, no_quit),
    }
}

#[derive(Parser)]
#[command(name = "snaps", version, about = "Snapshot apps and close them", propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Snapshot Zed windows/workspaces and quit the app
    Zed {
        /// Output path for snapshot JSON
        #[arg(long)]
        out: Option<PathBuf>,
        /// Do not quit the app after snapshot
        #[arg(long)]
        no_quit: bool,
    },
}

#[derive(Serialize)]
struct Snapshot {
    app: String,
    timestamp_epoch_sec: u64,
    windows: Vec<String>,
}

fn snapshot_zed(out: Option<PathBuf>, no_quit: bool) -> Result<()> {
    let windows = get_zed_windows().unwrap_or_default();

    let snapshot = Snapshot {
        app: "Zed".to_string(),
        timestamp_epoch_sec: now_epoch_sec(),
        windows,
    };

    let out = out.unwrap_or_else(|| default_output_path("zed"));
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).context("failed to create snapshot directory")?;
    }

    let json = serde_json::to_string_pretty(&snapshot).context("failed to serialize snapshot")?;
    fs::write(&out, json).context("failed to write snapshot")?;

    if !no_quit {
        quit_app("Zed")?;
    }

    println!("{}", out.display());
    Ok(())
}

fn get_zed_windows() -> Result<Vec<String>> {
    let script = r#"
tell application "System Events"
    if not (exists process "Zed") then
        return ""
    end if
    tell process "Zed"
        set windowNames to name of windows
    end tell
end tell
set text item delimiters to linefeed
return windowNames as text
"#;

    let output = run_osascript(script)?;
    let output = output.trim();
    if output.is_empty() {
        return Ok(Vec::new());
    }

    Ok(output
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect())
}

fn quit_app(app: &str) -> Result<()> {
    let script = format!(r#"tell application "{}" to quit"#, app);
    run_osascript(&script)?;
    Ok(())
}

fn run_osascript(script: &str) -> Result<String> {
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .context("failed to run osascript")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("osascript failed: {}", stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn default_output_path(app: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dir = PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("snaps");

    let ts = now_epoch_sec();
    dir.join(format!("{}-{}.json", app, ts))
}

fn now_epoch_sec() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
