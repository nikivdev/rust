use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};

fn main() {
    if let Err(err) = try_main() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

fn try_main() -> Result<()> {
    let cli = Cli::parse();

    if cli.doc_update {
        return generate_docs(cli.path.as_deref());
    }

    let index_path = cli.index_path.unwrap_or_else(default_index_path);
    let index = if cli.refresh || !index_path.exists() {
        build_index(&index_path, cli.path.as_deref())?
    } else {
        load_index(&index_path).or_else(|_| build_index(&index_path, cli.path.as_deref()))?
    };

    let query = match &cli.query {
        Some(q) => q,
        None if cli.refresh => {
            println!("Index refreshed: {} entries", index.entries.len());
            return Ok(());
        }
        None => anyhow::bail!("query is required unless --refresh is used"),
    };

    let matches = search_index(&index, query, cli.limit);
    if matches.is_empty() {
        println!("No matches found.");
        return Ok(());
    }

    if cli.run {
        run_match(&matches[0])?;
        return Ok(());
    }

    for (i, entry) in matches.iter().enumerate() {
        let hint = entry.help_hint.as_deref().unwrap_or("");
        if hint.is_empty() {
            println!("{}. {}  ({})", i + 1, entry.name, entry.path.display());
        } else {
            println!(
                "{}. {}  ({})\n    {}",
                i + 1,
                entry.name,
                entry.path.display(),
                hint
            );
        }
    }

    Ok(())
}

#[derive(Parser)]
#[command(name = "unite", version, about = "Find commands by intent across your PATH")]
struct Cli {
    /// Query text, e.g. "list zed windows"
    query: Option<String>,
    /// Path to index file
    #[arg(long)]
    index_path: Option<PathBuf>,
    /// Limit number of results
    #[arg(long, default_value_t = 5)]
    limit: usize,
    /// Rebuild index
    #[arg(long)]
    refresh: bool,
    /// Run the top match
    #[arg(long)]
    run: bool,
    /// Override PATH used for scanning
    #[arg(long)]
    path: Option<String>,
    /// Generate markdown docs for all commands in PATH
    #[arg(long)]
    doc_update: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct Index {
    version: u32,
    created_at_epoch: u64,
    entries: Vec<Entry>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Entry {
    name: String,
    path: PathBuf,
    help_hint: Option<String>,
    search_text: String,
}

fn build_index(index_path: &Path, path_override: Option<&str>) -> Result<Index> {
    let commands = scan_path(path_override)?;
    let total = commands.len();
    let mut entries = Vec::new();

    eprint!("Indexing {} commands...", total);
    for (i, (name, path)) in commands.iter().enumerate() {
        if (i + 1) % 50 == 0 {
            eprint!("\rIndexing {} commands... {}/{}", total, i + 1, total);
        }
        let help = fetch_help(&path).unwrap_or_default();
        let help_hint = extract_help_hint(&help);
        let name_lc = name.to_lowercase();
        let mut search_text = format!("{} {}", name_lc, help.to_lowercase());

        let subcommands = extract_subcommands(&help);
        for (sub, desc) in &subcommands {
            search_text.push_str(&format!(" {} {}", sub.to_lowercase(), desc.to_lowercase()));
        }

        entries.push(Entry {
            name: name.clone(),
            path: path.clone(),
            help_hint,
            search_text,
        });

        for (sub, desc) in subcommands {
            let sub_name = format!("{} {}", name, sub);
            let sub_search = format!("{} {}", sub_name.to_lowercase(), desc.to_lowercase());
            entries.push(Entry {
                name: sub_name,
                path: path.clone(),
                help_hint: if desc.is_empty() { None } else { Some(desc) },
                search_text: sub_search,
            });
        }
    }
    eprintln!("\rIndexed {} commands, {} entries total.", total, entries.len());

    let index = Index {
        version: 1,
        created_at_epoch: now_epoch_sec(),
        entries,
    };

    if let Some(parent) = index_path.parent() {
        fs::create_dir_all(parent).context("failed to create index directory")?;
    }
    let json = serde_json::to_string(&index).context("failed to serialize index")?;
    fs::write(index_path, json).context("failed to write index")?;

    Ok(index)
}

fn load_index(path: &Path) -> Result<Index> {
    let data = fs::read_to_string(path).context("failed to read index")?;
    let index: Index = serde_json::from_str(&data).context("failed to parse index")?;
    Ok(index)
}

fn scan_path(path_override: Option<&str>) -> Result<BTreeMap<String, PathBuf>> {
    let mut map = BTreeMap::new();
    let path_var = path_override
        .map(|s| s.to_string())
        .or_else(|| std::env::var("PATH").ok())
        .unwrap_or_default();

    for dir in path_var.split(':') {
        if dir.is_empty() {
            continue;
        }
        let dir_path = Path::new(dir);
        if let Ok(entries) = fs::read_dir(dir_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !is_executable(&path) {
                    continue;
                }
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    map.entry(name.to_string()).or_insert(path);
                }
            }
        }
    }

    Ok(map)
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = fs::metadata(path) {
        if !meta.is_file() {
            return false;
        }
        return meta.permissions().mode() & 0o111 != 0;
    }
    false
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn fetch_help(path: &Path) -> Result<String> {
    let mut child = Command::new(path)
        .arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to run {}", path.display()))?;

    let start = Instant::now();
    let timeout = Duration::from_millis(800);
    loop {
        if let Some(status) = child.try_wait()? {
            let output = child.wait_with_output().context("failed to read help output")?;
            let mut text = String::new();
            text.push_str(&String::from_utf8_lossy(&output.stdout));
            if status.success() {
                return Ok(text);
            }
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            return Ok(text);
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            return Ok(String::new());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn extract_help_hint(help: &str) -> Option<String> {
    for line in help.lines().take(6) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.to_lowercase().starts_with("usage:") {
            continue;
        }
        if line.chars().count() > 100 {
            return Some(line.chars().take(100).collect());
        }
        return Some(line.to_string());
    }
    None
}

fn extract_subcommands(help: &str) -> Vec<(String, String)> {
    let mut subs = Vec::new();
    let mut in_section = false;

    for line in help.lines() {
        let raw = line;
        let line = line.trim_end();
        let lower = line.to_lowercase();
        if lower == "commands:" || lower == "subcommands:" {
            in_section = true;
            continue;
        }
        if in_section {
            if line.trim().is_empty() {
                break;
            }
            if raw.starts_with(' ') || raw.starts_with('\t') {
                let trimmed = line.trim();
                let mut parts = trimmed.split_whitespace();
                if let Some(cmd) = parts.next() {
                    let desc = parts.collect::<Vec<_>>().join(" ");
                    subs.push((cmd.to_string(), desc));
                }
            } else {
                break;
            }
        }
    }

    subs
}

fn search_index(index: &Index, query: &str, limit: usize) -> Vec<Entry> {
    let query_lc = query.to_lowercase();
    let tokens = tokenize(query);
    let tokens = remove_stopwords(tokens);

    let mut scored: Vec<(i32, Entry)> = Vec::new();
    for entry in &index.entries {
        let hay = entry.search_text.as_str();
        let mut score = 0i32;
        if hay.contains(&query_lc) {
            score += 40;
        }
        for token in &tokens {
            if token.is_empty() {
                continue;
            }
            if entry.name.to_lowercase().contains(token) {
                score += 15;
            }
            if hay.contains(token) {
                score += 5;
            }
        }
        if score > 0 {
            scored.push((score, entry.clone()));
        }
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().take(limit).map(|(_, e)| e).collect()
}

fn run_match(entry: &Entry) -> Result<()> {
    let mut parts = entry.name.split_whitespace();
    if parts.next().is_none() {
        anyhow::bail!("invalid entry");
    }
    let args: Vec<&str> = parts.collect();

    let status = Command::new(&entry.path)
        .args(args)
        .status()
        .with_context(|| format!("failed to run {}", entry.name))?;
    if !status.success() {
        anyhow::bail!("command failed with status {}", status);
    }
    Ok(())
}

fn tokenize(input: &str) -> Vec<String> {
    input
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

fn remove_stopwords(tokens: Vec<String>) -> Vec<String> {
    let stop = stopwords();
    tokens
        .into_iter()
        .filter(|t| !stop.contains(t))
        .collect()
}

fn stopwords() -> HashSet<String> {
    [
        "how", "to", "list", "show", "get", "all", "the", "a", "an", "of", "and", "or", "is",
        "are", "for", "in", "on", "with", "me",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn default_index_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join("Library")
        .join("Caches")
        .join("unite")
        .join("index.json")
}

fn now_epoch_sec() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn generate_docs(path_override: Option<&str>) -> Result<()> {
    let commands = scan_path(path_override)?;
    let total = commands.len();

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let doc_path = PathBuf::from(&home).join(".db").join("unite").join("commands.md");

    if let Some(parent) = doc_path.parent() {
        fs::create_dir_all(parent).context("failed to create doc directory")?;
    }

    let mut output = String::new();
    output.push_str("# Commands\n\n");
    output.push_str(&format!("Generated: {}\n\n", chrono_date()));
    output.push_str(&format!("Total commands: {}\n\n", total));
    output.push_str("---\n\n");

    eprint!("Generating docs for {} commands...", total);
    for (i, (name, path)) in commands.iter().enumerate() {
        if (i + 1) % 50 == 0 {
            eprint!("\rGenerating docs for {} commands... {}/{}", total, i + 1, total);
        }

        let help = fetch_help(path).unwrap_or_default();

        output.push_str(&format!("## {}\n\n", name));
        output.push_str(&format!("**Path:** `{}`\n\n", path.display()));

        if help.is_empty() {
            output.push_str("*No help output available*\n\n");
        } else {
            output.push_str("```\n");
            // Limit help to first 100 lines to keep file manageable
            let help_lines: Vec<&str> = help.lines().take(100).collect();
            output.push_str(&help_lines.join("\n"));
            if help.lines().count() > 100 {
                output.push_str("\n... (truncated)");
            }
            output.push_str("\n```\n\n");
        }

        output.push_str("---\n\n");
    }
    eprintln!("\rGenerated docs for {} commands.", total);

    fs::write(&doc_path, &output).context("failed to write docs")?;
    println!("Wrote {} to {}", byte_size(output.len()), doc_path.display());

    Ok(())
}

fn chrono_date() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let secs_per_day = 86400;
    let days_since_epoch = now / secs_per_day;
    let (year, month, day) = days_to_ymd(days_since_epoch);
    format!("{:04}-{:02}-{:02}", year, month, day)
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

fn byte_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
