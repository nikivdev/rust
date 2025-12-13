use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

fn main() {
    if let Err(err) = try_main() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

fn try_main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Web { name } => new_web(&name),
    }
}

#[derive(Parser)]
#[command(name = "new", version, about = "Generate new files and projects", propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new web project from gendotnew/web template.
    ///
    /// Clones the template and sets up a new project directory.
    ///
    /// Examples:
    ///   new web my-app
    ///   new web ~/projects/my-app
    Web {
        /// Project name or path.
        name: String,
    },
}

fn new_web(name: &str) -> Result<()> {
    let project_path = if name.starts_with('/') || name.starts_with('~') {
        expand_tilde(name)
    } else {
        format!("./{}", name)
    };

    let path = Path::new(&project_path);

    if path.exists() {
        bail!("directory '{}' already exists", project_path);
    }

    println!("creating web project: {}", project_path);

    // Clone the template
    let status = Command::new("git")
        .args([
            "clone",
            "--depth=1",
            "https://github.com/gendotnew/web.git",
            &project_path,
        ])
        .status()
        .context("failed to run git clone")?;

    if !status.success() {
        bail!("git clone failed");
    }

    // Remove .git directory to start fresh
    let git_dir = format!("{}/.git", project_path);
    std::fs::remove_dir_all(&git_dir).context("failed to remove .git directory")?;

    // Initialize new git repo
    let status = Command::new("git")
        .args(["init"])
        .current_dir(&project_path)
        .status()
        .context("failed to run git init")?;

    if !status.success() {
        bail!("git init failed");
    }

    println!("created: {}", project_path);
    println!("\nnext steps:");
    println!("  cd {}", project_path);
    println!("  pnpm install");
    println!("  pnpm dev");

    Ok(())
}

fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}{}", home, &path[1..]);
        }
    }
    path.to_string()
}
