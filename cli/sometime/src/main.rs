use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{Local, NaiveDate, NaiveDateTime};
use clap::Parser;
use serde::{Deserialize, Serialize};

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Daemon) => run_daemon(),
        Some(Commands::List) => list_tasks(),
        Some(Commands::Add { name, date, command }) => add_task(&name, &date, &command),
        Some(Commands::Remove { name }) => remove_task(&name),
        None => run_daemon(),
    }
}

#[derive(Parser)]
#[command(name = "sometime", version, about = "Run tasks at scheduled times")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Run as daemon (default)
    Daemon,
    /// List scheduled tasks
    List,
    /// Add a new task
    Add {
        /// Task name
        name: String,
        /// Date/time (YYYY-MM-DD or YYYY-MM-DD HH:MM)
        date: String,
        /// Command to execute
        command: String,
    },
    /// Remove a task by name
    Remove {
        /// Task name to remove
        name: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Config {
    #[serde(default)]
    task: Vec<Task>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Task {
    name: String,
    date: String,
    command: String,
}

impl Task {
    fn datetime(&self) -> Option<NaiveDateTime> {
        // Try full datetime first: YYYY-MM-DD HH:MM
        if let Ok(dt) = NaiveDateTime::parse_from_str(&self.date, "%Y-%m-%d %H:%M") {
            return Some(dt);
        }
        // Try date only: YYYY-MM-DD (runs at midnight)
        if let Ok(d) = NaiveDate::parse_from_str(&self.date, "%Y-%m-%d") {
            return Some(d.and_hms_opt(0, 0, 0)?);
        }
        None
    }

    fn seconds_until(&self) -> Option<i64> {
        let target = self.datetime()?;
        let now = Local::now().naive_local();
        Some((target - now).num_seconds())
    }
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("sometime.toml")
}

fn load_config() -> Result<Config> {
    let path = config_path();
    if !path.exists() {
        return Ok(Config { task: vec![] });
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))
}

fn save_config(config: &Config) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(config)?;
    fs::write(&path, content)?;
    Ok(())
}

fn run_daemon() -> Result<()> {
    eprintln!("sometime: starting daemon");
    eprintln!("config: {}", config_path().display());

    loop {
        let config = load_config()?;

        if config.task.is_empty() {
            // No tasks, sleep for a while then check again
            thread::sleep(Duration::from_secs(3600));
            continue;
        }

        // Find the next task to run
        let mut next_task: Option<(usize, i64)> = None;

        for (i, task) in config.task.iter().enumerate() {
            if let Some(secs) = task.seconds_until() {
                if secs <= 0 {
                    // Task is due, run it now
                    run_task(task);

                    // Remove from config
                    let mut updated = config.clone();
                    updated.task.remove(i);
                    save_config(&updated)?;

                    // Restart loop to find next task
                    continue;
                }

                match next_task {
                    None => next_task = Some((i, secs)),
                    Some((_, next_secs)) if secs < next_secs => {
                        next_task = Some((i, secs));
                    }
                    _ => {}
                }
            }
        }

        match next_task {
            Some((i, secs)) => {
                let task = &config.task[i];
                eprintln!(
                    "next: {} in {} ({})",
                    task.name,
                    format_duration(secs),
                    task.date
                );

                // Sleep until task is due (wake up 1 second early to be safe)
                let sleep_secs = (secs - 1).max(1) as u64;
                thread::sleep(Duration::from_secs(sleep_secs));
            }
            None => {
                // All tasks have invalid dates, wait and retry
                eprintln!("no valid tasks, sleeping 1h");
                thread::sleep(Duration::from_secs(3600));
            }
        }
    }
}

fn run_task(task: &Task) {
    eprintln!("running: {} -> {}", task.name, task.command);

    let result = Command::new("sh")
        .args(["-c", &task.command])
        .status();

    match result {
        Ok(status) => {
            if status.success() {
                eprintln!("done: {}", task.name);
            } else {
                eprintln!("failed: {} (exit {})", task.name, status);
            }
        }
        Err(e) => {
            eprintln!("error: {} - {}", task.name, e);
        }
    }
}

fn format_duration(secs: i64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else {
        format!("{}d {}h", secs / 86400, (secs % 86400) / 3600)
    }
}

fn list_tasks() -> Result<()> {
    let config = load_config()?;

    if config.task.is_empty() {
        println!("no tasks scheduled");
        return Ok(());
    }

    for task in &config.task {
        let remaining = task.seconds_until()
            .map(|s| if s > 0 { format_duration(s) } else { "due".into() })
            .unwrap_or_else(|| "invalid date".into());

        println!("{}: {} [{}] -> {}", task.name, task.date, remaining, task.command);
    }

    Ok(())
}

fn add_task(name: &str, date: &str, command: &str) -> Result<()> {
    let mut config = load_config()?;

    // Validate date format
    let task = Task {
        name: name.to_string(),
        date: date.to_string(),
        command: command.to_string(),
    };

    if task.datetime().is_none() {
        anyhow::bail!("invalid date format: use YYYY-MM-DD or YYYY-MM-DD HH:MM");
    }

    config.task.push(task);
    save_config(&config)?;

    println!("added: {}", name);
    Ok(())
}

fn remove_task(name: &str) -> Result<()> {
    let mut config = load_config()?;
    let before = config.task.len();
    config.task.retain(|t| t.name != name);

    if config.task.len() == before {
        anyhow::bail!("task not found: {}", name);
    }

    save_config(&config)?;
    println!("removed: {}", name);
    Ok(())
}
