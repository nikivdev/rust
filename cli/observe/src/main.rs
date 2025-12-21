mod watcher;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};

use watcher::FileWatcher;

#[derive(Parser)]
#[command(name = "observe", version, about = "System observation daemon for Lin")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the observation daemon
    Start,
    /// Show current observations
    Status,
    /// Show recent activity
    Recent {
        /// Number of entries to show
        #[arg(short, default_value = "20")]
        n: usize,
    },
}

/// Observation entry stored to disk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub timestamp: DateTime<Utc>,
    pub kind: ObservationKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ObservationKind {
    /// File was modified
    FileModified { path: String, project: Option<String> },
    /// Git commit was made
    GitCommit { project: String, message: String, hash: String },
    /// Project was accessed (directory opened)
    ProjectAccess { path: String, name: String },
    /// Application became active
    AppFocus { app: String, window_title: Option<String> },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Start => run_daemon(),
        Commands::Status => show_status(),
        Commands::Recent { n } => show_recent(n),
    }
}

fn data_dir() -> Result<PathBuf> {
    let dir = dirs::data_dir()
        .context("get data dir")?
        .join("observe");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn observations_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("observations.jsonl"))
}

fn append_observation(obs: &Observation) -> Result<()> {
    use std::io::Write;
    let path = observations_path()?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(file, "{}", serde_json::to_string(obs)?)?;
    Ok(())
}

fn load_observations(limit: usize) -> Result<Vec<Observation>> {
    let path = observations_path()?;
    if !path.exists() {
        return Ok(vec![]);
    }

    let content = std::fs::read_to_string(&path)?;
    let mut observations: Vec<Observation> = content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();

    // Return last N
    if observations.len() > limit {
        observations = observations.split_off(observations.len() - limit);
    }
    Ok(observations)
}

fn run_daemon() -> Result<()> {
    eprintln!("Starting observe daemon...");

    // Watch key directories
    let home = dirs::home_dir().context("get home dir")?;
    let watch_dirs = vec![
        home.join("org"),
        home.join("lang"),
        home.join("flow"),
        home.join("fork-i"),
        home.join("config"),
    ];

    let mut watcher = FileWatcher::new()?;
    for dir in &watch_dirs {
        if dir.exists() {
            watcher.watch(dir)?;
            eprintln!("Watching: {}", dir.display());
        }
    }

    eprintln!("Daemon running...");

    // Run forever - Lin will kill us when needed
    loop {
        // Process file events
        for event in watcher.poll() {
            let project = extract_project(&event.path);
            let obs = Observation {
                timestamp: Utc::now(),
                kind: ObservationKind::FileModified {
                    path: event.path.clone(),
                    project,
                },
            };
            if let Err(e) = append_observation(&obs) {
                eprintln!("Error logging observation: {}", e);
            }
        }

        std::thread::sleep(Duration::from_millis(100));
    }
}

fn extract_project(path: &str) -> Option<String> {
    let home = dirs::home_dir()?;
    let rel = path.strip_prefix(home.to_str()?)?;

    // Extract project name from path like /org/linsa/lin or /lang/rust
    let parts: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() >= 2 {
        match parts[0] {
            "org" if parts.len() >= 3 => Some(parts[2].to_string()),
            "lang" => Some(parts[1].to_string()),
            "flow" => Some("flow".to_string()),
            "fork-i" if parts.len() >= 3 => Some(parts[2].to_string()),
            _ => Some(parts[1].to_string()),
        }
    } else {
        None
    }
}

fn show_status() -> Result<()> {
    let path = observations_path()?;
    if !path.exists() {
        println!("No observations yet. Run `observe start` to begin.");
        return Ok(());
    }

    let metadata = std::fs::metadata(&path)?;
    let size = metadata.len();
    let observations = load_observations(1000)?;

    println!("Observations file: {}", path.display());
    println!("File size: {} KB", size / 1024);
    println!("Total observations: {}", observations.len());

    // Count by type
    let mut file_mods = 0;
    let mut projects: std::collections::HashSet<String> = std::collections::HashSet::new();

    for obs in &observations {
        match &obs.kind {
            ObservationKind::FileModified { project, .. } => {
                file_mods += 1;
                if let Some(p) = project {
                    projects.insert(p.clone());
                }
            }
            _ => {}
        }
    }

    println!("File modifications: {}", file_mods);
    println!("Projects touched: {}", projects.len());

    Ok(())
}

fn show_recent(n: usize) -> Result<()> {
    let observations = load_observations(n)?;

    if observations.is_empty() {
        println!("No observations yet.");
        return Ok(());
    }

    for obs in observations {
        let time = obs.timestamp.format("%H:%M:%S");
        match obs.kind {
            ObservationKind::FileModified { path, project } => {
                let proj = project.unwrap_or_else(|| "?".to_string());
                // Show just filename
                let filename = path.rsplit('/').next().unwrap_or(&path);
                println!("[{}] {} - {}", time, proj, filename);
            }
            ObservationKind::GitCommit { project, message, .. } => {
                println!("[{}] {} - commit: {}", time, project, message);
            }
            ObservationKind::ProjectAccess { name, .. } => {
                println!("[{}] opened: {}", time, name);
            }
            ObservationKind::AppFocus { app, window_title } => {
                let title = window_title.unwrap_or_default();
                println!("[{}] focus: {} - {}", time, app, title);
            }
        }
    }

    Ok(())
}
