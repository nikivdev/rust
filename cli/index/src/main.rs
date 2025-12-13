use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use git2::{Repository, Status};
use rayon::prelude::*;
use serde::Serialize;
use walkdir::WalkDir;

fn main() {
    if let Err(err) = try_main() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

fn try_main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Repos(args) => run_index_repos(args),
    }
}

#[derive(Parser)]
#[command(
    name = "index",
    version,
    about = "Repository indexing CLI",
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build a JSON index for Git repositories under a directory.
    Repos(IndexReposArgs),
}

#[derive(Args)]
struct IndexReposArgs {
    /// Directory that contains Git repositories (defaults to current directory).
    #[arg(long = "root", default_value = ".", value_name = "DIR")]
    root: PathBuf,
    /// Path to the JSON file where the index will be written.
    #[arg(
        long = "output",
        default_value = "repo-index.json",
        value_name = "FILE"
    )]
    output: PathBuf,
    /// Limit how deep the scan should traverse (counts repository root levels).
    #[arg(long = "max-depth", value_name = "LEVELS")]
    max_depth: Option<usize>,
    /// Number of worker threads to use when collecting repository metadata.
    #[arg(long = "jobs", value_name = "COUNT")]
    jobs: Option<usize>,
    /// Suppress human readable progress output.
    #[arg(long = "quiet")]
    quiet: bool,
}

fn run_index_repos(args: IndexReposArgs) -> Result<()> {
    if let Some(depth) = args.max_depth {
        if depth == 0 {
            bail!("max-depth must be greater than zero");
        }
    }

    if let Some(jobs) = args.jobs {
        if jobs == 0 {
            bail!("jobs must be greater than zero");
        }
    }

    if !args.root.exists() {
        bail!("{} does not exist", args.root.display());
    }

    let root = args
        .root
        .canonicalize()
        .with_context(|| format!("Unable to resolve root {}", args.root.display()))?;
    if !root.is_dir() {
        bail!("{} is not a directory", root.display());
    }

    let total_started = Instant::now();
    let discovery_started = Instant::now();
    let repo_paths = discover_git_repositories(&root, args.max_depth)?;
    let discovery_elapsed = discovery_started.elapsed();

    if repo_paths.is_empty() {
        if !args.quiet {
            println!("No Git repositories were found under {}", root.display());
        }
        return Ok(());
    }

    let metadata_started = Instant::now();
    let (repositories, failures) = gather_repo_records(&repo_paths, args.jobs)?;
    let metadata_elapsed = metadata_started.elapsed();
    let total_elapsed = total_started.elapsed();

    if let Some(parent) = args.output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Unable to create {}", parent.display()))?;
        }
    }

    let index = RepoIndex {
        root: root.clone(),
        generated_at: unix_timestamp()?,
        total_ms: total_elapsed.as_millis() as u64,
        discovery_ms: discovery_elapsed.as_millis() as u64,
        metadata_ms: metadata_elapsed.as_millis() as u64,
        repositories,
        failures,
    };

    write_index(&index, &args.output)?;

    if !args.quiet {
        println!(
            "Indexed {} repositories ({} skipped) under {} in {:.2?}",
            index.repositories.len(),
            index.failures.len(),
            root.display(),
            total_elapsed
        );
        println!("Wrote index to {}", args.output.display());
    }

    Ok(())
}

fn discover_git_repositories(root: &Path, max_depth: Option<usize>) -> Result<Vec<PathBuf>> {
    let mut builder = WalkDir::new(root);
    builder = builder.follow_links(false);
    if let Some(depth) = max_depth {
        builder = builder.max_depth(depth.saturating_add(1));
    }

    let mut iter = builder.into_iter();
    let mut repos = HashSet::new();

    while let Some(entry) = iter.next() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                eprintln!("Skipping entry: {err}");
                continue;
            }
        };

        if entry.file_name() == OsStr::new(".git") {
            if let Some(parent) = entry.path().parent() {
                let resolved = parent
                    .canonicalize()
                    .unwrap_or_else(|_| parent.to_path_buf());
                repos.insert(resolved);
            }

            if entry.file_type().is_dir() {
                iter.skip_current_dir();
            }
        }
    }

    let mut repos: Vec<_> = repos.into_iter().collect();
    repos.sort();
    Ok(repos)
}

fn gather_repo_records(
    paths: &[PathBuf],
    jobs: Option<usize>,
) -> Result<(Vec<RepoEntry>, Vec<RepoFailure>)> {
    let results = if let Some(threads) = jobs {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .context("Unable to configure worker pool")?;
        pool.install(|| collect_repo_info(paths))
    } else {
        collect_repo_info(paths)
    };

    let mut entries = Vec::new();
    let mut failures = Vec::new();

    for (path, result) in results {
        match result {
            Ok(record) => entries.push(record),
            Err(err) => failures.push(RepoFailure {
                path,
                reason: format!("{err:#}"),
            }),
        }
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));
    failures.sort_by(|a, b| a.path.cmp(&b.path));

    Ok((entries, failures))
}

fn collect_repo_info(paths: &[PathBuf]) -> Vec<(PathBuf, Result<RepoEntry>)> {
    paths
        .par_iter()
        .map(|path| {
            let info = inspect_repository(path);
            (path.clone(), info)
        })
        .collect()
}

fn inspect_repository(path: &Path) -> Result<RepoEntry> {
    let repo = Repository::open(path)
        .with_context(|| format!("Unable to open repository at {}", path.display()))?;

    let name = repo
        .workdir()
        .and_then(|p| p.file_name())
        .and_then(|os| os.to_str())
        .map(|s| s.to_string())
        .or_else(|| {
            path.file_name()
                .and_then(|os| os.to_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| path.display().to_string());

    let (head_branch, head_commit) = match repo.head() {
        Ok(head) => {
            let branch = head.shorthand().map(|s| s.to_string());
            let commit = head.peel_to_commit().ok().map(|commit| CommitInfo {
                id: commit.id().to_string(),
                summary: commit.summary().map(|s| s.trim().to_string()),
                time: commit.time().seconds(),
            });
            (branch, commit)
        }
        Err(_) => (None, None),
    };

    let default_remote = primary_remote_url(&repo);
    let status = summarize_statuses(&repo);

    Ok(RepoEntry {
        name,
        path: path.to_path_buf(),
        head_branch,
        head_commit,
        default_remote,
        status,
    })
}

fn primary_remote_url(repo: &Repository) -> Option<String> {
    if let Ok(remote) = repo.find_remote("origin") {
        if let Some(url) = remote.url() {
            return Some(url.to_string());
        }
    }

    if let Ok(remotes) = repo.remotes() {
        for name in remotes.iter().flatten() {
            if let Ok(remote) = repo.find_remote(name) {
                if let Some(url) = remote.url() {
                    return Some(url.to_string());
                }
            }
        }
    }

    None
}

fn summarize_statuses(repo: &Repository) -> RepoStatusCounts {
    let mut counts = RepoStatusCounts::default();
    if let Ok(statuses) = repo.statuses(None) {
        for entry in statuses.iter() {
            let status = entry.status();
            if status.is_empty() {
                continue;
            }

            if status.intersects(
                Status::INDEX_NEW
                    | Status::INDEX_MODIFIED
                    | Status::INDEX_DELETED
                    | Status::INDEX_RENAMED
                    | Status::INDEX_TYPECHANGE,
            ) {
                counts.staged += 1;
            }

            if status.intersects(
                Status::WT_NEW
                    | Status::WT_MODIFIED
                    | Status::WT_DELETED
                    | Status::WT_RENAMED
                    | Status::WT_TYPECHANGE,
            ) {
                counts.unstaged += 1;
            }

            if status.contains(Status::WT_NEW) {
                counts.untracked += 1;
            }

            if status.contains(Status::CONFLICTED) {
                counts.conflicted += 1;
            }
        }
    }
    counts
}

fn write_index(index: &RepoIndex, path: &Path) -> Result<()> {
    let mut file =
        File::create(path).with_context(|| format!("Unable to create {}", path.display()))?;
    serde_json::to_writer_pretty(&mut file, index)
        .with_context(|| format!("Unable to write {}", path.display()))?;
    file.write_all(b"\n")
        .with_context(|| format!("Unable to finalize {}", path.display()))?;
    Ok(())
}

fn unix_timestamp() -> Result<u64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("System clock is before UNIX_EPOCH")?;
    Ok(duration.as_secs())
}

#[derive(Serialize)]
struct RepoIndex {
    root: PathBuf,
    generated_at: u64,
    total_ms: u64,
    discovery_ms: u64,
    metadata_ms: u64,
    repositories: Vec<RepoEntry>,
    failures: Vec<RepoFailure>,
}

#[derive(Serialize)]
struct RepoEntry {
    name: String,
    path: PathBuf,
    head_branch: Option<String>,
    head_commit: Option<CommitInfo>,
    default_remote: Option<String>,
    status: RepoStatusCounts,
}

#[derive(Serialize)]
struct CommitInfo {
    id: String,
    summary: Option<String>,
    time: i64,
}

#[derive(Default, Serialize)]
struct RepoStatusCounts {
    staged: usize,
    unstaged: usize,
    untracked: usize,
    conflicted: usize,
}

#[derive(Serialize)]
struct RepoFailure {
    path: PathBuf,
    reason: String,
}
