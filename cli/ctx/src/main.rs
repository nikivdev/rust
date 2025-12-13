use std::path::Path;
use std::process::{Command, Stdio};
use std::fs;
use std::io::Write;

use anyhow::{Context, Result};
use clap::Parser;
use ignore::WalkBuilder;

fn main() {
    if let Err(err) = try_main() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

fn try_main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Gather { path, task, max_size, output, optimized }) => gather_context(&path, &task, max_size, output.as_deref(), optimized),
        Some(Commands::Pack { path, output, max_size }) => pack_context(&path, output.as_deref(), max_size, false),
        None => {
            // Default: ctx <path> packs and copies to clipboard
            let path = cli.path.as_deref().unwrap_or(".");
            pack_context(path, None, cli.max_size, true)
        }
    }
}

#[derive(Parser)]
#[command(name = "ctx", version, about = "Turn folders into efficient AI context", propagate_version = true)]
struct Cli {
    /// Path to folder to pack (default: current directory).
    path: Option<String>,

    /// Maximum total size in bytes (default: 500KB).
    #[arg(long, default_value = "500000")]
    max_size: usize,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Pack a folder into a single context file (output to file instead of clipboard).
    ///
    /// Examples:
    ///   ctx pack ./src -o context.txt
    Pack {
        /// Path to folder to pack.
        path: String,

        /// Output file path.
        #[arg(short, long)]
        output: Option<String>,

        /// Maximum total size in bytes (default: 500KB).
        #[arg(long, default_value = "500000")]
        max_size: usize,
    },

    /// Use Claude to gather relevant context for a task.
    ///
    /// Analyzes the folder structure and uses Claude Code SDK to
    /// determine which files are relevant for the given task.
    /// Result is copied to clipboard.
    ///
    /// Examples:
    ///   ctx gather . "add user authentication"
    ///   ctx gather ./src "fix the login bug"
    Gather {
        /// Path to folder to analyze.
        path: String,

        /// Task description.
        task: String,

        /// Maximum context size in bytes (default: 200KB for ChatGPT).
        #[arg(long, default_value = "200000")]
        max_size: usize,

        /// Output file path (default: clipboard). Supports {date} and {time} placeholders.
        #[arg(short, long)]
        output: Option<String>,

        /// Optimized mode: fewer files, no tree in output, skip config/build files.
        #[arg(long)]
        optimized: bool,
    },
}

fn pack_context(path: &str, output: Option<&str>, max_size: usize, to_clipboard: bool) -> Result<()> {
    let root = expand_tilde(path);
    let root_path = fs::canonicalize(Path::new(&root)).context("failed to resolve path")?;

    if !root_path.exists() {
        anyhow::bail!("path '{}' does not exist", path);
    }

    let mut context = String::new();
    let mut total_size: usize = 0;
    let mut file_count = 0;
    let mut skipped_count = 0;

    // Header with root path
    context.push_str("<file_map>\n");
    context.push_str(&root_path.display().to_string());
    context.push_str("\n</file_map>\n");
    context.push_str("<file_contents>\n");

    // Walk directory respecting .gitignore, skip hidden files
    let walker = WalkBuilder::new(&root_path)
        .hidden(true)  // Skip hidden files/dirs like .git
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    for entry in walker.flatten() {
        let entry_path = entry.path();

        if !entry_path.is_file() {
            continue;
        }

        // Skip binary and large files
        if is_binary_file(entry_path) {
            continue;
        }

        // Read file content
        let content = match fs::read_to_string(entry_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let lang = get_language_hint(entry_path);
        let file_section = format!(
            "File: {}\n```{}\n{}\n```\n\n",
            entry_path.display(),
            lang,
            content
        );

        // Check size limit
        if total_size + file_section.len() > max_size {
            skipped_count += 1;
            continue;  // Skip this file but continue with others
        }

        total_size += file_section.len();
        context.push_str(&file_section);
        file_count += 1;
    }

    context.push_str("</file_contents>\n");

    // Output
    if to_clipboard {
        copy_to_clipboard(&context)?;
        if skipped_count > 0 {
            let skipped_word = if skipped_count == 1 { "file" } else { "files" };
            eprintln!("copied {} files ({} bytes) to clipboard, skipped {} large {}", file_count, context.len(), skipped_count, skipped_word);
        } else {
            eprintln!("copied {} files ({} bytes) to clipboard", file_count, context.len());
        }
    } else if let Some(out_path) = output {
        let expanded = expand_tilde(out_path);
        fs::write(&expanded, &context).context("failed to write output file")?;
        eprintln!("wrote {} files ({} bytes) to {}", file_count, context.len(), expanded);
    } else {
        print!("{}", context);
    }

    Ok(())
}

fn copy_to_clipboard(content: &str) -> Result<()> {
    let mut child = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .spawn()
        .context("failed to run pbcopy")?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(content.as_bytes()).context("failed to write to pbcopy")?;
    }

    let status = child.wait().context("failed to wait for pbcopy")?;
    if !status.success() {
        anyhow::bail!("pbcopy failed");
    }

    Ok(())
}

fn gather_context(path: &str, task: &str, max_size: usize, output_path: Option<&str>, optimized: bool) -> Result<()> {
    let root = expand_tilde(path);
    let root_path = fs::canonicalize(Path::new(&root)).context("failed to resolve path")?;

    if !root_path.exists() {
        anyhow::bail!("path '{}' does not exist", path);
    }

    eprintln!("building file tree...");

    // Build file tree
    let tree = build_file_tree(&root_path)?;

    eprintln!("asking claude to select relevant files...");

    // Create prompt for Claude - optimized mode is more selective
    let prompt = if optimized {
        format!(
            r#"Select the MINIMAL set of files needed for this task.

## Task
{}

## Files
```
{}
```

## Rules
- Select 3-8 files MAX, only the most essential
- Skip: readme, docs, config files (toml/yaml/json), build scripts, lock files
- Only include files with actual implementation code relevant to the task
- If task mentions a specific component/module, focus ONLY on that

Output ONLY a JSON array: ["path/file.rs"]"#,
            task, tree
        )
    } else {
        format!(
            r#"You are analyzing a codebase to determine which files are relevant for debugging a specific issue.

## Problem
{}

## File Structure
```
{}
```

## Instructions
Analyze the file structure and determine which files are MOST relevant to understanding and fixing this issue.

Consider:
- Files that directly implement the feature mentioned
- Configuration files that affect the feature
- Type definitions and interfaces
- Store/state management related to the feature
- Any error handling or initialization code

Output ONLY a JSON array of relative file paths (from the project root). Example:
["src/feature/component.tsx", "src/stores/feature-store.ts"]

Be thorough but selective - include files that would help debug this specific issue. Aim for 15-30 of the most relevant files."#,
            task, tree
        )
    };

    // Call claude CLI with print mode
    let output = Command::new("claude")
        .args(["-p", &prompt, "--output-format", "text"])
        .output()
        .context("failed to run claude CLI")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("claude CLI failed: {}", stderr);
    }

    let response = String::from_utf8_lossy(&output.stdout);
    let response = response.trim();

    // Try to extract JSON array from response (claude might add extra text)
    let json_str = extract_json_array(response).unwrap_or(response);

    // Parse the JSON array
    let files: Vec<String> = serde_json::from_str(json_str)
        .context(format!("failed to parse claude response as JSON array: {}", json_str))?;

    if files.is_empty() {
        eprintln!("no relevant files found for task: {}", task);
        return Ok(());
    }

    eprintln!("claude selected {} files, building context...", files.len());

    // Build context in the same format as pack_context
    let mut context = String::new();
    let mut total_size: usize = 0;
    let mut file_count = 0;
    let mut skipped_count = 0;

    // Header with task description
    context.push_str(&format!("# Task: {}\n\n", task));

    // Add file tree (skip in optimized mode - redundant since we have the files)
    if !optimized {
        context.push_str("<file_tree>\n");
        context.push_str(&tree);
        context.push_str("</file_tree>\n\n");
    }

    context.push_str("<file_contents>\n");

    // Add each file
    for file_path in &files {
        let full_path = root_path.join(file_path);

        if !full_path.exists() {
            eprintln!("warning: file not found: {}", file_path);
            continue;
        }

        let content = match fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("warning: could not read {}: {}", file_path, e);
                continue;
            }
        };

        let lang = get_language_hint(&full_path);
        let file_section = format!(
            "File: {}\n```{}\n{}\n```\n\n",
            file_path,
            lang,
            content
        );

        // Check size limit
        if total_size + file_section.len() > max_size {
            skipped_count += 1;
            continue;
        }

        total_size += file_section.len();
        context.push_str(&file_section);
        file_count += 1;
    }

    context.push_str("</file_contents>\n");

    // Always save to ~/done with timestamp
    let done_path = expand_output_path("~/done/{datetime}.md");
    if let Some(parent) = Path::new(&done_path).parent() {
        fs::create_dir_all(parent).context("failed to create ~/done directory")?;
    }
    fs::write(&done_path, &context).context("failed to write to ~/done")?;

    // Output to file or clipboard
    if let Some(out_path) = output_path {
        let expanded = expand_output_path(out_path);

        // Create parent directories if needed
        if let Some(parent) = Path::new(&expanded).parent() {
            fs::create_dir_all(parent).context("failed to create output directory")?;
        }

        fs::write(&expanded, &context).context("failed to write output file")?;

        if skipped_count > 0 {
            let skipped_word = if skipped_count == 1 { "file" } else { "files" };
            eprintln!("wrote {} files ({} bytes) to {}, skipped {} large {}", file_count, context.len(), expanded, skipped_count, skipped_word);
        } else {
            eprintln!("wrote {} files ({} bytes) to {}", file_count, context.len(), expanded);
        }
    } else {
        copy_to_clipboard(&context)?;

        if skipped_count > 0 {
            let skipped_word = if skipped_count == 1 { "file" } else { "files" };
            eprintln!("copied {} files ({} bytes) to clipboard, skipped {} large {}", file_count, context.len(), skipped_count, skipped_word);
        } else {
            eprintln!("copied {} files ({} bytes) to clipboard", file_count, context.len());
        }
    }

    // Print the saved file path
    eprintln!("{}", done_path);

    Ok(())
}

fn expand_output_path(path: &str) -> String {
    use chrono::Local;

    let now = Local::now();
    let expanded = expand_tilde(path);

    expanded
        .replace("{date}", &now.format("%Y-%m-%d").to_string())
        .replace("{time}", &now.format("%H-%M-%S").to_string())
        .replace("{datetime}", &now.format("%Y-%m-%d-%H-%M-%S").to_string())
}

fn extract_json_array(text: &str) -> Option<&str> {
    // Find the first [ and last ]
    let start = text.find('[')?;
    let end = text.rfind(']')?;
    if start < end {
        Some(&text[start..=end])
    } else {
        None
    }
}

fn build_file_tree(root: &Path) -> Result<String> {
    let mut tree = String::new();
    build_tree_recursive(root, root, "", &mut tree)?;
    Ok(tree)
}

fn build_tree_recursive(root: &Path, current: &Path, prefix: &str, output: &mut String) -> Result<()> {
    let mut entries: Vec<_> = WalkBuilder::new(current)
        .max_depth(Some(1))
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build()
        .flatten()
        .filter(|e| e.path() != current)
        .collect();

    entries.sort_by(|a, b| {
        let a_is_dir = a.path().is_dir();
        let b_is_dir = b.path().is_dir();
        match (a_is_dir, b_is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.path().cmp(b.path()),
        }
    });

    let count = entries.len();
    for (i, entry) in entries.into_iter().enumerate() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let is_last = i == count - 1;
        let connector = if is_last { "└── " } else { "├── " };

        if path.is_dir() {
            output.push_str(&format!("{}{}{}/\n", prefix, connector, name));
            let new_prefix = format!("{}{}   ", prefix, if is_last { " " } else { "│" });
            build_tree_recursive(root, path, &new_prefix, output)?;
        } else {
            output.push_str(&format!("{}{}{}\n", prefix, connector, name));
        }
    }

    Ok(())
}

fn is_binary_file(path: &Path) -> bool {
    let binary_extensions = [
        "png", "jpg", "jpeg", "gif", "bmp", "ico", "webp", "svg",
        "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx",
        "zip", "tar", "gz", "rar", "7z",
        "exe", "dll", "so", "dylib", "a",
        "wasm", "pyc", "class",
        "mp3", "mp4", "wav", "avi", "mov", "mkv",
        "ttf", "otf", "woff", "woff2", "eot",
        "db", "sqlite", "sqlite3",
        "lock", // often large and not useful for context
    ];

    if let Some(ext) = path.extension() {
        let ext_lower = ext.to_string_lossy().to_lowercase();
        if binary_extensions.contains(&ext_lower.as_str()) {
            return true;
        }
    }

    // Check for files without extension that are likely binary
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    if name.starts_with('.') && !name.contains('.', ) {
        // Hidden files without extension might be config, check content
    }

    // Quick check: read first few bytes for null bytes
    if let Ok(bytes) = fs::read(path) {
        let check_len = bytes.len().min(8192);
        return bytes[..check_len].contains(&0);
    }

    false
}

fn get_language_hint(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("py") => "python",
        Some("js") => "javascript",
        Some("ts") => "typescript",
        Some("tsx") => "tsx",
        Some("jsx") => "jsx",
        Some("go") => "go",
        Some("rb") => "ruby",
        Some("java") => "java",
        Some("kt") => "kotlin",
        Some("swift") => "swift",
        Some("c") => "c",
        Some("cpp") | Some("cc") | Some("cxx") => "cpp",
        Some("h") | Some("hpp") => "cpp",
        Some("cs") => "csharp",
        Some("php") => "php",
        Some("sh") | Some("bash") => "bash",
        Some("zsh") => "zsh",
        Some("fish") => "fish",
        Some("sql") => "sql",
        Some("html") => "html",
        Some("css") => "css",
        Some("scss") | Some("sass") => "scss",
        Some("json") => "json",
        Some("yaml") | Some("yml") => "yaml",
        Some("toml") => "toml",
        Some("xml") => "xml",
        Some("md") => "markdown",
        Some("dockerfile") => "dockerfile",
        _ => {
            // Check filename
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            match name.as_ref() {
                "Dockerfile" => "dockerfile",
                "Makefile" => "makefile",
                "CMakeLists.txt" => "cmake",
                _ => "",
            }
        }
    }
}

fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}{}", home, &path[1..]);
        }
    }
    path.to_string()
}
