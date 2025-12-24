use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use clap::Parser;

fn main() {
    if let Err(err) = try_main() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

fn try_main() -> Result<()> {
    let cli = Cli::parse();

    if cli.paths.is_empty() {
        anyhow::bail!("provide at least one path to sync");
    }

    for path in cli.paths {
        sync_path(&cli.remote, &cli.remote_base, &path, cli.delete, cli.dry_run)?;
    }

    Ok(())
}

#[derive(Parser)]
#[command(name = "sync", version, about = "Sync local paths with a remote over rsync")]
struct Cli {
    /// Remote in the form user@host
    #[arg(long)]
    remote: String,
    /// Base path on the remote (each local path syncs into remote_base/<name>)
    #[arg(long)]
    remote_base: PathBuf,
    /// Delete files on receiver to match sender
    #[arg(long)]
    delete: bool,
    /// Dry run (no changes)
    #[arg(long)]
    dry_run: bool,
    /// One or more local paths to sync
    paths: Vec<PathBuf>,
}

fn sync_path(remote: &str, remote_base: &Path, local_path: &Path, delete: bool, dry_run: bool) -> Result<()> {
    let name = local_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("invalid path: {}", local_path.display()))?;

    let remote_path = remote_base.join(name);
    let remote_spec = format!("{}:{}", remote, escape_remote_path(&remote_path));

    let local_src = format_local_src(local_path)?;
    let local_dst = format_local_dst(local_path)?;

    run_rsync(&local_src, &remote_spec, delete, dry_run)
        .with_context(|| format!("sync to remote failed for {}", local_path.display()))?;

    run_rsync(&remote_spec, &local_dst, delete, dry_run)
        .with_context(|| format!("sync from remote failed for {}", local_path.display()))?;

    Ok(())
}

fn run_rsync(src: &str, dst: &str, delete: bool, dry_run: bool) -> Result<()> {
    let mut cmd = Command::new("rsync");
    cmd.arg("-az");
    if delete {
        cmd.arg("--delete");
    }
    if dry_run {
        cmd.arg("--dry-run");
    }
    cmd.arg(src);
    cmd.arg(dst);

    let status = cmd.status().context("failed to run rsync")?;
    if !status.success() {
        anyhow::bail!("rsync failed with status {}", status);
    }
    Ok(())
}

fn format_local_src(path: &Path) -> Result<String> {
    let path_str = path.to_string_lossy();
    let meta = std::fs::metadata(path).with_context(|| format!("missing path {}", path.display()))?;
    if meta.is_dir() {
        Ok(format!("{}/", path_str.trim_end_matches('/')))
    } else {
        Ok(path_str.to_string())
    }
}

fn format_local_dst(path: &Path) -> Result<String> {
    let path_str = path.to_string_lossy();
    let _meta = std::fs::metadata(path).with_context(|| format!("missing path {}", path.display()))?;
    Ok(path_str.trim_end_matches('/').to_string())
}

fn escape_remote_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    raw.replace(' ', "\\ ")
}
