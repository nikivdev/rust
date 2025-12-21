use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use claude_code_sdk::{query, AssistantMessage, ClaudeCodeOptions, ContentBlock, Message, TextBlock};
use serde::Serialize;
use tokio_stream::StreamExt;
use walkdir::WalkDir;

#[tokio::main]
async fn main() {
    if let Err(err) = try_main().await {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

async fn try_main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Suggest(args) => run_suggest(args).await,
    }
}

#[derive(Parser)]
#[command(name = "move", version, about = "Cleanup + archive helper", propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Suggest files and folders to delete or archive to free space.
    Suggest(SuggestArgs),
}

#[derive(Args)]
struct SuggestArgs {
    /// Root directory to scan (defaults to HOME).
    #[arg(long, value_name = "DIR")]
    root: Option<PathBuf>,

    /// Minimum file size to include (e.g. 200MB, 1.5GB).
    #[arg(long, default_value = "200MB", value_name = "SIZE")]
    min_size: String,

    /// Maximum directory depth to scan.
    #[arg(long, value_name = "LEVELS")]
    max_depth: Option<usize>,

    /// Maximum number of files to scan.
    #[arg(long, value_name = "COUNT")]
    max_files: Option<usize>,

    /// How many top files to include.
    #[arg(long, default_value_t = 50, value_name = "COUNT")]
    top_files: usize,

    /// How many top folders to include.
    #[arg(long, default_value_t = 30, value_name = "COUNT")]
    top_folders: usize,

    /// Bucket depth for folder aggregation (1 = immediate child of root).
    #[arg(long, default_value_t = 2, value_name = "LEVELS")]
    bucket_depth: usize,

    /// Skip any path containing these substrings (repeatable).
    #[arg(long, value_name = "TEXT")]
    exclude: Vec<String>,

    /// Include system paths when root is "/".
    #[arg(long)]
    include_system: bool,

    /// Skip calling Claude and only print the local scan.
    #[arg(long)]
    no_claude: bool,

    /// Claude model override.
    #[arg(long, value_name = "MODEL")]
    model: Option<String>,

    /// Custom system prompt for Claude.
    #[arg(long, value_name = "TEXT")]
    system: Option<String>,
}

#[derive(Serialize)]
struct ScanReport {
    root: PathBuf,
    min_size_bytes: u64,
    scanned_files: u64,
    scanned_dirs: u64,
    errors: u64,
    top_files: Vec<FileEntry>,
    top_folders: Vec<FolderEntry>,
}

#[derive(Serialize)]
struct FileEntry {
    path: PathBuf,
    size_bytes: u64,
    modified_secs: Option<u64>,
}

#[derive(Serialize)]
struct FolderEntry {
    path: PathBuf,
    size_bytes: u64,
}

async fn run_suggest(args: SuggestArgs) -> Result<()> {
    let root = args
        .root
        .unwrap_or_else(|| default_root().unwrap_or_else(|| PathBuf::from(".")));

    if !root.exists() {
        bail!("{} does not exist", root.display());
    }

    let root = root
        .canonicalize()
        .with_context(|| format!("Unable to resolve {}", root.display()))?;

    if !root.is_dir() {
        bail!("{} is not a directory", root.display());
    }

    let min_size_bytes = parse_size(&args.min_size)?;

    let mut excludes = args.exclude.clone();
    if should_exclude_system(&root, args.include_system) {
        excludes.extend(system_excludes());
    }

    let report = scan_root(
        &root,
        min_size_bytes,
        args.max_depth,
        args.max_files,
        args.top_files,
        args.top_folders,
        args.bucket_depth,
        &excludes,
    )?;

    print_local_report(&report);

    if args.no_claude {
        return Ok(());
    }

    let prompt = build_claude_prompt(&report)?;
    let options = ClaudeCodeOptions {
        model: args.model.clone(),
        system_prompt: args.system.clone().or_else(|| Some(default_system_prompt())),
        ..Default::default()
    };

    println!();
    println!("Claude suggestions:");
    println!("------------------");

    run_claude(prompt, options).await?;

    Ok(())
}

fn default_root() -> Option<PathBuf> {
    env::var("HOME").ok().map(PathBuf::from)
}

fn scan_root(
    root: &Path,
    min_size_bytes: u64,
    max_depth: Option<usize>,
    max_files: Option<usize>,
    top_files: usize,
    top_folders: usize,
    bucket_depth: usize,
    exclude: &[String],
) -> Result<ScanReport> {
    let mut builder = WalkDir::new(root).follow_links(false);
    if let Some(depth) = max_depth {
        if depth == 0 {
            bail!("max-depth must be greater than zero");
        }
        builder = builder.max_depth(depth);
    }

    let mut files = Vec::new();
    let mut folder_sizes: HashMap<PathBuf, u64> = HashMap::new();
    let mut scanned_files = 0u64;
    let mut scanned_dirs = 0u64;
    let mut errors = 0u64;

    let mut iter = builder.into_iter();

    while let Some(entry) = iter.next() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                errors += 1;
                eprintln!("Skipping entry: {err}");
                continue;
            }
        };

        let path = entry.path();
        if is_excluded(path, exclude) {
            if entry.file_type().is_dir() {
                iter.skip_current_dir();
            }
            continue;
        }

        if entry.file_type().is_dir() {
            scanned_dirs += 1;
            continue;
        }

        if !entry.file_type().is_file() {
            continue;
        }

        scanned_files += 1;
        if let Some(limit) = max_files {
            if scanned_files > limit {
                break;
            }
        }

        let metadata = match entry.metadata() {
            Ok(meta) => meta,
            Err(err) => {
                errors += 1;
                eprintln!("Skipping metadata for {}: {err}", path.display());
                continue;
            }
        };

        let size = metadata.len();
        let bucket = bucket_path(root, path, bucket_depth);
        *folder_sizes.entry(bucket).or_insert(0) += size;

        if size < min_size_bytes {
            continue;
        }

        let modified_secs = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());

        files.push(FileEntry {
            path: path.to_path_buf(),
            size_bytes: size,
            modified_secs,
        });
    }

    files.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    if files.len() > top_files {
        files.truncate(top_files);
    }

    let mut folders: Vec<FolderEntry> = folder_sizes
        .into_iter()
        .filter(|(_, size)| *size >= min_size_bytes)
        .map(|(path, size_bytes)| FolderEntry { path, size_bytes })
        .collect();

    folders.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    if folders.len() > top_folders {
        folders.truncate(top_folders);
    }

    Ok(ScanReport {
        root: root.to_path_buf(),
        min_size_bytes,
        scanned_files,
        scanned_dirs,
        errors,
        top_files: files,
        top_folders: folders,
    })
}

fn bucket_path(root: &Path, path: &Path, depth: usize) -> PathBuf {
    if depth == 0 {
        return root.to_path_buf();
    }

    let root_count = root.components().count();
    let base = path.parent().unwrap_or(path);
    let mut buf = PathBuf::new();
    for (idx, component) in base.components().enumerate() {
        buf.push(component.as_os_str());
        if idx + 1 >= root_count + depth {
            break;
        }
    }

    if buf.as_os_str().is_empty() {
        root.to_path_buf()
    } else {
        buf
    }
}

fn is_excluded(path: &Path, excludes: &[String]) -> bool {
    if excludes.is_empty() {
        return false;
    }

    let path_str = path.to_string_lossy();
    excludes.iter().any(|pattern| path_str.contains(pattern))
}

fn print_local_report(report: &ScanReport) {
    println!("Scan root: {}", report.root.display());
    println!("Min size: {}", format_size(report.min_size_bytes));
    println!(
        "Scanned {} files, {} dirs, {} errors",
        report.scanned_files, report.scanned_dirs, report.errors
    );

    println!();
    println!("Largest files:");
    if report.top_files.is_empty() {
        println!("  (none above threshold)");
    } else {
        for entry in &report.top_files {
            let age = format_age(entry.modified_secs);
            println!(
                "  {:>10}  {:>8}  {}",
                format_size(entry.size_bytes),
                age,
                entry.path.display()
            );
        }
    }

    println!();
    println!("Largest folders (approx):");
    if report.top_folders.is_empty() {
        println!("  (none above threshold)");
    } else {
        for entry in &report.top_folders {
            println!("  {:>10}  {}", format_size(entry.size_bytes), entry.path.display());
        }
    }
}

fn format_age(modified_secs: Option<u64>) -> String {
    let Some(modified) = modified_secs else {
        return "unknown".to_string();
    };

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs();

    if modified > now {
        return "0d".to_string();
    }

    let age_secs = now - modified;
    let days = age_secs / 86_400;
    if days > 0 {
        return format!("{}d", days);
    }

    let hours = age_secs / 3_600;
    if hours > 0 {
        return format!("{}h", hours);
    }

    let minutes = age_secs / 60;
    format!("{}m", minutes)
}

fn format_size(bytes: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut size = bytes as f64;
    let mut idx = 0usize;
    while size >= 1024.0 && idx < units.len() - 1 {
        size /= 1024.0;
        idx += 1;
    }

    if idx == 0 {
        format!("{} {}", bytes, units[idx])
    } else {
        format!("{:.1} {}", size, units[idx])
    }
}

fn parse_size(input: &str) -> Result<u64> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("size cannot be empty");
    }

    let mut num = String::new();
    let mut unit = String::new();
    for ch in trimmed.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            num.push(ch);
        } else if !ch.is_whitespace() {
            unit.push(ch);
        }
    }

    if num.is_empty() {
        bail!("invalid size: {input}");
    }

    let value: f64 = num
        .parse()
        .with_context(|| format!("invalid size: {input}"))?;
    if value.is_sign_negative() {
        bail!("size must be positive");
    }

    let unit = unit.to_ascii_lowercase();
    let multiplier = match unit.as_str() {
        "" | "b" => 1.0,
        "k" | "kb" | "kib" => 1024.0,
        "m" | "mb" | "mib" => 1024.0 * 1024.0,
        "g" | "gb" | "gib" => 1024.0 * 1024.0 * 1024.0,
        "t" | "tb" | "tib" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => bail!("unknown size unit: {unit}"),
    };

    Ok((value * multiplier) as u64)
}

fn build_claude_prompt(report: &ScanReport) -> Result<String> {
    let payload = serde_json::to_string_pretty(report)?;
    Ok(format!(
        "Here is a macOS disk usage scan summary. Suggest what the user can archive or delete to free space.\n\nRules:\n- Only use paths provided.\n- Avoid suggesting deletes for system-critical paths (/System, /Library, /Applications).\n- Prefer archiving older large files and deleting caches/build artifacts if safe.\n- Provide concise reasons.\n\nOutput format:\nArchive candidates:\n- path | size | reason\nDelete candidates:\n- path | size | reason\nKeep (if any path should be kept despite size):\n- path | reason\n\nScan summary JSON:\n{payload}\n"
    ))
}

fn default_system_prompt() -> String {
    "You help users reclaim disk space safely on macOS. Be conservative about deletions; prefer archiving personal data and deleting caches/build artifacts. Call out Xcode DerivedData/Archives and simulator data if relevant."
        .to_string()
}

fn should_exclude_system(root: &Path, include_system: bool) -> bool {
    if include_system {
        return false;
    }
    root == Path::new("/")
}

fn system_excludes() -> Vec<String> {
    vec![
        "/System/".to_string(),
        "/Library/".to_string(),
        "/Applications/".to_string(),
        "/Volumes/".to_string(),
        "/dev/".to_string(),
        "/private/".to_string(),
        "/cores/".to_string(),
    ]
}

async fn run_claude(prompt: String, options: ClaudeCodeOptions) -> Result<()> {
    let mut stream = query(prompt, Some(options)).await?;

    let mut output = String::new();
    while let Some(message) = stream.next().await {
        match message {
            Message::Assistant(AssistantMessage { content }) => {
                for block in content {
                    if let ContentBlock::Text(TextBlock { text }) = block {
                        output.push_str(&text);
                    }
                }
            }
            Message::Result(result) => {
                if result.is_error {
                    if let Some(err) = result.result {
                        eprintln!("\nClaude error: {err}");
                    }
                }
            }
            _ => {}
        }
    }

    print!("{}", output);
    if !output.ends_with('\n') {
        println!();
    }

    Ok(())
}
